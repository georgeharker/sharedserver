// OpenCode plugin: manage shared backend processes via the `sharedserver` CLI.
// See README.md for installation and configuration.

import { spawnSync } from "node:child_process"
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { dirname, join, resolve as resolvePath } from "node:path"
import { fileURLToPath } from "node:url"
import type { Plugin } from "@opencode-ai/plugin"

type ServerSpec = {
    /** Binary to run (required unless `lazy` is true). */
    command?: string
    /** Arguments passed to `command`. */
    args?: string[]
    /** Extra environment variables forwarded via `--env KEY=VALUE`. */
    env?: Record<string, string>
    /** Grace period for sharedserver, e.g. "30m", "1h", "2h30m". */
    gracePeriod?: string
    /** Capture stdout/stderr of the managed server to this path. */
    logFile?: string
    /** Optional metadata string forwarded to sharedserver. */
    metadata?: string
    /** Only attach if the server is already running; never start it. */
    lazy?: boolean
    /** Name of an env var; when it is set (non-empty) this server is skipped
     *  entirely — neither started nor attached. Use it when another host has
     *  already launched the process for this session. */
    skipIfEnv?: string
}

type Options = {
    /** Explicit path to the `sharedserver` binary. */
    binary?: string
    /** Override SHAREDSERVER_LOCKDIR for child invocations. */
    lockdir?: string
    /** Show TUI toasts for attach success/failure. Defaults to `true`. */
    notify?: boolean
    /** Map of sharedserver name -> server config. Takes precedence over any
     *  config file; use it to keep everything inline in opencode.json. */
    servers?: Record<string, ServerSpec>
    /** Explicit path to a servers.json. Overrides the discovery chain. */
    config?: string
}

type LogFn = (level: "info" | "warn" | "error", message: string) => void
type ToastFn = (variant: "success" | "warning" | "error", message: string) => void

const CANDIDATE_BINARIES = [
    "sharedserver",
    join(homedir(), ".cargo", "bin", "sharedserver"),
    join(homedir(), ".local", "bin", "sharedserver"),
    "/usr/local/bin/sharedserver",
    "/opt/homebrew/bin/sharedserver",
]

/** The floor IS the pin — one number from this package's version, self-maintaining via
 *  lockstep releases. The test is >=, so a newer local install is never undone.
 *  HARDCODED_FLOOR is the backstop for an unreadable manifest: 0.5.0 added the
 *  PID-reuse guard. See plugins/claude/bin/sharedserver. */
const HARDCODED_FLOOR = "0.5.0"

const PLUGIN_VERSION: string | undefined = (() => {
    try {
        const here = dirname(fileURLToPath(import.meta.url))
        const pkg = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8")) as { version?: string }
        return pkg.version
    } catch {
        return undefined
    }
})()

const MIN_SHAREDSERVER_VERSION = PLUGIN_VERSION ?? HARDCODED_FLOOR

function parseVersion(text: string): [number, number, number] | undefined {
    const m = text.match(/(\d+)\.(\d+)\.(\d+)/)
    return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : undefined
}

function gte(a: [number, number, number], b: [number, number, number]): boolean {
    for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i]
    return true
}

function versionOf(candidate: string, env: NodeJS.ProcessEnv): [number, number, number] | undefined {
    const r = spawnSync(candidate, ["--version"], { env })
    if (r.error) return undefined
    return parseVersion(`${r.stdout?.toString() ?? ""}${r.stderr?.toString() ?? ""}`)
}

/** Synchronous sleep — this resolution path runs before anything can be awaited, so the
 *  lock wait cannot use timers. Atomics.wait blocks without spinning. */
function sleepSync(ms: number): void {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)
}

/** First sharedserver found at ANY version, or undefined. Mirrors the shim's ladder. */
function probeAny(override: string | undefined, env: NodeJS.ProcessEnv): string | undefined {
    const candidates = [override, env.SHAREDSERVER_BIN, ...CANDIDATE_BINARIES].filter(
        (v): v is string => typeof v === "string" && v.length > 0,
    )
    for (const candidate of candidates) {
        if (candidate.includes("/")) {
            if (existsSync(candidate)) return candidate
            continue
        }
        // Presence is "spawn did not fail", NOT "exited 0" — the shim draws the same
        // distinction. A build that rejects some flag is still present.
        if (!spawnSync(candidate, ["--version"], { stdio: "ignore", env }).error) return candidate
    }
    return undefined
}

