// Pi extension: manage shared backend processes via the `sharedserver` CLI.
// See README.md for installation and configuration.
//
// It is the Pi counterpart of sharedserver's Claude Code and OpenCode plugins and
// mirrors the OpenCode one's behaviour — a generic, config-driven manager rather than a
// single-backend plugin:
//
//   1. Config — the same servers.json discovery chain the Claude hook and OpenCode
//      plugin use (explicit override → per-project file walked UP from cwd → global),
//      so one file drives every client. `${VAR}` references are expanded identically.
//   2. Process — on `session_start`, drive `sharedserver use … -- <argv>` for each
//      configured server so each warm backend is running and refcounted (shared across
//      clients). Released on `session_shutdown` when `reason === "quit"`
//      (reload/resume/fork keep the processes and re-attach).
//   3. Health — 2.5s after start, verify each wrapped process is still alive.
//
// Config comes from the servers.json chain plus a few env knobs (no inline options, as
// Pi extensions receive none): $SHAREDSERVER_BIN, $SHAREDSERVER_LOCKDIR,
// $SHAREDSERVER_CONFIG, $PI_SHAREDSERVER_NOTIFY. The sharedserver resolution and the
// servers.json handling are ported byte-for-byte from plugins/opencode.

import { spawnSync } from "node:child_process"
import { existsSync, readFileSync } from "node:fs"
import { homedir } from "node:os"
import { dirname, join, resolve as resolvePath } from "node:path"
import { fileURLToPath } from "node:url"
import type { ExtensionContext, ExtensionAPI, SessionShutdownEvent } from "./pi.js"
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

