//! Shared-server configuration: the `servers` + `profiles` document that the
//! editor plugins used to each parse for themselves (Claude hook via jq,
//! OpenCode/Pi in TypeScript, Neovim in Lua). The binary now owns parsing so the
//! logic lives in one place and `${VAR}` expansion, discovery, and profile
//! selection behave identically for every client.
//!
//! Design notes:
//! - The schema is intentionally byte-compatible with the existing
//!   `servers.json` the plugins already read: camelCase keys (`gracePeriod`,
//!   `logFile`, `skipIfEnv`), and a server with no profile membership is
//!   "universal" — it is selected for every profile, which reproduces today's
//!   "every client brings up everything" as the default.
//! - Reads are lossy on purpose: unknown keys are ignored here (see serde
//!   defaults). The config-*edit* verbs (register/unregister) operate on
//!   `serde_json::Value` instead, so a rewrite never drops fields it does not
//!   model — "edits are JSON ops".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One managed server. Field names mirror the existing `servers.json` exactly so
/// a file authored for the plugins loads unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerSpec {
    /// Binary to run. Required unless `lazy` — a lazy entry only attaches to an
    /// already-running server and never starts one, so it needs no command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to `command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra environment variables forwarded to the server via `--env KEY=VALUE`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Grace period before shutdown once the last client leaves (e.g. "30m").
    #[serde(
        rename = "gracePeriod",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub grace_period: Option<String>,
    /// Capture the managed server's stdout/stderr to this path.
    #[serde(rename = "logFile", default, skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    /// Opaque client metadata forwarded to sharedserver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    /// Attach-only: never start, only attach if already running.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lazy: bool,
    /// Name of an env var; when it is set (non-empty) this server is skipped
    /// entirely for the current client — another host already launched it.
    #[serde(rename = "skipIfEnv", default, skip_serializing_if = "Option::is_none")]
    pub skip_if_env: Option<String>,
    /// Owning scope, stamped by `config register --scope <id>` so `unregister` is
    /// precise and cross-scope clashes are attributable. Absent for hand-authored
    /// entries. Carried through here so selection can report it; the edit verbs
    /// are the authority on writing it.
    #[serde(rename = "_scope", default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The whole config document: a flat map of servers plus named profiles that
/// group server names. A "host" (claude/opencode/pi/neovim) is just a reserved
/// profile name — there is no separate host axis.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub servers: BTreeMap<String, ServerSpec>,
    /// Profile name -> the server names it includes. A server absent from every
    /// profile is universal (selected for any profile).
    #[serde(default)]
    pub profiles: BTreeMap<String, Vec<String>>,
}

/// A server chosen by [`select`], resolved to its spec.
#[derive(Debug, Clone, PartialEq)]
pub struct Selected {
    pub name: String,
    pub spec: ServerSpec,
}

/// The result of resolving a profile against a config: the servers to act on,
/// plus non-fatal warnings (unknown profile, dangling profile member) that the
/// caller should surface but need not treat as errors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub servers: Vec<Selected>,
    pub warnings: Vec<String>,
}

// ── ${VAR} expansion ────────────────────────────────────────────────────────
//
// Matches the envsubst / TypeScript behaviour the plugins used: only `${NAME}`
// (braced) is expanded, `NAME` is `[A-Za-z_][A-Za-z0-9_]*`, and an undefined var
// is left verbatim rather than blanked. Implemented by hand to avoid pulling in
// a regex dependency for one small pattern.

fn expand_str(input: &str, lookup: &impl Fn(&str) -> Option<String>) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Scan a `${NAME}` with a valid identifier and closing brace.
            if let Some((name, end)) = scan_var(bytes, i + 2) {
                if let Some(val) = lookup(name) {
                    out.push_str(&val);
                } else {
                    // Undefined: leave the reference literally, like envsubst's
                    // and the TS plugins' behaviour.
                    out.push_str(&input[i..end]);
                }
                i = end;
                continue;
            }
        }
        // Not a var opener (or malformed): copy the byte. `input` is valid UTF-8
        // and we only ever branch on ASCII, so byte-at-a-time is safe here.
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// From the byte after `${`, return `(name, index-past-closing-brace)` if a
/// valid `${NAME}` runs from here, else `None` (malformed → copied verbatim).
fn scan_var(bytes: &[u8], start: usize) -> Option<(&str, usize)> {
    let mut j = start;
    while j < bytes.len() {
        let c = bytes[j];
        let ident = c == b'_' || c.is_ascii_alphanumeric();
        if c == b'}' {
            if j == start {
                return None; // empty `${}`
            }
            let first = bytes[start];
            if first.is_ascii_digit() {
                return None; // identifiers cannot start with a digit
            }
            let name = std::str::from_utf8(&bytes[start..j]).ok()?;
            return Some((name, j + 1));
        }
        if !ident {
            return None; // invalid char before a closing brace
        }
        j += 1;
    }
    None // no closing brace
}

