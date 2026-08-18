// OpenCode plugin: manage shared backend processes via the `sharedserver` CLI.
// See README.md for installation and configuration.
//
// The `sharedserver` binary now owns config parsing, so this plugin drives one
// profile rather than looping per server: on load it runs
// `sharedserver up --profile <host> --json` (the binary discovers the config,
// expands ${VAR}, and selects the profile's servers plus any universal ones),
// health-checks what the JSON report says came up, and on process exit runs
// `sharedserver down --profile <host>` to release. Config given inline via the
// `servers` option is materialized to a temp file so it flows through the binary
// the same way a servers.json does.

import { spawnSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
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
     *  config file; materialized to a temp servers.json so it flows through the
     *  binary like a file config. */
    servers?: Record<string, ServerSpec>
    /** Optional named profiles ({ "<profile>": ["<server>", ...] }) for inline
     *  configs — groups the inline `servers` the same way a file config's
     *  `profiles` map does. */
    profiles?: Record<string, string[]>
    /** Explicit path to a servers.json. Overrides the discovery chain. */
    config?: string
    /** Profile this session brings up. Default "opencode". */
    profile?: string
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

// ── health check ─────────────────────────────────────────────────────
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

/** Verify a server the profile brought up is still alive `delayMs` later — catches
 *  the case where `up` reported success but the process crashed a moment after. */
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

// ── up/down --json driver ────────────────────────────────────────────
type ServerOutcome = "started" | "attached" | "skipped" | "failed" | "released"
type ReportServer = { name: string; outcome: ServerOutcome; pid?: number; reason?: string }
type Report = { profile: string; servers: ReportServer[]; warnings?: string[] }

function runProfile(
    verb: "up" | "down",
    binary: string,
    profile: string,
    pid: number,
    cwd: string,
    configArg: string[],
    env: NodeJS.ProcessEnv,
    log: LogFn,
): Report | undefined {
    const args = [verb, "--profile", profile, "--pid", String(pid), "--cwd", cwd, "--profile-optional", "--json", ...configArg]
    const result = spawnSync(binary, args, { env })
    if (result.error) {
        log("error", `${verb}: failed to spawn sharedserver (${result.error.message})`)
        return undefined
    }
    if (result.status !== 0) {
        const stderr = result.stderr?.toString().trim()
        log("error", `${verb} --profile ${profile} exited ${result.status}${stderr ? ` (${stderr})` : ""}`)
        return undefined
    }
    try {
        return JSON.parse(result.stdout.toString()) as Report
    } catch (err) {
        log("error", `${verb}: could not parse JSON report (${err instanceof Error ? err.message : String(err)})`)
        return undefined
    }
}

// Teardown state for this process, recorded on the first `up` so exit/signal
// paths can `down` the same profile and remove any temp config. Guards against a
// second `up` if the plugin is initialized more than once in one process.
type Session = {
    binary: string
    profile: string
    pid: number
    cwd: string
    configArg: string[]
    tempDir?: string
    env: NodeJS.ProcessEnv
}
let session: Session | undefined
let cleanupInstalled = false

function tearDown(log: LogFn) {
    if (!session) return
    const s = session
    session = undefined
    runProfile("down", s.binary, s.profile, s.pid, s.cwd, s.configArg, s.env, log)
    if (s.tempDir) {
        try {
            rmSync(s.tempDir, { recursive: true, force: true })
        } catch {
            /* best effort */
        }
    }
}

function installCleanup(log: LogFn) {
    if (cleanupInstalled) return
    cleanupInstalled = true
    process.on("exit", () => tearDown(log))
    for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"] as NodeJS.Signals[]) {
        process.on(sig, () => {
            tearDown(log)
            process.kill(process.pid, sig) // re-raise so original signal semantics apply
        })
    }
}

/** Materialize an inline `servers`/`profiles` config to a temp servers.json and
 *  return its dir + path, so `up`/`down` read it like any file config. */