/** Resolve the binary, fetching a pinned release when nothing usable is installed.
 *  Behaviourally identical to plugins/claude/bin/sharedserver — same ladder, floor,
 *  lock and fallback. Change them together. */
function resolveBinary(
    override: string | undefined,
    env: NodeJS.ProcessEnv,
    log?: LogFn,
    toast?: ToastFn,
): string | undefined {
    const floor = parseVersion(MIN_SHAREDSERVER_VERSION)

    // An explicit binary / SHAREDSERVER_BIN is never second-guessed: the user named a
    // specific one, so quietly downloading a different one is the wrong answer to
    // that. Warn if it is old, honour it regardless.
    const explicit = override ?? env.SHAREDSERVER_BIN
    if (explicit) {
        const present = explicit.includes("/")
            ? existsSync(explicit)
            : !spawnSync(explicit, ["--version"], { stdio: "ignore", env }).error
        if (present) {
            const v = versionOf(explicit, env)
            if (v && floor && !gte(v, floor)) {
                const msg =
                    `sharedserver at ${explicit} is ${v.join(".")}; this plugin ships against ` +
                    `${MIN_SHAREDSERVER_VERSION}. Using it anyway because you set it explicitly.`
                log?.("warn", msg)
                toast?.("warning", msg)
            }
            return explicit
        }
    }

    const found = probeAny(override, env)
    if (found) {
        const v = versionOf(found, env)
        if (v && floor && gte(v, floor)) return found
        // Too old: fetch the matching release, but remember this one — if the download
        // fails we would rather run the old binary than nothing.
        const msg =
            `sharedserver at ${found} is ${v?.join(".") ?? "an unknown version"}; this plugin ships against ` +
            `${MIN_SHAREDSERVER_VERSION}. Fetching the matching release so the plugin and binary stay in ` +
            `lockstep. To keep your own build instead, set SHAREDSERVER_BIN=${found}`
        log?.("warn", msg)
        toast?.("warning", msg)
        return installPinned(env, log, toast) ?? found
    }

    log?.(
        "info",
        `sharedserver not found; fetching v${MIN_SHAREDSERVER_VERSION} from GitHub releases (one time). ` +
            "This needs no Rust toolchain. To use your own build instead, set SHAREDSERVER_BIN.",
    )
    return installPinned(env, log, toast)
}

/** Download and run the cargo-dist installer for the pinned version, then re-probe.
 *  Returns the resolved binary, or undefined if anything went wrong. */