/// Expand `${VAR}` in every JSON string in the tree, in place. Applied before
/// typed deserialization so expansion reaches command/args/env/etc. uniformly —
/// the same "expand all strings" the plugins do.
fn expand_json(value: &mut serde_json::Value, lookup: &impl Fn(&str) -> Option<String>) {
    match value {
        serde_json::Value::String(s) => *s = expand_str(s, lookup),
        serde_json::Value::Array(items) => {
            for item in items {
                expand_json(item, lookup);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                expand_json(v, lookup);
            }
        }
        _ => {}
    }
}

// ── loading ─────────────────────────────────────────────────────────────────

/// Parse a config document from JSON text, expanding `${VAR}` via `lookup`.
/// Split from file I/O so it is directly unit-testable.
pub fn parse_config(text: &str, lookup: &impl Fn(&str) -> Option<String>) -> Result<Config> {
    let mut value: serde_json::Value =
        serde_json::from_str(text).context("config is not valid JSON")?;
    expand_json(&mut value, lookup);
    let cfg: Config = serde_json::from_value(value).context("config does not match the schema")?;
    Ok(cfg)
}

/// Read and parse a config file, expanding `${VAR}` from the process environment.
pub fn load_config(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read config file: {}", path.display()))?;
    parse_config(&text, &|name| std::env::var(name).ok())
}

// ── discovery ───────────────────────────────────────────────────────────────
//
// Mirrors the chain every plugin used, so one file drives them all: explicit
// override -> per-project file walked UP from the project dir -> global. First
// existing file wins; a per-project file REPLACES the global rather than merging.

/// Per-project config file names, checked at each level of the upward walk.
const PROJECT_CONFIG_NAMES: [&str; 2] = [".sharedserver.json", ".sharedserver/servers.json"];

/// Resolve which single config file to use, or `None` if nothing is configured
/// (a normal, non-error state). `explicit` is a caller-provided override (e.g. a
/// `--config` flag); `SHAREDSERVER_CONFIG` is consulted next; then the upward
/// walk from `cwd`; then the global file under `$HOME`.
pub fn discover_config_path(explicit: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    if let Ok(env_path) = std::env::var("SHAREDSERVER_CONFIG") {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            return Some(p);
        }
    }

    let mut dir = Some(cwd.to_path_buf());
    while let Some(d) = dir {
        for name in PROJECT_CONFIG_NAMES {
            let candidate = d.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }

    if let Ok(home) = std::env::var("HOME") {
        let global = PathBuf::from(home)
            .join(".config")
            .join("sharedserver")
            .join("servers.json");
        if global.is_file() {
            return Some(global);
        }
    }
    None
}

// ── selection ───────────────────────────────────────────────────────────────

/// Server names that appear in at least one profile.
fn referenced_servers(cfg: &Config) -> BTreeSet<&str> {
    cfg.profiles
        .values()
        .flat_map(|names| names.iter().map(String::as_str))
        .collect()
}

