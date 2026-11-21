//! Runs bunny's Portage helper (`helper/bunny-portage.py`) and reads its JSON.
//!
//! The helper asks Portage's own library what emerge would do, so bunny
//! doesn't have to read emerge's screen text. If it can't run (no python3,
//! an older or newer Portage it doesn't understand), bunny falls back to
//! reading emerge's output, as before.

use serde::Deserialize;

use crate::apply::Runner;
use crate::emerge::{Action, Merge};

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
    /// Why there's no plan (helper 3 and later). Read one by one, so one
    /// odd entry doesn't spoil the rest.
    #[serde(default)]
    problems: Vec<serde_json::Value>,
}

/// Why Portage couldn't work out a plan, as data from the helper. bunny
/// explains these kinds from here instead of from emerge's text.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Problem {
    /// Only a testing (`~amd64`) or live (`**`) version would do.
    Keywords {
        name: String,
        version: String,
        keyword: String,
        #[serde(default)]
        required_by: Vec<String>,
    },
    /// Every version that would do is masked.
    Masked {
        name: String,
        /// Like `package.mask`, `~amd64 keyword`, `missing keyword`.
        reasons: Vec<String>,
        /// Gentoo's package.mask comment.
        #[serde(default)]
        comment: Option<String>,
        #[serde(default)]
        required_by: Vec<String>,
    },
    /// It has to be built with other USE flags: flag → on (true) or off.
    Use {
        name: String,
        version: String,
        changes: std::collections::BTreeMap<String, bool>,
        #[serde(default)]
        required_by: Vec<String>,
    },
}

impl Problem {
    fn sane(&self) -> bool {
        let flag_ok = |f: &String| {
            !f.is_empty()
                && f.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+_-@.".contains(c))
        };
        match self {
            Problem::Keywords { name, keyword, .. } => {
                crate::config::is_package_name(name)
                    && (keyword == "**"
                        || (keyword.starts_with('~') && flag_ok(&keyword[1..].to_string())))
            }
            Problem::Masked { name, reasons, .. } => {
                crate::config::is_package_name(name) && !reasons.is_empty()
            }
            Problem::Use { name, changes, .. } => {
                crate::config::is_package_name(name)
                    && !changes.is_empty()
                    && changes.keys().all(flag_ok)
            }
        }
    }
}

/// The helper's answer to `plan`.
#[derive(Debug, PartialEq)]
pub enum PlanAnswer {
    /// What emerge would install.
    List(Vec<Merge>),
    /// Portage couldn't work out a plan; these are the problems it found
    /// that bunny can explain from data (maybe none).
    Refused(Vec<Problem>),
    /// No answer (no python3, an old helper, something unexpected): ask
    /// emerge.
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct HelperMerge {
    name: String,
    version: String,
    from: Option<String>,
    action: String,
    binary: bool,
}

/// Parses the helper's answer to `plan`. `None` if it didn't work out a plan.
#[cfg(test)]
pub fn parse_plan(json: &str) -> Option<Vec<Merge>> {
    match parse_plan_answer(json) {
        PlanAnswer::List(list) => Some(list),
        _ => None,
    }
}

/// Parses the helper's answer to `plan`, including why it failed.
pub fn parse_plan_answer(json: &str) -> PlanAnswer {
    let Some(reply) = json
        .lines()
        .last()
        .and_then(|l| serde_json::from_str::<PlanReply>(l).ok())
    else {
        return PlanAnswer::Unavailable;
    };
    if !reply.ok {
        let problems = reply
            .problems
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Problem>(v).ok())
            .filter(Problem::sane)
            .collect();
        return PlanAnswer::Refused(problems);
    }
    let list: Option<Vec<Merge>> = reply
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
            })
        })
        .collect();
    list.map_or(PlanAnswer::Unavailable, PlanAnswer::List)
}

/// What `emerge --update --deep --newuse @world` would install, from Portage
/// itself, or why it can't.
pub fn plan(runner: &mut dyn Runner) -> PlanAnswer {
    match runner.output(&command(&["plan"])) {
        Ok(Some(out)) => parse_plan_answer(&out),
        _ => PlanAnswer::Unavailable,
    }
}