function installPinned(env: NodeJS.ProcessEnv, log?: LogFn, toast?: ToastFn): string | undefined {
    // Same lock directory as the shell shim, so Claude and OpenCode coordinate rather
    // than race. mkdir is atomic everywhere.
    const lockdir = join(env.TMPDIR || "/tmp", ".sharedserver-install.lock")
    let haveLock = false
    try {
        mkdirSync(lockdir)
        haveLock = true
    } catch {
        // Someone else is installing. Wait, bounded, then re-probe rather than
        // installing concurrently against the same target path.
        const deadline = Date.now() + 20_000
        while (existsSync(lockdir) && Date.now() < deadline) sleepSync(250)
        const after = probeAny(undefined, env)
        if (after) return after
        try {
            mkdirSync(lockdir)
            haveLock = true
        } catch {
            log?.(
                "warn",
                `another process is installing sharedserver and did not finish; remove the stale lock if this persists: ${lockdir}`,
            )
            return undefined
        }
    }

    try {
        // Re-probe under the lock: the winner may have finished between our failed
        // mkdir and acquiring it. Without this we reinstall what we just waited for.
        const already = probeAny(undefined, env)
        if (already) {
            const v = versionOf(already, env)
            const floor = parseVersion(MIN_SHAREDSERVER_VERSION)
            if (v && floor && gte(v, floor)) return already
        }

        const url = PLUGIN_VERSION
            ? `https://github.com/georgeharker/sharedserver/releases/download/v${PLUGIN_VERSION}/sharedserver-installer.sh`
            : "https://github.com/georgeharker/sharedserver/releases/latest/download/sharedserver-installer.sh"

        // Download to a file, then run it. `curl … | sh` would report success on a 404,
        // since a pipeline's status is sh's; spawnSync uses no shell, so this is exact.
        const script = join(lockdir, "installer.sh")
        const dl = spawnSync("curl", ["--proto", "=https", "--tlsv1.2", "-LsSf", url, "-o", script], { env })
        if (dl.error || dl.status !== 0) {
            const msg =
                `could not download the sharedserver installer from ${url} — a release for ` +
                `v${PLUGIN_VERSION ?? "?"} may not exist yet, or the network is unavailable`
            log?.("warn", msg)
            toast?.("warning", msg)
            return undefined
        }
        if (!existsSync(script) || statSync(script).size === 0) {
            log?.("warn", `the downloaded sharedserver installer was empty (from ${url})`)
            return undefined
        }

        // Make sure the installer can actually verify what it downloads. cargo-dist
        // embeds the expected sha256 but looks up only `sha256sum` and returns SUCCESS
        // when it is missing — so on macOS (which ships `shasum -a 256`) verification is
        // silently skipped. Put a sha256sum on PATH rather than patching their script;
        // `shasum -a 256 -b` output is byte-identical. With neither, refuse.
        let runEnv = env
        if (spawnSync("sh", ["-c", "command -v sha256sum"], { stdio: "ignore" }).status !== 0) {
            if (spawnSync("sh", ["-c", "command -v shasum"], { stdio: "ignore" }).status !== 0) {
                log?.("warn", "refusing to install sharedserver — neither sha256sum nor shasum is available, so the download could not be checksum-verified")
                return undefined
            }
            const shim = join(lockdir, "sha256sum")
            writeFileSync(shim, '#!/bin/sh\nexec shasum -a 256 "$@"\n', { mode: 0o755 })
            runEnv = { ...env, PATH: `${lockdir}:${env.PATH ?? ""}` }
        }

        const run = spawnSync("sh", [script], { env: runEnv, stdio: "ignore" })
        if (run.error || run.status !== 0) {
            log?.("warn", `the sharedserver installer ran but failed (from ${url})`)
            return undefined
        }
        // The installer lands in ~/.cargo/bin or ~/.local/bin, both already in
        // CANDIDATE_BINARIES — re-probe rather than assuming a path.
        return probeAny(undefined, env)
    } finally {
        if (haveLock) {
            try {
                rmSync(lockdir, { recursive: true, force: true })
            } catch {
                /* best effort — a leaked lock costs the next run its bounded wait */
            }
        }
    }
}

// `sharedserver check` exit codes: 0 = active, 1 = grace, 2 = stopped.
type PreState = "active" | "grace" | "stopped" | "unknown"

function preCheck(binary: string, name: string, env: NodeJS.ProcessEnv): PreState {
    const result = spawnSync(binary, ["check", name], { stdio: "ignore", env })
    switch (result.status) {
        case 0:
            return "active"
        case 1:
            return "grace"
        case 2:
            return "stopped"
        default:
            return "unknown"
    }
}

type ServerInfo = { pid?: number; state?: string }

function readServerInfo(binary: string, name: string, env: NodeJS.ProcessEnv): ServerInfo | undefined {
    const result = spawnSync(binary, ["info", name, "--json"], { env })
    if (result.status !== 0) return undefined
    try {
        return JSON.parse(result.stdout.toString()) as ServerInfo
    } catch {
        return undefined
    }
}

function isPidAlive(pid: number): boolean {
    try {
        process.kill(pid, 0)
        return true
    } catch {
        return false
    }
}

function scheduleHealthCheck(
    binary: string,
    name: string,
    env: NodeJS.ProcessEnv,
    log: LogFn,
    toast: ToastFn,
    delayMs: number,
) {
    setTimeout(() => {
        const info = readServerInfo(binary, name, env)
        if (!info) {
            const msg = `${name}: health check failed (sharedserver info returned no data)`
            log("warn", msg)
            toast("warning", msg)
            return
        }
        if (info.state && info.state !== "active") {
            const msg = `${name}: server is not active after start (state: ${info.state})`
            log("error", msg)
            toast("error", msg)
            return
        }
        if (info.pid && !isPidAlive(info.pid)) {
            const msg = `${name}: server PID ${info.pid} died shortly after start`
            log("error", msg)
            toast("error", msg)
            return
        }
        log("info", `${name}: health check passed (pid=${info.pid}, state=${info.state})`)
    }, delayMs).unref()
}