type LogFn = (level: "info" | "warn" | "error", message: string) => void

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
// consuming plugins. Here it runs in lockstep mode: no label, floor or URL overrides,
// so it pins to this package's own version.
function resolveBinary(override: string | undefined, env: NodeJS.ProcessEnv, log?: LogFn): string | undefined {
    return resolveSharedserver({ pkgVersion: PLUGIN_VERSION }, override, env, log)
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
    delayMs: number,
) {
    setTimeout(() => {
        const info = readServerInfo(binary, name, env)
        if (!info) {
            log("warn", `${name}: health check failed (sharedserver info returned no data)`)
            return
        }
        if (info.state && info.state !== "active") {
            log("error", `${name}: server is not active after start (state: ${info.state})`)
            return
        }
        if (info.pid && !isPidAlive(info.pid)) {
            log("error", `${name}: server PID ${info.pid} died shortly after start`)
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

function drainAttached() {
    while (attached.length) {
        const s = attached.pop()!
        spawnSync(s.binary, ["unuse", s.name, "--pid", String(process.pid)], { stdio: "ignore", env: s.env })
    }
}

// ── shared servers.json discovery (parity with the Claude Code and OpenCode plugins) ──
//
// Every client reads the same file so one config drives them all. The chain mirrors
// hooks/use-servers.sh exactly — explicit override, then a per-project config walked UP
// from the project dir, then the global one. First hit wins; a per-project file REPLACES
// the global rather than merging with it.

const PROJECT_CONFIG_NAMES = [".sharedserver.json", join(".sharedserver", "servers.json")]

function resolveConfigPath(explicitOverride: string | undefined, env: NodeJS.ProcessEnv, cwd: string): string | undefined {
    const explicit = explicitOverride ?? env.SHAREDSERVER_CONFIG
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

/** Expand ${VAR} references in every string, mirroring the envsubst pass the Claude hook
 *  runs, so one file behaves identically in every client. */
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

function loadServersFromFile(path: string, env: NodeJS.ProcessEnv, log: LogFn): Record<string, ServerSpec> {
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

// ── env configuration ───────────────────────────────────────────────
function env(name: string): string | undefined {
    const v = process.env[name]
    return v !== undefined && v !== "" ? v : undefined
}

function makeLog(ctx: ExtensionContext, notify: boolean): LogFn {
    return (level, message) => {
        const line = `sharedserver: ${message}`
        if (notify && ctx.hasUI && ctx.ui?.notify) {
            ctx.ui.notify(line, level === "error" ? "error" : level === "warn" ? "warn" : "info")
        } else if (level === "error" || level === "warn") {
            process.stderr.write(`${line}\n`)
        }
    }
}

// ── the extension ────────────────────────────────────────────────────

export default function sharedserverPi(pi: ExtensionAPI): void {
    const notifyEnabled = env("PI_SHAREDSERVER_NOTIFY") !== "false"

    // Start/attach on session_start; release on session_shutdown("quit"). A session that
    // reloads/resumes/forks keeps its processes and re-attaches, so guard against a
    // second start within the same process.
    pi.on("session_start", (_event, ctx) => {
        if (attached.length > 0) return

        const log = makeLog(ctx, notifyEnabled)

        const childEnv: NodeJS.ProcessEnv = { ...process.env }
        const lockdir = env("SHAREDSERVER_LOCKDIR")
        if (lockdir) childEnv.SHAREDSERVER_LOCKDIR = lockdir

        const configPath = resolveConfigPath(undefined, childEnv, ctx.cwd ?? process.cwd())
        const servers = configPath ? loadServersFromFile(configPath, childEnv, log) : {}

        // No servers is a normal state, not an error: nothing is configured. Stay quiet
        // so an unconfigured install starts cleanly.
        if (Object.keys(servers).length === 0) return

        const binary = resolveBinary(env("SHAREDSERVER_BIN"), childEnv, log)
        if (!binary) {
            log("error", "sharedserver binary not found; set $SHAREDSERVER_BIN or install it on PATH")
            return
        }

        installCleanup()

        const started: string[] = []
        const reattached: string[] = []
        for (const [name, spec] of Object.entries(servers)) {
            // skipIfEnv: another host already launched this one for us. Don't start or
            // attach — matches the Claude hook's behaviour.
            if (spec.skipIfEnv && (childEnv[spec.skipIfEnv] ?? "") !== "") {
                log("info", `skipping "${name}": ${spec.skipIfEnv} is set`)
                continue
            }
            if (!spec.command && !spec.lazy) {
                const keys = typeof spec === "object" && spec !== null ? Object.keys(spec) : []
                log(
                    "error",
                    `server "${name}" has no \`command\` and is not lazy; skipping. ` +
                        `Received keys: [${keys.join(", ")}]. ` +
                        `Spec must be an object like { "command": "<bin>", "args": [...] }.`,
                )
                continue
            }

            const pre = preCheck(binary, name, childEnv)
            const args = buildUseArgs(name, spec, process.pid)
            const result = spawnSync(binary, args, { stdio: "pipe", env: childEnv })

            if (result.error) {
                log("error", `${name}: failed to spawn sharedserver (${result.error.message})`)
                continue
            }
            if (result.status !== 0) {
                const stderr = result.stderr?.toString().trim()
                log("error", `${name}: sharedserver use exited ${result.status}${stderr ? ` (${stderr})` : ""}`)
                continue
            }

            attached.push({ binary, name, env: childEnv })
            if (pre === "stopped" || pre === "unknown") {
                started.push(name)
                log("info", `started sharedserver "${name}"`)
            } else {
                reattached.push(name)
                log("info", `attached to running sharedserver "${name}" (was ${pre})`)
            }
            // Verify the wrapped binary is still alive 2.5s later. Catches the case where
            // `sharedserver use` reports success but the underlying process crashes.
            scheduleHealthCheck(binary, name, childEnv, log, 2500)
        }

        const parts: string[] = []
        if (started.length) parts.push(`started ${started.join(", ")}`)
        if (reattached.length) parts.push(`attached ${reattached.join(", ")}`)
        if (parts.length) log("info", parts.join("; "))
    })

    pi.on("session_shutdown", (event: SessionShutdownEvent) => {
        if (event.reason === "quit") drainAttached()
    })
}
