//! Config *edit* verbs — the self-define side. Where `config.rs` reads a typed,
//! lossy view for selection, this module mutates the single `servers.json` as
//! lossless `serde_json::Value` under an exclusive flock ("edits are JSON ops"),
//! so a rewrite never drops keys it does not model.
//!
//! Rules (per the design):
//! - Each server entry is stamped with an owning `_scope`. Registering a name
//!   already owned by a *different* scope is a HARD ERROR (clash detected at
//!   write time); the same scope overwrites its own entry idempotently.
//! - Profile membership is a UNION — many callers may add the same server to the
//!   same profile; that never clashes.
//! - `register --if-absent` is the atomic "define it only if nobody has" used by
//!   the lookup → register-if-absent → up lifecycle.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::lockfile::with_lock;

/// What a `register` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// The server was newly added.
    Registered,
    /// The server existed under this scope and was replaced.
    Overwritten,
    /// `--if-absent` and the name already existed; the definition was left as-is
    /// (profile membership is still applied — "defer to the existing def, but
    /// ensure it's in my profile").
    Skipped,
}

/// Read the config JSON (or an empty object for a new/empty file), hand it to
/// `op` to mutate, and write it back pretty-printed — all under one exclusive
/// lock. Reuses the same `flock`-per-file discipline as the runtime lockfiles.
fn edit<F, R>(path: &Path, op: F) -> Result<R>
where
    F: FnOnce(&mut Value) -> Result<R>,
{
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create config directory: {}", parent.display())
            })?;
        }
    }
    with_lock(path, |file| {
        file.seek(SeekFrom::Start(0))?;
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let mut root: Value = if text.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&text).context("config is not valid JSON")?
        };
        if !root.is_object() {
            bail!("config root is not a JSON object");
        }
        let result = op(&mut root)?;
        let out = serde_json::to_string_pretty(&root)?;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(out.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(result)
    })
}

/// Get-or-create the object at `root[key]`, erroring if it exists but isn't an object.
fn obj_field<'a>(root: &'a mut Value, key: &str) -> Result<&'a mut Map<String, Value>> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root is not an object"))?;
    obj.entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("config field '{key}' is not an object"))
}

/// Union `name` into each of `profiles` (creating profiles as needed).
fn union_into_profiles(root: &mut Value, name: &str, profiles: &[String]) -> Result<()> {
    if profiles.is_empty() {
        return Ok(());
    }
    let profs = obj_field(root, "profiles")?;
    for p in profiles {
        let arr = profs
            .entry(p.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        let list = arr
            .as_array_mut()
            .ok_or_else(|| anyhow!("profile '{p}' is not an array"))?;
        if !list.iter().any(|v| v.as_str() == Some(name)) {
            list.push(Value::String(name.to_string()));
        }
    }
    Ok(())
}

/// Remove `names` from every profile; drop any profile left empty.
fn cascade_remove_from_profiles(root: &mut Value, names: &[String]) -> Result<()> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root is not an object"))?;
    let Some(profs) = obj.get_mut("profiles").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    for list in profs.values_mut() {
        if let Some(arr) = list.as_array_mut() {
            arr.retain(|v| !names.iter().any(|n| v.as_str() == Some(n.as_str())));
        }
    }
    profs.retain(|_, list| list.as_array().map(|a| !a.is_empty()).unwrap_or(true));
    Ok(())
}

/// The owning scope of an existing server entry, if any (`None` = hand-authored).
fn scope_of(entry: &Value) -> Option<&str> {
    entry.get("_scope").and_then(|v| v.as_str())
}

