//! Runs bunny's Portage helper (`helper/bunny-portage.py`) and reads its JSON.
//!
//! The helper asks Portage's own library what emerge would do, so bunny
//! doesn't have to read emerge's screen text. If it can't run (no python3,
//! an older or newer Portage it doesn't understand), bunny falls back to
//! reading emerge's output, as before.

use serde::Deserialize;

use crate::apply::Runner;
use crate::emerge::{Action, Merge, Why};

pub const SCRIPT: &str = include_str!("helper/bunny-portage.py");

/// The command that runs the helper with these arguments.
pub fn command(args: &[&str]) -> Vec<String> {
    let mut cmd = vec!["python3".to_string(), "-c".to_string(), SCRIPT.to_string()];
    cmd.extend(args.iter().map(|a| a.to_string()));
    cmd
}

#[derive(Debug, Deserialize)]
struct PlanReply {
    ok: bool,
    #[serde(default)]
    merges: Vec<HelperMerge>,
}

#[derive(Debug, Deserialize)]
struct HelperMerge {
    name: String,
    version: String,
    from: Option<String>,
    action: String,
    binary: bool,
    /// Why a rebuild happens (helper version 2 and later).
    #[serde(default)]
    why: HelperWhy,
}

#[derive(Debug, Default, Deserialize)]
struct HelperWhy {
    #[serde(default, rename = "use")]
    use_flags: Vec<String>,
    #[serde(default)]
    libraries: Vec<String>,
}

/// Parses the helper's answer to `plan`. `None` if it didn't work out a plan.
pub fn parse_plan(json: &str) -> Option<Vec<Merge>> {
    let reply: PlanReply = serde_json::from_str(json.lines().last()?).ok()?;
    if !reply.ok {
        return None;
    }
    reply
        .merges
        .into_iter()
        .map(|m| {
            Some(Merge {
                name: m.name,
                version: m.version,
                from: m.from,
                action: match m.action.as_str() {
                    "new" => Action::New,
                    "update" => Action::Update,
                    "downgrade" => Action::Downgrade,
                    "rebuild" => Action::Rebuild,
                    _ => return None,
                },
                binary: m.binary,
                why: Why {
                    use_flags: m.why.use_flags,
                    library_changed: !m.why.libraries.is_empty(),
                    libraries: m.why.libraries,
                },
            })
        })
        .collect()
}

/// What `emerge --update --deep --newuse @world` would install, from Portage
/// itself. `None` means "ask emerge instead".
pub fn plan(runner: &mut dyn Runner) -> Option<Vec<Merge>> {
    let out = runner.output(&command(&["plan"])).ok()??;
    parse_plan(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plan() {
        let json = r#"{"ok": true, "merges": [
            {"name": "app-editors/vim-core", "version": "9.2.1202", "from": null, "action": "new", "binary": true, "slot": "0"},
            {"name": "app-admin/sudo", "version": "1.9.17_p2", "from": "1.9.17_p2", "action": "rebuild", "binary": false, "slot": "0"},
            {"name": "sys-libs/glibc", "version": "2.43", "from": "2.42-r1", "action": "update", "binary": true, "slot": "2.2"}
        ], "helper": 1}"#
            .replace('\n', " ");
        let list = parse_plan(&json).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "app-editors/vim-core");
        assert_eq!(list[0].action, Action::New);
        assert!(list[0].binary);
        assert_eq!(list[1].action, Action::Rebuild);
        assert!(!list[1].binary);
        assert_eq!(list[2].from.as_deref(), Some("2.42-r1"));
        // Helper version 1 doesn't say why; that's fine.
        assert_eq!(list[1].why, Why::default());
    }

    #[test]
    fn reads_why_a_package_is_rebuilt() {
        let json = r#"{"ok": true, "merges": [
            {"name": "net-misc/curl", "version": "8.16.0", "from": "8.16.0", "action": "rebuild", "binary": true, "slot": "0",
             "why": {"use": ["+adns"], "libraries": []}},
            {"name": "sys-devel/gettext", "version": "0.23.1", "from": "0.23.1", "action": "rebuild", "binary": false, "slot": "0",
             "why": {"use": [], "libraries": ["dev-libs/libunistring"]}}
        ], "helper": 2}"#
            .replace('\n', " ");
        let list = parse_plan(&json).unwrap();
        assert_eq!(list[0].why.short(), "USE +adns");
        assert_eq!(list[1].why.short(), "libunistring changed");
    }

    #[test]
    fn falls_back_when_the_helper_cant_help() {
        assert!(parse_plan(
            r#"{"ok": false, "reason": "portage couldn't work out a plan", "helper": 1}"#
        )
        .is_none());
        assert!(parse_plan("Traceback (most recent call last):").is_none());
        assert!(parse_plan("").is_none());
        // An action this bunny doesn't know: don't guess.
        assert!(parse_plan(r#"{"ok": true, "merges": [{"name": "a/b", "version": "1", "from": null, "action": "teleport", "binary": true}]}"#).is_none());
    }

    #[test]
    fn the_script_is_valid_python() {
        let ok = std::process::Command::new("python3")
            .args(["-c", "import ast, sys; ast.parse(sys.stdin.read())"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(SCRIPT.as_bytes())?;
                child.wait()
            });
        match ok {
            Ok(status) => assert!(status.success(), "bunny-portage.py has a syntax error"),
            Err(_) => eprintln!("python3 not available; skipped"),
        }
    }

    #[test]
    fn says_so_without_portage() {
        // Outside Gentoo the helper must still answer in JSON, not crash.
        let Ok(out) = std::process::Command::new("python3")
            .args(["-c", SCRIPT, "plan"])
            .output()
        else {
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        if std::path::Path::new("/usr/lib/portage").exists() {
            return; // a real Gentoo: the canary covers it
        }
        assert!(text.contains("\"ok\": false"), "{text}");
        assert!(parse_plan(&text).is_none());
    }
}
