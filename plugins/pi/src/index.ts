// Pi extension: manage shared backend processes via the `sharedserver` CLI.
// See README.md for installation and configuration.
//
// It is the Pi counterpart of sharedserver's Claude Code and OpenCode plugins.
// The `sharedserver` binary now owns config parsing, so this extension no longer
// reads servers.json itself — it drives one profile:
//
//   1. On `session_start`, run `sharedserver up --profile <host> --json` for this
//      host's profile. The binary discovers the config (from --cwd), expands
//      ${VAR}, selects the profile's servers (plus universal, profile-less ones),
//      and starts/attaches each; the JSON report says exactly what came up.
//   2. Health — 2.5s after start, verify each server it reported started/attached
//      is still alive (`sharedserver info --json`).
//   3. On `session_shutdown` ("quit"), run `sharedserver down --profile <host>` to
//      release. reload/resume/fork keep the processes and re-attach.
//
// The profile defaults to "pi"; override with $PI_SHAREDSERVER_PROFILE. A config
// with no `profiles` brings up every server (all universal), exactly as before.
// Env knobs: $SHAREDSERVER_BIN, $SHAREDSERVER_LOCKDIR, $SHAREDSERVER_CONFIG,
// $PI_SHAREDSERVER_NOTIFY, $PI_SHAREDSERVER_PROFILE.

import { spawnSync } from "node:child_process"
import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import type { ExtensionAPI, ExtensionContext, SessionShutdownEvent } from "./pi.js"
import { resolveSharedserver } from "./sharedserver-resolve.js"

type LogFn = (level: "info" | "warn" | "error", message: string) => void