/// Register (or overwrite) a server owned by `scope`, and union it into `profiles`.
/// `spec` is the server body (command/args/env/…); `_scope` is stamped on write.
pub fn register(
    path: &Path,
    scope: &str,
    name: &str,
    spec: Value,
    profiles: &[String],
    if_absent: bool,
) -> Result<RegisterOutcome> {
    edit(path, |root| {
        let existing_scope = {
            let servers = obj_field(root, "servers")?;
            servers.get(name).map(|e| scope_of(e).map(String::from))
        };

        if let Some(owner) = existing_scope {
            // The name already exists.
            if if_absent {
                // Defer to whoever defined it, but still ensure our profiles.
                union_into_profiles(root, name, profiles)?;
                return Ok(RegisterOutcome::Skipped);
            }
            match owner.as_deref() {
                Some(s) if s == scope => { /* same owner: overwrite below */ }
                Some(s) => bail!(
                    "server '{name}' is registered by scope '{s}', not '{scope}'; \
                     refusing to overwrite (use --if-absent, a different name, or --scope {s})"
                ),
                None => bail!(
                    "server '{name}' already exists without a scope (hand-authored); \
                     refusing to overwrite. Rename or remove it first, or use --if-absent."
                ),
            }
        }

        let mut body = match spec {
            Value::Object(m) => m,
            _ => bail!("server spec must be a JSON object"),
        };
        body.insert("_scope".to_string(), Value::String(scope.to_string()));

        let servers = obj_field(root, "servers")?;
        let replaced = servers
            .insert(name.to_string(), Value::Object(body))
            .is_some();
        union_into_profiles(root, name, profiles)?;
        Ok(if replaced {
            RegisterOutcome::Overwritten
        } else {
            RegisterOutcome::Registered
        })
    })
}

/// Unregister a `scope`'s entries: a specific `name`, or all of them when `None`.
/// Removing a name owned by a different scope is an error. Returns removed names.
pub fn unregister(path: &Path, scope: &str, name: Option<&str>) -> Result<Vec<String>> {
    edit(path, |root| {
        let removed: Vec<String> = {
            let servers = obj_field(root, "servers")?;

            if let Some(nm) = name {
                match servers.get(nm).map(scope_of) {
                    None => bail!("server '{nm}' is not registered"),
                    Some(Some(s)) if s != scope => bail!(
                        "server '{nm}' is owned by scope '{s}', not '{scope}'; refusing to unregister"
                    ),
                    Some(None) => bail!(
                        "server '{nm}' is hand-authored (no scope); refusing to unregister via --scope {scope}"
                    ),
                    Some(Some(_)) => {} // owned by us
                }
            }

            let to_remove: Vec<String> = servers
                .iter()
                .filter(|(n, v)| {
                    scope_of(v) == Some(scope) && name.is_none_or(|nm| nm == n.as_str())
                })
                .map(|(n, _)| n.clone())
                .collect();
            for n in &to_remove {
                servers.remove(n);
            }
            to_remove
        };
        cascade_remove_from_profiles(root, &removed)?;
        Ok(removed)
    })
}

/// Add `names` to `profile` (union). Servers need not exist yet — a dangling
/// member is a warning at selection time, and this lets a plugin tag a shared
/// server it did not itself define.
pub fn profile_add(path: &Path, profile: &str, names: &[String]) -> Result<()> {
    edit(path, |root| {
        for n in names {
            union_into_profiles(root, n, std::slice::from_ref(&profile.to_string()))?;
        }
        Ok(())
    })
}

/// Remove `names` from `profile`; drop the profile if it becomes empty.
pub fn profile_remove(path: &Path, profile: &str, names: &[String]) -> Result<()> {
    edit(path, |root| {
        let profs = obj_field(root, "profiles")?;
        if let Some(list) = profs.get_mut(profile).and_then(|v| v.as_array_mut()) {
            list.retain(|v| !names.iter().any(|n| v.as_str() == Some(n.as_str())));
        }
        profs.retain(|_, list| list.as_array().map(|a| !a.is_empty()).unwrap_or(true));
        Ok(())
    })
}

/// A read of one server: its spec plus the profiles that name it.
#[derive(Debug, Clone, PartialEq)]
pub struct LookupHit {
    pub name: String,
    pub spec: Value,
    pub profiles: Vec<String>,
}