function buildUseArgs(name: string, spec: ServerSpec, pid: number): string[] {
    const args = ["use", name, "--pid", String(pid)]
    if (spec.gracePeriod) args.push("--grace-period", spec.gracePeriod)
    if (spec.metadata) args.push("--metadata", spec.metadata)
    if (spec.logFile) args.push("--log-file", spec.logFile)
    for (const [k, v] of Object.entries(spec.env ?? {})) {
        args.push("--env", `${k}=${v}`)
    }
    if (!spec.lazy && spec.command) {
        args.push("--", spec.command, ...(spec.args ?? []))
    }
    return args
}

type Attached = { binary: string; name: string; env: NodeJS.ProcessEnv }

const attached: Attached[] = []
let cleanupInstalled = false

function installCleanup() {
    if (cleanupInstalled) return
    cleanupInstalled = true

    const drain = () => {
        while (attached.length) {
            const s = attached.pop()!
            // Synchronous spawn so this works from `exit` handlers too.
            spawnSync(s.binary, ["unuse", s.name, "--pid", String(process.pid)], {
                stdio: "ignore",
                env: s.env,
            })
        }
    }

    process.on("exit", drain)

    const signals: NodeJS.Signals[] = ["SIGINT", "SIGTERM", "SIGHUP"]
    for (const sig of signals) {
        process.on(sig, () => {
            drain()
            // Re-raise so the original signal semantics apply (e.g. exit code).
            process.kill(process.pid, sig)
        })
    }
}

// ── shared servers.json discovery (parity with the Claude Code plugin) ──────
//
// Both plugins read the same file so one config drives every client. The chain
// mirrors hooks/use-servers.sh exactly — explicit override, then a per-project
// config walked UP from the project dir, then the global one. First hit wins;
// a per-project file REPLACES the global rather than merging with it.

const PROJECT_CONFIG_NAMES = [".sharedserver.json", join(".sharedserver", "servers.json")]

function resolveConfigPath(
    opts: Options,
    env: NodeJS.ProcessEnv,
    cwd: string,
): string | undefined {
    const explicit = opts.config ?? env.SHAREDSERVER_CONFIG
    if (explicit && existsSync(explicit)) return explicit

    let dir = resolvePath(cwd)
    for (;;) {
        for (const name of PROJECT_CONFIG_NAMES) {
            const candidate = join(dir, name)
            if (existsSync(candidate)) return candidate
        }
        const parent = dirname(dir)
        if (parent === dir) break
        dir = parent
    }

    const global = join(homedir(), ".config", "sharedserver", "servers.json")
    return existsSync(global) ? global : undefined
}

/** Expand ${VAR} references in every string, mirroring the envsubst pass the
 *  Claude hook runs, so one file behaves identically in both clients. */
function expandVars<T>(value: T, env: NodeJS.ProcessEnv): T {
    if (typeof value === "string") {
        return value.replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (m, name) =>
            env[name] !== undefined ? (env[name] as string) : m,
        ) as unknown as T
    }
    if (Array.isArray(value)) return value.map((v) => expandVars(v, env)) as unknown as T
    if (value && typeof value === "object") {
        const out: Record<string, unknown> = {}
        for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
            out[k] = expandVars(v, env)
        }
        return out as unknown as T
    }
    return value
}

function loadServersFromFile(
    path: string,
    env: NodeJS.ProcessEnv,
    log: LogFn,
): Record<string, ServerSpec> {
    try {
        const parsed = JSON.parse(readFileSync(path, "utf8")) as { servers?: Record<string, ServerSpec> }
        const servers = expandVars(parsed.servers ?? {}, env)
        log("info", `loaded ${Object.keys(servers).length} server(s) from ${path}`)
        return servers
    } catch (err) {
        log("error", `could not read ${path}: ${err instanceof Error ? err.message : String(err)}`)
        return {}
    }
}