/** One server's outcome in an `up`/`down --json` report. */
type ServerOutcome = "started" | "attached" | "skipped" | "failed" | "released"
type ReportServer = { name: string; outcome: ServerOutcome; pid?: number; reason?: string }
type Report = { profile: string; servers: ReportServer[]; warnings?: string[] }

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
function scheduleHealthCheck(binary: string, name: string, env: NodeJS.ProcessEnv, log: LogFn, delayMs: number) {
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

// ── env helpers ──────────────────────────────────────────────────────
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

/** Run `up`/`down --profile <profile> --json` and parse the report. Returns null
 *  on spawn failure, a non-zero exit (a hard error like unreadable config), or
 *  unparseable output — the message is logged in each case. */
function runProfile(
    verb: "up" | "down",
    binary: string,
    profile: string,
    pid: number,
    cwd: string,
    env: NodeJS.ProcessEnv,
    log: LogFn,
): Report | undefined {
    const args = [verb, "--profile", profile, "--pid", String(pid), "--cwd", cwd, "--profile-optional", "--json"]
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

// Teardown state for the current session, recorded on the first `up` so exit/quit
// paths can `down` the same profile. Guards against a second `up` within one process
// (reload/resume/fork re-enter session_start but keep the processes attached).
type Session = { binary: string; profile: string; pid: number; cwd: string; env: NodeJS.ProcessEnv }
let session: Session | undefined

function tearDown(log: LogFn) {
    if (!session) return
    const s = session
    session = undefined
    runProfile("down", s.binary, s.profile, s.pid, s.cwd, s.env, log)
}

/** Process-global guard — jiti loads extensions with moduleCache:false, so every
 *  session bind (parent + each subagent child) re-instantiates this module with
 *  fresh state; a module-level flag resets and stacks another handler per bind.
 *  Symbol.for survives the isolation, installing the handlers once per process. */
const CLEANUP_GUARD = Symbol.for("pi-sharedserver:cleanup-installed")

function installCleanup(log: LogFn) {
    const g = globalThis as Record<symbol, boolean | undefined>
    if (g[CLEANUP_GUARD]) return
    g[CLEANUP_GUARD] = true
    process.on("exit", () => tearDown(log))
    for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"] as NodeJS.Signals[]) {
        const handler = () => {
            tearDown(log)
            // Remove our listener before re-raising: re-killing with the handler
            // still installed re-delivers the signal to it forever (loop verified —
            // ctrl+c wedged pi instead of exiting). Gone, the re-raise falls
            // through to the default disposition and terminates.
            process.removeListener(sig, handler)
            process.kill(process.pid, sig)
        }
        process.on(sig, handler)
    }
}

// ── the extension ────────────────────────────────────────────────────

export default function sharedserverPi(pi: ExtensionAPI): void {
    const notifyEnabled = env("PI_SHAREDSERVER_NOTIFY") !== "false"
    const profile = env("PI_SHAREDSERVER_PROFILE") ?? "pi"

    // ── /sharedserver slash command ──────────────────────────────────
    // On-demand control + introspection the auto-lifecycle can't give:
    //   /sharedserver status                 what's running
    //   /sharedserver up   <profile>         bring a (task) profile up on demand
    //   /sharedserver down <profile>         release it
    //   /sharedserver config show            the whole config
    //   /sharedserver config lookup <name>   one server's def + profiles
    // Config *mutations* (register/unregister) are deliberately NOT here — those
    // are install-time edits, not in-session actions.
    const VERBS = ["status", "up", "down", "config"]
    const CONFIG_SUBS = ["show", "lookup"]

    pi.registerCommand("sharedserver", {
        description: "sharedserver — status | up <profile> | down <profile> | config show | config lookup <name>",
        getArgumentCompletions: (prefix) => {
            const toks = prefix.split(/\s+/)
            if (toks.length <= 1) {
                return VERBS.filter((v) => v.startsWith(toks[0] ?? "")).map((v) => ({ value: v }))
            }
            if (toks[0] === "config" && toks.length === 2) {
                return CONFIG_SUBS.filter((s) => s.startsWith(toks[1] ?? "")).map((s) => ({ value: s }))
            }
            return null
        },
        handler: async (args, ctx) => {
            const log = makeLog(ctx, notifyEnabled)
            const toks = args.trim().split(/\s+/).filter(Boolean)
            const verb = toks[0] ?? "status"

            const childEnv: NodeJS.ProcessEnv = { ...process.env }
            const lockdir = env("SHAREDSERVER_LOCKDIR")
            if (lockdir) childEnv.SHAREDSERVER_LOCKDIR = lockdir
            const binary = resolveBinary(env("SHAREDSERVER_BIN"), childEnv, log)
            if (!binary) {
                ctx.ui?.notify?.("sharedserver: binary not found", "error")
                return
            }
            const run = (cliArgs: string[]) => pi.exec(binary, cliArgs, { env: childEnv })
            const emit = (content: string) => pi.sendMessage({ customType: "sharedserver", content, display: true })

            switch (verb) {
                case "status": {
                    const r = await run(["list"])
                    emit(r.stdout.trim() || "(no servers running)")
                    return
                }
                case "up":
                case "down": {
                    const prof = toks[1] ?? profile
                    const r = await run([verb, "--profile", prof, "--pid", String(process.pid), "--profile-optional"])
                    const line = (r.stdout || r.stderr).trim() || `${verb} ${prof}: done`
                    ctx.ui?.notify?.(`sharedserver: ${line}`, r.code === 0 ? "info" : "error")
                    return
                }
                case "config": {
                    const sub = toks[1]
                    if (sub === "show") {
                        emit((await run(["config", "show"])).stdout.trim() || "(empty config)")
                    } else if (sub === "lookup") {
                        const name = toks[2]
                        if (!name) {
                            ctx.ui?.notify?.("usage: /sharedserver config lookup <name>", "warn")
                            return
                        }
                        emit((await run(["config", "lookup", name])).stdout.trim() || `'${name}' not registered`)
                    } else {
                        ctx.ui?.notify?.(`config: unknown sub "${sub ?? ""}". Try: show, lookup <name>`, "warn")
                    }
                    return
                }
                default:
                    ctx.ui?.notify?.(`sharedserver: unknown verb "${verb}". Try: ${VERBS.join(", ")}`, "warn")
            }
        },
    })

    pi.on("session_start", (_event, ctx) => {
        if (session) return // already up for this process

        const log = makeLog(ctx, notifyEnabled)

        const childEnv: NodeJS.ProcessEnv = { ...process.env }
        const lockdir = env("SHAREDSERVER_LOCKDIR")
        if (lockdir) childEnv.SHAREDSERVER_LOCKDIR = lockdir

        const binary = resolveBinary(env("SHAREDSERVER_BIN"), childEnv, log)
        if (!binary) {
            log("error", "sharedserver binary not found; set $SHAREDSERVER_BIN or install it on PATH")
            return
        }

        const cwd = ctx.cwd ?? process.cwd()
        const report = runProfile("up", binary, profile, process.pid, cwd, childEnv, log)
        if (!report) return

        for (const w of report.warnings ?? []) log("warn", w)

        const started: string[] = []
        const attached: string[] = []
        for (const s of report.servers) {
            if (s.outcome === "started" || s.outcome === "attached") {
                ;(s.outcome === "started" ? started : attached).push(s.name)
                // Verify liveness shortly after — `up` may report success just before a crash.
                scheduleHealthCheck(binary, s.name, childEnv, log, 2500)
            } else if (s.outcome === "failed") {
                log("error", `${s.name}: failed to start${s.reason ? ` (${s.reason})` : ""}`)
            } else if (s.outcome === "skipped") {
                log("info", `${s.name}: skipped${s.reason ? ` (${s.reason})` : ""}`)
            }
        }

        // Nothing configured (no config / empty profile) is a normal, quiet state.
        if (started.length === 0 && attached.length === 0) {
            if (report.servers.length > 0) log("info", `profile "${profile}": nothing started`)
            return
        }

        // Record teardown state and install exit hooks now that we hold references.
        session = { binary, profile, pid: process.pid, cwd, env: childEnv }
        installCleanup(log)

        const parts: string[] = []
        if (started.length) parts.push(`started ${started.join(", ")}`)
        if (attached.length) parts.push(`attached ${attached.join(", ")}`)
        log("info", `profile "${profile}": ${parts.join("; ")}`)
    })

    pi.on("session_shutdown", (event: SessionShutdownEvent, ctx) => {
        if (event.reason === "quit") tearDown(makeLog(ctx, notifyEnabled))
    })
}