/// A package `emerge --depclean` would remove.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Removal {
    /// `category/name`.
    pub name: String,
    pub version: String,
    /// Other packages in the same removal whose dependencies name this one:
    /// it goes because they go. Empty when Portage didn't say (or from
    /// emerge's text, which doesn't say).
    #[serde(default)]
    pub needed_by: Vec<String>,
}

impl Removal {
    /// `cat/pkg-1.0`, as emerge writes it (and accepts it after `=`).
    pub fn cpv(&self) -> String {
        format!("{}-{}", self.name, self.version)
    }
}

#[derive(Debug, Deserialize)]
struct DepcleanReply {
    ok: bool,
    removals: Option<Vec<Removal>>,
}

/// Parses the helper's answer to `depclean`. `None` if it didn't say.
pub fn parse_depclean(json: &str) -> Option<Vec<Removal>> {
    let reply: DepcleanReply = serde_json::from_str(json.lines().last()?).ok()?;
    if !reply.ok {
        return None;
    }
    let removals = reply.removals?;
    // Anything that doesn't look like a package: don't trust the rest either.
    let sane = removals.iter().all(|r| {
        crate::config::is_package_name(&r.name)
            && r.version.starts_with(|c: char| c.is_ascii_digit())
            && !r.version.contains(char::is_whitespace)
    });
    sane.then_some(removals)
}