const SharedServerPlugin: Plugin = async ({ client }, options) => {
    const opts = (options ?? {}) as Options
    const notifyEnabled = opts.notify !== false

    const log: LogFn = (level, message) => {
        client.app
            .log({ body: { service: "sharedserver", level, message } })
            .catch(() => {})
    }

    const toast: ToastFn = (variant, message) => {
        if (!notifyEnabled) return
        // The plugin runs inside InstanceBootstrap, before the bus subscribers
        // that forward events to the TUI are wired up. Defer the toast so it
        // arrives after the TUI has subscribed. Best-effort: no TUI attached
        // (headless CLI) → request just no-ops.
        setTimeout(() => {
            log("info", `posting toast (${variant}): ${message}`)
            client.tui
                .showToast({ body: { title: "sharedserver", message, variant } })
                .then(
                    () => log("info", `toast posted (${variant}): ${message}`),
                    (err: unknown) =>
                        log(
                            "warn",
                            `toast post failed: ${err instanceof Error ? err.message : String(err)}`,
                        ),
                )
        }, 1500).unref()
    }

    const env: NodeJS.ProcessEnv = { ...process.env }
    if (opts.lockdir) env.SHAREDSERVER_LOCKDIR = opts.lockdir

    // Inline `servers` wins; otherwise fall back to the shared servers.json.
    let servers: Record<string, ServerSpec> = opts.servers ?? {}
    if (Object.keys(servers).length === 0) {
        const configPath = resolveConfigPath(opts, env, process.cwd())
        if (configPath) servers = loadServersFromFile(configPath, env, log)
    }

    // No servers is a normal state, not an error: it just means nothing is
    // configured. Stay quiet so an unconfigured install starts cleanly.
    if (Object.keys(servers).length === 0) return {}

    log(
        "info",
        `loaded options: binary=${opts.binary ?? "<auto>"} lockdir=${opts.lockdir ?? "<unset>"} ` +
            `servers=${JSON.stringify(servers)}`,
    )

    const binary = resolveBinary(opts.binary, env, log, toast)
    if (!binary) {
        const msg = "sharedserver binary not found; set `binary` option or install it on PATH"
        log("error", msg)
        toast("error", msg)
        return {}
    }

    installCleanup()

    const started: string[] = []
    const reattached: string[] = []
    for (const [name, spec] of Object.entries(servers)) {
        // skipIfEnv: another host already launched this one for us (e.g.
        // CodeCompanion injects MCP_COMPANION_COMBINER_URL). Don't start or
        // attach — matches the Claude hook's behaviour.
        if (spec.skipIfEnv && (env[spec.skipIfEnv] ?? "") !== "") {
            log("info", `skipping "${name}": ${spec.skipIfEnv} is set`)
            continue
        }
        if (!spec.command && !spec.lazy) {
            const keys = typeof spec === "object" && spec !== null ? Object.keys(spec) : []
            const msg =
                `server "${name}" has no \`command\` and is not lazy; skipping. ` +
                `Received keys: [${keys.join(", ")}]. ` +
                `Spec must be an object like { "command": "<bin>", "args": [...] }.`
            log("error", msg)
            toast("error", msg)
            continue
        }

        const pre = preCheck(binary, name, env)
        const args = buildUseArgs(name, spec, process.pid)
        const result = spawnSync(binary, args, { stdio: "pipe", env })

        if (result.error) {
            const msg = `${name}: failed to spawn sharedserver (${result.error.message})`
            log("error", msg)
            toast("error", msg)
            continue
        }
        if (result.status !== 0) {
            const stderr = result.stderr?.toString().trim()
            const msg = `${name}: sharedserver use exited ${result.status}${stderr ? ` (${stderr})` : ""}`
            log("error", msg)
            toast("error", msg)
            continue
        }

        attached.push({ binary, name, env })
        if (pre === "stopped" || pre === "unknown") {
            started.push(name)
            log("info", `started sharedserver "${name}"`)
        } else {
            reattached.push(name)
            log("info", `attached to running sharedserver "${name}" (was ${pre})`)
        }
        // Verify the wrapped binary is still alive 2.5s later. Catches the
        // case where `sharedserver use` reports success but the underlying
        // process crashes a moment later.
        scheduleHealthCheck(binary, name, env, log, toast, 2500)
    }

    const parts: string[] = []
    if (started.length) parts.push(`started ${started.join(", ")}`)
    if (reattached.length) parts.push(`attached ${reattached.join(", ")}`)
    if (parts.length) toast("success", parts.join("; "))

    return {}
}

export default SharedServerPlugin