/// Select the servers to act on for `profile`: the profile's members plus every
/// universal (profile-less) server, resolved to their specs in a deterministic
/// (name-sorted) order so `up` and `down` agree. Dangling profile members always
/// warn (a real config error). A *missing* profile warns only when
/// `warn_missing_profile` is set: a human typing `up --profile typo` wants the
/// warning, but a plugin asking for its own host profile that the user never
/// defined is a normal, back-compat case that should stay silent.
pub fn select(cfg: &Config, profile: &str, warn_missing_profile: bool) -> Selection {
    let mut warnings = Vec::new();
    let referenced = referenced_servers(cfg);

    // Universal servers: defined but named by no profile. These reproduce the
    // old "everything comes up" default for any selector.
    let universal: BTreeSet<&str> = cfg
        .servers
        .keys()
        .map(String::as_str)
        .filter(|n| !referenced.contains(n))
        .collect();

    let members: Vec<&str> = match cfg.profiles.get(profile) {
        Some(list) => list.iter().map(String::as_str).collect(),
        None => {
            // No such profile. Flag it only for the CLI, and only when profiles
            // exist at all (a config with none is pure back-compat).
            if warn_missing_profile && !cfg.profiles.is_empty() {
                warnings.push(format!(
                    "unknown profile '{profile}'; bringing up only universal (profile-less) servers"
                ));
            }
            Vec::new()
        }
    };

    // Union of the profile's members and the universal set, deduped + sorted.
    let mut chosen: BTreeSet<&str> = universal;
    for name in members {
        if cfg.servers.contains_key(name) {
            chosen.insert(name);
        } else {
            warnings.push(format!(
                "profile '{profile}' lists unknown server '{name}'; skipping"
            ));
        }
    }

    let servers = chosen
        .into_iter()
        .map(|name| Selected {
            name: name.to_string(),
            spec: cfg.servers[name].clone(),
        })
        .collect();

    Selection { servers, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn parses_camelcase_and_defaults() {
        let text = r#"{
            "servers": {
                "chroma": {
                    "command": "chroma",
                    "args": ["run", "--path", "/data"],
                    "gracePeriod": "30m",
                    "logFile": "/tmp/chroma.log",
                    "skipIfEnv": "MCP_URL",
                    "env": { "ANONYMIZED_TELEMETRY": "False" }
                },
                "watchman": { "lazy": true }
            },
            "profiles": { "opencode": ["chroma"] }
        }"#;
        let cfg = parse_config(text, &no_env).unwrap();
        let chroma = &cfg.servers["chroma"];
        assert_eq!(chroma.command.as_deref(), Some("chroma"));
        assert_eq!(chroma.args, vec!["run", "--path", "/data"]);
        assert_eq!(chroma.grace_period.as_deref(), Some("30m"));
        assert_eq!(chroma.log_file.as_deref(), Some("/tmp/chroma.log"));
        assert_eq!(chroma.skip_if_env.as_deref(), Some("MCP_URL"));
        assert_eq!(chroma.env["ANONYMIZED_TELEMETRY"], "False");
        assert!(cfg.servers["watchman"].lazy);
        assert_eq!(cfg.profiles["opencode"], vec!["chroma"]);
    }

    #[test]
    fn ignores_unknown_keys() {
        let cfg = parse_config(
            r#"{ "servers": { "a": { "command": "a", "futureField": 1 } }, "extraTopLevel": true }"#,
            &no_env,
        )
        .unwrap();
        assert_eq!(cfg.servers["a"].command.as_deref(), Some("a"));
    }

    #[test]
    fn expands_defined_vars_and_leaves_undefined() {
        let lookup = |name: &str| match name {
            "HOME" => Some("/home/geo".to_string()),
            _ => None,
        };
        let cfg = parse_config(
            r#"{ "servers": { "a": { "command": "${HOME}/bin/a", "args": ["${MISSING}", "x${HOME}y"] } } }"#,
            &lookup,
        )
        .unwrap();
        assert_eq!(cfg.servers["a"].command.as_deref(), Some("/home/geo/bin/a"));
        assert_eq!(cfg.servers["a"].args, vec!["${MISSING}", "x/home/geoy"]);
    }

    #[test]
    fn expand_edge_cases() {
        let lookup = |n: &str| (n == "V").then(|| "ok".to_string());
        // malformed / empty / digit-leading are left verbatim
        assert_eq!(expand_str("${}", &lookup), "${}");
        assert_eq!(expand_str("${1BAD}", &lookup), "${1BAD}");
        assert_eq!(expand_str("${UNCLOSED", &lookup), "${UNCLOSED");
        assert_eq!(expand_str("a${V}b${V}", &lookup), "aokbok");
        assert_eq!(expand_str("$V ${V}", &lookup), "$V ok"); // bare $V not expanded
    }

    fn cfg_for_selection() -> Config {
        parse_config(
            r#"{
                "servers": {
                    "chroma":   { "command": "chroma" },
                    "pi-thing": { "command": "pi" },
                    "watchman": { "lazy": true }
                },
                "profiles": {
                    "opencode": ["chroma"],
                    "pi":       ["pi-thing"]
                }
            }"#,
            &no_env,
        )
        .unwrap()
    }

    #[test]
    fn selects_profile_members_plus_universal() {
        // watchman is in no profile => universal, comes up for every profile.
        let sel = select(&cfg_for_selection(), "opencode", true);
        let names: Vec<_> = sel.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["chroma", "watchman"]);
        assert!(sel.warnings.is_empty());

        let sel = select(&cfg_for_selection(), "pi", true);
        let names: Vec<_> = sel.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["pi-thing", "watchman"]);
    }

    #[test]
    fn unknown_profile_yields_universals_and_a_warning() {
        let sel = select(&cfg_for_selection(), "nope", true);
        let names: Vec<_> = sel.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["watchman"]);
        assert_eq!(sel.warnings.len(), 1);
        assert!(sel.warnings[0].contains("unknown profile 'nope'"));
    }

    #[test]
    fn missing_profile_is_silent_when_not_warning() {
        // Plugin path: a missing host profile brings up universals with no warning.
        let sel = select(&cfg_for_selection(), "nope", false);
        let names: Vec<_> = sel.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["watchman"]);
        assert!(sel.warnings.is_empty());
    }

    #[test]
    fn no_profiles_defined_is_pure_backcompat() {
        // With no profiles at all, every server is universal and nothing warns —
        // any host that asks brings up everything, exactly like before.
        let cfg = parse_config(
            r#"{ "servers": { "a": { "command": "a" }, "b": { "command": "b" } } }"#,
            &no_env,
        )
        .unwrap();
        let sel = select(&cfg, "opencode", true);
        let names: Vec<_> = sel.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert!(sel.warnings.is_empty());
    }

    #[test]
    fn dangling_profile_member_warns_even_when_missing_profile_silent() {
        // A dangling member is a real config error and warns regardless of the
        // missing-profile suppression flag.
        let cfg = parse_config(
            r#"{ "servers": { "a": { "command": "a" } }, "profiles": { "p": ["a", "ghost"] } }"#,
            &no_env,
        )
        .unwrap();
        let sel = select(&cfg, "p", false);
        let names: Vec<_> = sel.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a"]);
        assert_eq!(sel.warnings.len(), 1);
        assert!(sel.warnings[0].contains("unknown server 'ghost'"));
    }
}
