// OpenCode plugin: manage shared backend processes via the `sharedserver` CLI.
// See README.md for installation and configuration.

import { spawnSync } from "node:child_process"
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { dirname, join, resolve as resolvePath } from "node:path"
import { fileURLToPath } from "node:url"
import type { Plugin } from "@opencode-ai/plugin"
import { resolveSharedserver } from "./sharedserver-resolve.js"

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

const PLUGIN_VERSION: string | undefined = (() => {
    try {
        const here = dirname(fileURLToPath(import.meta.url))
        const pkg = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8")) as { version?: string }
        return pkg.version
    } catch {
        return undefined
    }
})()

// The resolver lives in its own module so it can be vendored byte-identical into the
// consuming OpenCode plugins. Here it runs in lockstep mode: no label, floor or URL
// overrides, so it pins to this package's own version.
function resolveBinary(
    override: string | undefined,
    env: NodeJS.ProcessEnv,
    log?: LogFn,
    toast?: ToastFn,
): string | undefined {
    return resolveSharedserver({ pkgVersion: PLUGIN_VERSION }, override, env, log, toast)
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