/// Read the config file's JSON, or `Value::Null` if it doesn't exist/parse-fails
/// as empty. Read-only, no lock (writers are atomic).
fn read_root(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(Value::Object(Map::new())),
        Ok(t) => serde_json::from_str(&t).context("config is not valid JSON"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

fn profiles_containing(root: &Value, name: &str) -> Vec<String> {
    let Some(profs) = root.get("profiles").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = profs
        .iter()
        .filter(|(_, list)| {
            list.as_array()
                .map(|a| a.iter().any(|v| v.as_str() == Some(name)))
                .unwrap_or(false)
        })
        .map(|(p, _)| p.clone())
        .collect();
    out.sort();
    out
}

/// Point query: the server `name`'s spec + the profiles it belongs to, or `None`.
pub fn lookup(path: &Path, name: &str) -> Result<Option<LookupHit>> {
    let root = read_root(path)?;
    let Some(spec) = root.get("servers").and_then(|s| s.get(name)) else {
        return Ok(None);
    };
    Ok(Some(LookupHit {
        name: name.to_string(),
        spec: spec.clone(),
        profiles: profiles_containing(&root, name),
    }))
}

/// The whole config document (as written on disk), for `config show`.
pub fn read_config_value(path: &Path) -> Result<Value> {
    read_root(path)
}

/// A structural issue found by `validate`.
pub fn validate(path: &Path) -> Result<Vec<String>> {
    let root = read_root(path)?;
    let mut issues = Vec::new();

    let servers = root.get("servers").and_then(|v| v.as_object());
    if root.get("servers").is_some() && servers.is_none() {
        issues.push("`servers` is present but is not an object".to_string());
    }
    let profiles = root.get("profiles").and_then(|v| v.as_object());
    if root.get("profiles").is_some() && profiles.is_none() {
        issues.push("`profiles` is present but is not an object".to_string());
    }

    if let (Some(servers), Some(profiles)) = (servers, profiles) {
        for (p, list) in profiles {
            let Some(arr) = list.as_array() else {
                issues.push(format!("profile '{p}' is not an array"));
                continue;
            };
            for v in arr {
                if let Some(n) = v.as_str() {
                    if !servers.contains_key(n) {
                        issues.push(format!("profile '{p}' lists unknown server '{n}'"));
                    }
                } else {
                    issues.push(format!("profile '{p}' has a non-string member"));
                }
            }
        }
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // A unique temp config path per test; cleaned up at the end of each test.
    struct Tmp(std::path::PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("ss-cfgedit-{tag}-{}.json", std::process::id()));
            let _ = std::fs::remove_file(&p);
            Tmp(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn read(&self) -> Value {
            read_root(&self.0).unwrap()
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn spec(cmd: &str) -> Value {
        json!({ "command": cmd })
    }

    #[test]
    fn register_stamps_scope_and_unions_profile() {
        let t = Tmp::new("reg");
        let out = register(
            t.path(),
            "pluginA",
            "chroma",
            spec("chroma"),
            &["opencode".into()],
            false,
        )
        .unwrap();
        assert_eq!(out, RegisterOutcome::Registered);
        let root = t.read();
        assert_eq!(root["servers"]["chroma"]["command"], json!("chroma"));
        assert_eq!(root["servers"]["chroma"]["_scope"], json!("pluginA"));
        assert_eq!(root["profiles"]["opencode"], json!(["chroma"]));
    }

    #[test]
    fn same_scope_overwrites_other_scope_clashes() {
        let t = Tmp::new("clash");
        register(t.path(), "A", "chroma", spec("chroma"), &[], false).unwrap();
        // same scope: overwrite ok
        let out = register(t.path(), "A", "chroma", spec("chroma2"), &[], false).unwrap();
        assert_eq!(out, RegisterOutcome::Overwritten);
        assert_eq!(t.read()["servers"]["chroma"]["command"], json!("chroma2"));
        // different scope: hard error
        let err = register(t.path(), "B", "chroma", spec("x"), &[], false).unwrap_err();
        assert!(err.to_string().contains("registered by scope 'A'"));
    }

    #[test]
    fn if_absent_skips_but_still_unions_profile() {
        let t = Tmp::new("ifabsent");
        register(t.path(), "A", "chroma", spec("chroma"), &[], false).unwrap();
        let out = register(t.path(), "B", "chroma", spec("other"), &["pi".into()], true).unwrap();
        assert_eq!(out, RegisterOutcome::Skipped);
        let root = t.read();
        // definition unchanged (A's), but pi profile now includes it
        assert_eq!(root["servers"]["chroma"]["command"], json!("chroma"));
        assert_eq!(root["servers"]["chroma"]["_scope"], json!("A"));
        assert_eq!(root["profiles"]["pi"], json!(["chroma"]));
    }

    #[test]
    fn unregister_removes_and_cascades_out_of_profiles() {
        let t = Tmp::new("unreg");
        register(
            t.path(),
            "A",
            "chroma",
            spec("chroma"),
            &["opencode".into()],
            false,
        )
        .unwrap();
        register(
            t.path(),
            "A",
            "watchman",
            spec("watchman"),
            &["opencode".into()],
            false,
        )
        .unwrap();
        let removed = unregister(t.path(), "A", Some("chroma")).unwrap();
        assert_eq!(removed, vec!["chroma"]);
        let root = t.read();
        assert!(root["servers"].get("chroma").is_none());
        assert_eq!(root["profiles"]["opencode"], json!(["watchman"]));
        // remove the rest by scope; empty profile is dropped
        let removed = unregister(t.path(), "A", None).unwrap();
        assert_eq!(removed, vec!["watchman"]);
        assert!(t.read()["profiles"].get("opencode").is_none());
    }

    #[test]
    fn unregister_foreign_scope_errors() {
        let t = Tmp::new("unregforeign");
        register(t.path(), "A", "chroma", spec("chroma"), &[], false).unwrap();
        let err = unregister(t.path(), "B", Some("chroma")).unwrap_err();
        assert!(err.to_string().contains("owned by scope 'A'"));
    }

    #[test]
    fn profile_add_and_remove_union_and_drop() {
        let t = Tmp::new("profadd");
        register(t.path(), "A", "chroma", spec("chroma"), &[], false).unwrap();
        profile_add(t.path(), "web", &["chroma".into(), "chroma".into()]).unwrap(); // idempotent union
        assert_eq!(t.read()["profiles"]["web"], json!(["chroma"]));
        profile_remove(t.path(), "web", &["chroma".into()]).unwrap();
        assert!(t.read()["profiles"].get("web").is_none()); // empty profile dropped
    }

    #[test]
    fn lookup_reports_spec_and_profiles() {
        let t = Tmp::new("lookup");
        register(
            t.path(),
            "A",
            "chroma",
            spec("chroma"),
            &["opencode".into(), "web".into()],
            false,
        )
        .unwrap();
        let hit = lookup(t.path(), "chroma").unwrap().unwrap();
        assert_eq!(hit.name, "chroma");
        assert_eq!(
            hit.profiles,
            vec!["opencode".to_string(), "web".to_string()]
        );
        assert!(lookup(t.path(), "nope").unwrap().is_none());
    }

    #[test]
    fn validate_flags_dangling_members() {
        let t = Tmp::new("validate");
        register(t.path(), "A", "chroma", spec("chroma"), &[], false).unwrap();
        profile_add(t.path(), "web", &["chroma".into(), "ghost".into()]).unwrap();
        let issues = validate(t.path()).unwrap();
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("unknown server 'ghost'"));
    }

    #[test]
    fn preserves_unknown_keys_on_edit() {
        let t = Tmp::new("lossless");
        std::fs::write(
            t.path(),
            r#"{ "servers": { "a": { "command": "a", "futureField": 42 } }, "topLevelExtra": true }"#,
        )
        .unwrap();
        register(t.path(), "A", "b", spec("b"), &[], false).unwrap();
        let root = t.read();
        // unknown keys survived the rewrite
        assert_eq!(root["servers"]["a"]["futureField"], json!(42));
        assert_eq!(root["topLevelExtra"], json!(true));
        assert_eq!(root["servers"]["b"]["command"], json!("b"));
    }
}