function materializeInline(
    servers: Record<string, ServerSpec>,
    profiles: Record<string, string[]> | undefined,
): { dir: string; path: string } {
    const dir = mkdtempSync(join(tmpdir(), "sharedserver-opencode-"))
    const path = join(dir, "servers.json")
    writeFileSync(path, JSON.stringify({ servers, ...(profiles ? { profiles } : {}) }))
    return { dir, path }
}

const SharedServerPlugin: Plugin = async ({ client }, options) => {
    const opts = (options ?? {}) as Options
    const notifyEnabled = opts.notify !== false
    const profile = opts.profile ?? process.env.OPENCODE_SHAREDSERVER_PROFILE ?? "opencode"

    const log: LogFn = (level, message) => {
        client.app.log({ body: { service: "sharedserver", level, message } }).catch(() => {})
    }

    const toast: ToastFn = (variant, message) => {
        if (!notifyEnabled) return
        // The plugin runs inside InstanceBootstrap, before the bus subscribers that
        // forward events to the TUI are wired up. Defer the toast so it arrives after
        // the TUI has subscribed. Best-effort: no TUI attached → request no-ops.
        setTimeout(() => {
            client.tui.showToast({ body: { title: "sharedserver", message, variant } }).catch(() => {})
        }, 1500).unref()
    }

    if (session) return {} // already up in this process

    const env: NodeJS.ProcessEnv = { ...process.env }
    if (opts.lockdir) env.SHAREDSERVER_LOCKDIR = opts.lockdir

    const binary = resolveBinary(opts.binary, env, log, toast)
    if (!binary) {
        const msg = "sharedserver binary not found; set `binary` option or install it on PATH"
        log("error", msg)
        toast("error", msg)
        return {}
    }

    // Config source: inline `servers` (materialized) wins; else an explicit
    // `config` path; else the binary's own discovery chain from --cwd.
    let configArg: string[] = []
    let tempDir: string | undefined
    if (opts.servers && Object.keys(opts.servers).length > 0) {
        const mat = materializeInline(opts.servers, opts.profiles)
        tempDir = mat.dir
        configArg = ["--config", mat.path]
    } else if (opts.config) {
        configArg = ["--config", opts.config]
    }

    const cwd = process.cwd()
    const report = runProfile("up", binary, profile, process.pid, cwd, configArg, env, log)
    if (!report) {
        if (tempDir) {
            try {
                rmSync(tempDir, { recursive: true, force: true })
            } catch {
                /* best effort */
            }
        }
        return {}
    }

    for (const w of report.warnings ?? []) log("warn", w)

    const started: string[] = []
    const reattached: string[] = []
    for (const s of report.servers) {
        if (s.outcome === "started" || s.outcome === "attached") {
            ;(s.outcome === "started" ? started : reattached).push(s.name)
            scheduleHealthCheck(binary, s.name, env, log, toast, 2500)
        } else if (s.outcome === "failed") {
            const msg = `${s.name}: failed to start${s.reason ? ` (${s.reason})` : ""}`
            log("error", msg)
            toast("error", msg)
        } else if (s.outcome === "skipped") {
            log("info", `${s.name}: skipped${s.reason ? ` (${s.reason})` : ""}`)
        }
    }

    // Nothing configured (no config / empty profile) is a normal, quiet state.
    if (started.length === 0 && reattached.length === 0) {
        if (tempDir) {
            try {
                rmSync(tempDir, { recursive: true, force: true })
            } catch {
                /* best effort */
            }
        }
        return {}
    }

    session = { binary, profile, pid: process.pid, cwd, configArg, tempDir, env }
    installCleanup(log)

    const parts: string[] = []
    if (started.length) parts.push(`started ${started.join(", ")}`)
    if (reattached.length) parts.push(`attached ${reattached.join(", ")}`)
    if (parts.length) toast("success", parts.join("; "))

    return {}
}

export default SharedServerPlugin