/// What `emerge --depclean` would remove, from Portage itself. `None` means
/// "ask emerge instead".
pub fn depclean(runner: &mut dyn Runner) -> Option<Vec<Removal>> {
    let out = runner.output(&command(&["depclean"])).ok()??;
    parse_depclean(&out)
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
    fn reads_what_depclean_would_remove() {
        let json = r#"{"ok": true, "removals": [
            {"name": "app-text/tree", "version": "2.2.1", "slot": "0", "needed_by": []},
            {"name": "acct-user/nullmail", "version": "0-r2", "slot": "0", "needed_by": ["mail-mta/nullmailer"]},
            {"name": "mail-mta/nullmailer", "version": "2.2-r2", "slot": "0", "needed_by": []},
            {"name": "dev-lang/python", "version": "3.12.12_p1", "slot": "3.12", "needed_by": []}
        ], "helper": 2}"#
            .replace('\n', " ");
        let list = parse_depclean(&json).unwrap();
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].cpv(), "app-text/tree-2.2.1");
        assert_eq!(list[1].needed_by, ["mail-mta/nullmailer"]);
        assert_eq!(list[3].cpv(), "dev-lang/python-3.12.12_p1");

        // Nothing to remove is an answer too.
        let none = parse_depclean(r#"{"ok": true, "removals": [], "helper": 2}"#).unwrap();
        assert!(none.is_empty());
        // `needed_by` is optional.
        let plain = parse_depclean(
            r#"{"ok": true, "removals": [{"name": "a/b", "version": "1"}], "helper": 2}"#,
        )
        .unwrap();
        assert!(plain[0].needed_by.is_empty());
    }

    #[test]
    fn depclean_falls_back_when_the_helper_cant_help() {
        for bad in [
            r#"{"ok": false, "reason": "portage couldn't work out what to remove", "helper": 2}"#,
            // An older helper that doesn't know the command.
            r#"{"ok": false, "reason": "unknown command: depclean", "helper": 1}"#,
            // ok, but no list: don't take that as "nothing to remove".
            r#"{"ok": true, "helper": 2}"#,
            // Something that isn't a package.
            r#"{"ok": true, "removals": [{"name": "tree", "version": "2.2.1"}]}"#,
            r#"{"ok": true, "removals": [{"name": "a/b", "version": "1 --ask"}]}"#,
            "Traceback (most recent call last):",
            "",
        ] {
            assert!(parse_depclean(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn reads_why_there_is_no_plan() {
        let json = r#"{"ok": false, "reason": "portage couldn't work out a plan", "problems": [
            {"kind": "keywords", "name": "dev-qt/qtbase", "version": "6.10.1", "keyword": "~amd64", "required_by": ["gui-apps/quickshell"]},
            {"kind": "masked", "name": "www-client/oldbrowser", "atom": "www-client/oldbrowser", "reasons": ["package.mask"], "comment": "Unmaintained.", "required_by": []},
            {"kind": "use", "name": "net-libs/gnutls", "version": "3.8.13", "changes": {"pkcs11": true, "tools": true}, "required_by": ["app-crypt/swtpm"]},
            {"kind": "slot-conflict", "name": "a/b"}
        ], "helper": 3}"#
            .replace('\n', " ");
        let PlanAnswer::Refused(problems) = parse_plan_answer(&json) else {
            panic!("not a refusal");
        };
        // The kind this bunny doesn't know yet is left to emerge's text.
        assert_eq!(problems.len(), 3);
        assert_eq!(
            problems[0],
            Problem::Keywords {
                name: "dev-qt/qtbase".into(),
                version: "6.10.1".into(),
                keyword: "~amd64".into(),
                required_by: vec!["gui-apps/quickshell".into()],
            }
        );
        assert!(
            matches!(&problems[1], Problem::Masked { comment: Some(c), .. } if c == "Unmaintained.")
        );
        let Problem::Use { changes, .. } = &problems[2] else {
            panic!()
        };
        assert_eq!(changes.get("pkcs11"), Some(&true));
    }

    #[test]
    fn odd_problems_are_left_out_not_trusted() {
        for bad in [
            r#"{"kind": "keywords", "name": "qtbase", "version": "1", "keyword": "~amd64"}"#,
            r#"{"kind": "keywords", "name": "a/b", "version": "1", "keyword": "amd64; rm -rf /"}"#,
            r#"{"kind": "masked", "name": "a/b", "reasons": []}"#,
            r#"{"kind": "use", "name": "a/b", "version": "1", "changes": {}}"#,
            r#"{"kind": "use", "name": "a/b", "version": "1", "changes": {"x\"]\nfoo": true}}"#,
            r#"{"kind": "use", "name": "a/b"}"#,
        ] {
            let json = format!(r#"{{"ok": false, "problems": [{bad}], "helper": 3}}"#);
            assert_eq!(
                parse_plan_answer(&json),
                PlanAnswer::Refused(vec![]),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_older_helper_or_no_helper() {
        // Helper 2: no problems listed. Still a refusal; emerge explains.
        assert_eq!(
            parse_plan_answer(
                r#"{"ok": false, "reason": "portage couldn't work out a plan", "helper": 2}"#
            ),
            PlanAnswer::Refused(vec![])
        );
        assert_eq!(parse_plan_answer(""), PlanAnswer::Unavailable);
        assert_eq!(
            parse_plan_answer("Traceback (most recent call last):"),
            PlanAnswer::Unavailable
        );
    }

    /// `problems()` against a stand-in for Portage's depgraph, shaped like
    /// Portage 3.0's (the real one is only checked on Gentoo). Checks the
    /// script's own logic: the keyword it picks, the chain, the USE map, and
    /// that a dependency with an unmasked version isn't called masked.
    #[test]
    fn problems_from_a_stand_in_depgraph() {
        const STAND_IN: &str = r##"
import sys, types, json
portage = types.ModuleType("portage")
MASKS = {"www-client/oldbrowser-52.0": ["package.mask"], "a/half-1": ["package.mask"], "a/half-2": []}
portage.getmaskingstatus = lambda cpv, settings=None, portdb=None: MASKS[cpv]
portage.getmaskingreason = lambda cpv, settings=None, portdb=None: "# Jane Dev (2026-10-01)\n# Unmaintained.\n# Removal soon.\n"
sys.modules["portage"] = portage
class Pkg:
    def __init__(self, cp, version, repo="gentoo"):
        self.cp, self.version, self.repo = cp, version, repo
        self.cpv = cp + "-" + version
class Arg:  # a command-line argument or set: not a package
    pass
class Atom(str):
    cp = property(lambda self: self.split("[")[0])
    without_use = property(lambda self: Atom(self.split("[")[0]))
qt = Pkg("dev-qt/qtbase", "6.10.1")
live = Pkg("dev-libs/openssl", "9999")
gnutls = Pkg("net-libs/gnutls", "3.8.13")
shell = Pkg("gui-apps/quickshell", "0.2.1", "guru")
wm = Pkg("gui-wm/hyprland", "0.52")
PARENTS = {id(qt): [Arg(), shell], id(shell): [wm], id(wm): [Arg()], id(gnutls): [Pkg("app-crypt/swtpm", "0.10.2")]}
class Digraph:
    def parent_nodes(self, node):
        return PARENTS.get(id(node), [])
class Dyn:
    digraph = Digraph()
    _needed_unstable_keywords = {qt, live}
    _needed_p_mask_changes = set()
    _needed_use_config_changes = {gnutls: (frozenset(["pkcs11", "tools"]), {"tools": True, "pkcs11": True, "static": False})}
    _unsatisfied_deps_for_display = [
        (("/", Atom("www-client/oldbrowser")), {}),
        (("/", Atom("a/half[x]")), {}),
        (("/", Atom("a/missing")), {}),
    ]
class Graph:
    _dynamic_config = Dyn()
class PortDb:
    def aux_get(self, cpv, keys, myrepo=None):
        return [{"dev-qt/qtbase-6.10.1": "~amd64 ~arm64", "dev-libs/openssl-9999": ""}[cpv]]
    def xmatch(self, how, atom):
        return {"www-client/oldbrowser": ["www-client/oldbrowser-52.0"], "a/half": ["a/half-1", "a/half-2"]}.get(str(atom), [])
settings = {"EROOT": "/", "ARCH": "amd64"}
trees = {"/": {"porttree": types.SimpleNamespace(dbapi=PortDb())}}
print(json.dumps(problems(Graph(), settings, trees)))
"##;
        // The script's own definitions, without running its command line.
        let script = SCRIPT.replace("if __name__ == \"__main__\":", "if False:");
        let code = format!("{script}\n{STAND_IN}");
        let Ok(out) = std::process::Command::new("python3")
            .args(["-c", &code])
            .output()
        else {
            eprintln!("python3 not available; skipped");
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{text}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json = format!(r#"{{"ok": false, "problems": {}}}"#, text.trim());
        let PlanAnswer::Refused(found) = parse_plan_answer(&json) else {
            panic!("{text}");
        };
        assert_eq!(
            found[0],
            Problem::Keywords {
                name: "dev-libs/openssl".into(),
                version: "9999".into(),
                keyword: "**".into(),
                required_by: vec![],
            }
        );
        assert_eq!(
            found[1],
            Problem::Keywords {
                name: "dev-qt/qtbase".into(),
                version: "6.10.1".into(),
                keyword: "~amd64".into(),
                required_by: vec!["gui-apps/quickshell".into(), "gui-wm/hyprland".into()],
            }
        );
        let Problem::Use {
            changes,
            required_by,
            ..
        } = &found[2]
        else {
            panic!("{found:#?}")
        };
        assert_eq!(changes.get("static"), Some(&false));
        assert_eq!(required_by, &["app-crypt/swtpm"]);
        assert_eq!(
            found[3],
            Problem::Masked {
                name: "www-client/oldbrowser".into(),
                reasons: vec!["package.mask".into()],
                comment: Some("Unmaintained. Removal soon.".into()),
                required_by: vec![],
            }
        );
        // a/half has an unmasked version; a/missing has none at all: both
        // are left to emerge's text.
        assert_eq!(found.len(), 4, "{found:#?}");
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

        let Ok(out) = std::process::Command::new("python3")
            .args(["-c", SCRIPT, "depclean"])
            .output()
        else {
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("\"ok\": false"), "{text}");
        assert!(parse_depclean(&text).is_none());
    }
}
