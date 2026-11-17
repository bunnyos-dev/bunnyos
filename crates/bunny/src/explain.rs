//! `bunny explain`: turn emerge's messages into plain English with a fix.
//!
//! Each recogniser looks for one kind of message. Unknown messages produce
//! nothing, and the caller says so honestly instead of guessing.

use std::fmt::Write as _;

use crate::import::strip_version;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    /// emerge stopped because of this.
    Problem,
    /// Something risky emerge suggested, or something to watch out for.
    Warning,
    /// Good to know, nothing is wrong.
    Info,
}

#[derive(Debug, PartialEq)]
pub struct Explanation {
    pub severity: Severity,
    /// One sentence: what happened.
    pub title: String,
    /// Extra lines: why, or what's involved.
    pub details: Vec<String>,
    /// What to do about it.
    pub fix: Option<String>,
}

impl Explanation {
    fn new(severity: Severity, title: impl Into<String>) -> Self {
        Self {
            severity,
            title: title.into(),
            details: Vec::new(),
            fix: None,
        }
    }

    fn detail(mut self, line: impl Into<String>) -> Self {
        self.details.push(line.into());
        self
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn render(&self) -> String {
        let icon = match self.severity {
            Severity::Problem => "✗",
            Severity::Warning => "⚠",
            Severity::Info => "ℹ",
        };
        let mut out = format!("{icon} {}\n", self.title);
        for line in &self.details {
            let _ = writeln!(out, "  {line}");
        }
        if let Some(fix) = &self.fix {
            // Nothing is wrong with an info item, so it gets a tip, not a fix.
            let label = match self.severity {
                Severity::Info => "Tip:",
                _ => "Fix:",
            };
            let mut lines = fix.lines();
            if let Some(first) = lines.next() {
                let _ = writeln!(out, "  {label} {first}");
            }
            for line in lines {
                let _ = writeln!(out, "       {line}");
            }
        }
        out
    }
}

/// Explains everything bunny recognises in `output`, most important first.
pub fn explain(output: &str) -> Vec<Explanation> {
    let lines: Vec<&str> = output.lines().map(str::trim_end).collect();
    let mut found = Vec::new();
    found.extend(use_changes(&lines));
    found.extend(keyword_changes(&lines));
    found.extend(masked(&lines));
    found.extend(no_such_package(&lines));
    found.extend(skipped_world_entries(&lines));
    found.extend(slot_conflicts(&lines));
    found.extend(blockers(&lines));
    found.extend(fetch_failures(&lines));
    found.extend(config_updates(&lines));
    found.extend(notices(&lines));
    found.sort_by_key(|e| match e.severity {
        Severity::Problem => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    });
    found
}

/// Removes terminal color codes (`ESC [ ... letter`) from captured output.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Parameters, then one final letter.
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// `>=net-libs/gnutls-3.8.13:0/30::gentoo[ssl]` → `net-libs/gnutls`
pub fn base_name(atom: &str) -> String {
    let atom = atom.trim().trim_matches('"');
    let atom = atom.trim_start_matches(['<', '>', '=', '~', '!']);
    let atom = atom.split("::").next().unwrap_or(atom);
    let atom = atom.split('[').next().unwrap_or(atom);
    let atom = atom.split(':').next().unwrap_or(atom);
    let atom = atom.trim_end_matches('*');
    strip_version(atom).unwrap_or(atom).to_string()
}

/// `net-libs/gnutls` → `gnutls`
fn short(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// The bunny.toml snippet that sets these USE flags.
fn use_snippet(package: &str, flags: &[String]) -> String {
    let quoted: Vec<String> = flags.iter().map(|f| format!("\"{f}\"")).collect();
    format!(
        "add this to your bunny.toml, then run `bunny switch` again:\n\
         [pkg.\"{package}\"]\n\
         use = [{}]",
        quoted.join(", ")
    )
}

fn use_change(package: &str, flags: Vec<String>, needed_by: Option<String>) -> Explanation {
    let on: Vec<&str> = flags
        .iter()
        .filter(|f| !f.starts_with('-'))
        .map(String::as_str)
        .collect();
    let off: Vec<&str> = flags.iter().filter_map(|f| f.strip_prefix('-')).collect();
    let who = needed_by
        .map(|n| short(&n).to_string())
        .unwrap_or_else(|| "Something you're installing".to_string());
    let mut what = Vec::new();
    if !on.is_empty() {
        what.push(format!("these features turned on: {}", on.join(", ")));
    }
    if !off.is_empty() {
        what.push(format!("these features turned off: {}", off.join(", ")));
    }
    Explanation::new(
        Severity::Problem,
        format!(
            "{who} needs {} built with {}.",
            short(package),
            what.join(" and ")
        ),
    )
    .detail(format!(
        "This only rebuilds {}; nothing else changes.",
        short(package)
    ))
    .fix(use_snippet(package, &flags))
}

/// Both forms: emerge's "USE changes are necessary" list, and the
/// "(Change USE: +a +b)" line.
fn use_changes(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();

    // Form 1: "- net-libs/gnutls-3.8.13::gentoo (Change USE: +pkcs11 +tools)"
    for (i, line) in lines.iter().enumerate() {
        let Some(start) = line.find("(Change USE:") else {
            continue;
        };
        let atom = line
            .trim_start_matches(['-', ' '])
            .split_whitespace()
            .next();
        let Some(atom) = atom else { continue };
        let inside = line[start + "(Change USE:".len()..].trim_end_matches(')');
        let flags = inside
            .split_whitespace()
            .map(|f| f.trim_start_matches('+').to_string())
            .collect();
        let needed_by = lines[i..]
            .iter()
            .find_map(|l| l.split("(dependency required by \"").nth(1))
            .map(|rest| base_name(rest.split('"').next().unwrap_or(rest)));
        found.push(use_change(&base_name(atom), flags, needed_by));
    }

    // Form 2: the list after "The following USE changes are necessary".
    for block in blocks_after(lines, "The following USE changes are necessary") {
        let mut needed_by = None;
        for line in block {
            if let Some(rest) = line.strip_prefix("# required by ") {
                needed_by.get_or_insert_with(|| {
                    base_name(rest.split_whitespace().next().unwrap_or(rest))
                });
                continue;
            }
            let mut words = line.split_whitespace();
            let Some(atom) = words.next() else { continue };
            if !atom.contains('/') {
                continue;
            }
            let flags = words.map(String::from).collect();
            found.push(use_change(&base_name(atom), flags, needed_by.take()));
        }
    }
    found
}

/// Lines after a heading, up to the next blank line (skipping emerge's
/// "(see ... man page)" note).
fn blocks_after<'a>(lines: &[&'a str], heading: &str) -> Vec<Vec<&'a str>> {
    let mut blocks = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains(heading) {
            continue;
        }
        let block = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .skip_while(|l| l.starts_with("(see "))
            .take_while(|l| !l.is_empty())
            .collect();
        blocks.push(block);
    }
    blocks
}

fn keyword_changes(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();
    let written = lines
        .iter()
        .any(|l| l.contains("Autounmask changes successfully written"));
    for block in blocks_after(lines, "The following keyword changes are necessary") {
        let mut chain: Vec<String> = block
            .iter()
            .filter_map(|l| l.strip_prefix("# required by "))
            .map(|r| r.split_whitespace().next().unwrap_or(r))
            .filter(|r| r.contains('/'))
            .map(|r| short(&base_name(r)).to_string())
            .collect();
        chain.dedup();
        for line in block.iter().filter(|l| !l.starts_with('#')) {
            let mut words = line.split_whitespace();
            let (Some(atom), keyword) = (words.next(), words.next().unwrap_or("")) else {
                continue;
            };
            let package = base_name(atom);
            let live = atom.contains("9999") || keyword == "**";
            let because = if chain.is_empty() {
                String::new()
            } else {
                format!("It's needed because of: {}.", chain.join(" → "))
            };
            if live {
                let mut e = Explanation::new(
                    Severity::Warning,
                    format!(
                        "emerge wants to use the live development version of {} ({line}).",
                        short(&package)
                    ),
                )
                .detail(
                    "That's unreleased code built straight from the developers' git repository,",
                )
                .detail("not a tested release. Don't accept this.");
                if !because.is_empty() {
                    e = e.detail(because);
                }
                let mut fix = String::from(
                    "run the command again with --autounmask=n to see the real problem,\n\
                     and paste that into `bunny explain`.",
                );
                if written {
                    fix.push_str(
                        "\nemerge saved this suggestion in waiting files. Delete them, don't accept them:\n\
                         doas find /etc/portage -name '._cfg*' -exec grep -l '9999' {} + | xargs -r doas rm -v",
                    );
                }
                found.push(e.fix(fix));
            } else {
                let mut e = Explanation::new(
                    Severity::Problem,
                    format!(
                        "{} is only available as a testing version ({keyword}) right now.",
                        short(&package)
                    ),
                )
                .detail("Testing versions work for most people but haven't been declared stable by Gentoo yet.");
                if !because.is_empty() {
                    e = e.detail(because);
                }
                found.push(e.fix(format!(
                    "bunny can't manage testing versions yet. To allow this one, run:\n\
                     echo '{line}' | doas tee /etc/portage/package.accept_keywords/{}",
                    short(&package)
                )));
            }
        }
    }
    found
}

fn masked(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("!!! All ebuilds that could satisfy \"") else {
            continue;
        };
        let package = base_name(rest.split('"').next().unwrap_or(rest));
        let following = &lines[i + 1..];
        let reason = following
            .iter()
            .find_map(|l| l.split("(masked by: ").nth(1))
            .map(|r| r.trim_end_matches(')').to_string())
            .unwrap_or_default();
        // Gentoo's comment explaining the mask: the "# " lines after the
        // package.mask file name, minus the first (author and date).
        let comment: Vec<String> = following
            .iter()
            .skip_while(|l| !l.ends_with("package.mask:"))
            .skip(2)
            .take_while(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .collect();

        let e = if reason.contains("keyword") && reason.contains('~') {
            Explanation::new(
                Severity::Problem,
                format!(
                    "{} is only available as a testing version right now.",
                    short(&package)
                ),
            )
        } else if reason.contains("missing keyword") {
            Explanation::new(
                Severity::Problem,
                format!(
                    "{} isn't available for your kind of computer (CPU architecture).",
                    short(&package)
                ),
            )
        } else {
            let mut e = Explanation::new(
                Severity::Problem,
                format!(
                    "Gentoo has blocked {} on purpose (it's \"masked\").",
                    short(&package)
                ),
            );
            if !comment.is_empty() {
                e = e.detail(format!("Gentoo's reason: {}", comment.join(" ")));
            }
            e.fix(format!(
                "it's usually best not to use it. Remove {package} from your bunny.toml,\n\
                 or look for a replacement."
            ))
        };
        found.push(e);
    }
    found
}

/// "there are no ebuilds to satisfy" (but not the "built with USE flags"
/// variant, which is a USE change): usually a typo or a removed package.
fn no_such_package(lines: &[&str]) -> Vec<Explanation> {
    lines
        .iter()
        .filter_map(|l| l.split("there are no ebuilds to satisfy \"").nth(1))
        .map(|rest| base_name(rest.split('"').next().unwrap_or(rest)))
        .map(|package| {
            Explanation::new(
                Severity::Problem,
                format!("There's no package called {package}."),
            )
            .detail("It may be a typo, or the package was renamed or removed from Gentoo.")
            .fix(format!(
                "check the name at https://packages.gentoo.org and fix it in your bunny.toml\n\
                 (look for \"{}\").",
                short(&package)
            ))
        })
        .collect()
}

/// "Ebuilds for the following packages are either all masked or don't
/// exist": emerge skips them and carries on, so this is easy to miss.
fn skipped_world_entries(lines: &[&str]) -> Vec<Explanation> {
    let Some(i) = lines
        .iter()
        .position(|l| l.contains("masked or don't exist:"))
    else {
        return Vec::new();
    };
    let packages: Vec<String> = lines[i + 1..]
        .iter()
        .map(|l| l.trim())
        .take_while(|l| !l.is_empty() && !l.starts_with("!!!"))
        .flat_map(|l| l.split_whitespace())
        .filter(|w| w.contains('/'))
        .map(String::from)
        .collect();
    if packages.is_empty() {
        return Vec::new();
    }
    vec![Explanation::new(
        Severity::Warning,
        format!(
            "emerge skipped {} because it couldn't find {}.",
            packages.join(", "),
            if packages.len() == 1 { "it" } else { "them" }
        ),
    )
    .detail("Everything else was handled normally.")
    .detail("Usually it's a typo, or the package was renamed, removed or blocked by Gentoo.")
    .fix("check the name at https://packages.gentoo.org and fix it in your bunny.toml.")]
}

fn slot_conflicts(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("Multiple package instances within a single package slot") {
            continue;
        }
        let following = &lines[i + 1..];
        let package = following
            .iter()
            .map(|l| l.trim())
            .find(|l| l.contains('/') && !l.starts_with("!!!") && !l.starts_with('('))
            .map(base_name)
            .unwrap_or_else(|| "a package".into());
        let mut e = Explanation::new(
            Severity::Problem,
            format!(
                "Two packages need different, incompatible versions of {}.",
                short(&package)
            ),
        );
        let mut requesters = Vec::new();
        for l in following.iter().take(40) {
            let Some((wanted, rest)) = l.trim().split_once(" required by (") else {
                continue;
            };
            let who = base_name(rest.split_whitespace().next().unwrap_or(rest));
            let version = wanted
                .trim_start_matches(|c: char| {
                    !c.is_ascii_alphanumeric() && c != '<' && c != '>' && c != '='
                })
                .split(':')
                .next()
                .unwrap_or(wanted);
            e = e.detail(format!("• {} needs {version}", short(&who)));
            requesters.push(short(&who).to_string());
        }
        let fix = match requesters.as_slice() {
            [a, b, ..] => format!(
                "one of them is usually outdated. Update or remove {b} (or {a}),\n\
                 then run `bunny switch` again."
            ),
            _ => "update or remove whichever of them is outdated, then try again.".into(),
        };
        found.push(e.fix(fix));
    }
    found
}

fn blockers(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();
    for line in lines {
        if !line.starts_with("[blocks") {
            continue;
        }
        // [blocks B      ] app-misc/a ("app-misc/a" is soft blocking app-shells/b-4.1.2)
        let Some(inside) = line.split_once('(').map(|(_, r)| r) else {
            continue;
        };
        let words: Vec<&str> = inside.trim_end_matches(')').split_whitespace().collect();
        let (Some(blocker), Some(blocked)) = (words.first(), words.last()) else {
            continue;
        };
        let (blocker, blocked) = (base_name(blocker), base_name(blocked));
        found.push(
            Explanation::new(
                Severity::Problem,
                format!(
                    "{} and {} can't be installed at the same time.",
                    short(&blocked),
                    short(&blocker)
                ),
            )
            .fix(format!(
                "pick one. To keep {}, remove {blocker} from your bunny.toml and run `bunny switch`.",
                short(&blocked)
            )),
        );
    }
    found
}

fn fetch_failures(lines: &[&str]) -> Vec<Explanation> {
    lines
        .iter()
        .filter_map(|l| l.split("Fetch failed for '").nth(1))
        .map(|rest| base_name(rest.split('\'').next().unwrap_or(rest)))
        .map(|package| {
            Explanation::new(
                Severity::Problem,
                format!("Couldn't download the source code for {}.", short(&package)),
            )
            .detail("Either the internet connection dropped or the download server is down.")
            .fix("check your connection and run the command again. If it keeps failing,\nthe server may be down; try again later.")
        })
        .collect()
}

fn config_updates(lines: &[&str]) -> Vec<Explanation> {
    let written = lines
        .iter()
        .any(|l| l.contains("Autounmask changes successfully written"));
    lines
        .iter()
        .filter_map(|l| l.split("IMPORTANT: ").nth(1))
        .filter(|rest| rest.contains("config files in"))
        .map(|rest| {
            let n = rest.split_whitespace().next().unwrap_or("Some");
            if written {
                Explanation::new(
                    Severity::Warning,
                    format!("emerge saved its suggested changes in {n} waiting config files."),
                )
                .detail("They do nothing until you accept them with dispatch-conf or etc-update.")
                .fix("read the warnings above before accepting anything.")
            } else {
                Explanation::new(
                    Severity::Info,
                    format!("{n} config files have new versions waiting to be merged."),
                )
                .fix("run `doas dispatch-conf` to review them.")
            }
        })
        .collect()
}

fn notices(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();
    for line in lines {
        if let Some(rest) = line.split("IMPORTANT: ").nth(1) {
            if rest.contains("news items need reading") {
                let n = rest.split_whitespace().next().unwrap_or("Some");
                found.push(
                    Explanation::new(
                        Severity::Info,
                        format!("Gentoo has {n} announcements you haven't read."),
                    )
                    .detail("They're about changes that might need your attention. Most won't apply to you.")
                    .fix("run `eselect news read` to read them."),
                );
            }
        }
        if line.contains("it is important to remove obsolete packages with") {
            found.push(Explanation::new(
                Severity::Info,
                "emerge reminds you to remove unused packages. bunny does that for you during `bunny switch`.",
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn strips_colors() {
        assert_eq!(
            strip_ansi("\u{1b}[32;01m * \u{1b}[39;49;00mIMPORTANT: 3 news items"),
            " * IMPORTANT: 3 news items"
        );
        let colored =
            fixture("use-change-swtpm.txt").replace("emerge:", "\u{1b}[31;01memerge:\u{1b}[0m");
        assert_eq!(explain(&strip_ansi(&colored)).len(), 1);
    }

    #[test]
    fn base_names() {
        assert_eq!(base_name(">=net-libs/gnutls-3.8.13"), "net-libs/gnutls");
        assert_eq!(
            base_name("net-libs/gnutls-3.8.13::gentoo"),
            "net-libs/gnutls"
        );
        assert_eq!(
            base_name("\"net-libs/gnutls:=[pkcs11,tools]\""),
            "net-libs/gnutls"
        );
        assert_eq!(base_name("=dev-libs/openssl-3.6.9999"), "dev-libs/openssl");
        assert_eq!(base_name("dev-libs/openssl:0"), "dev-libs/openssl");
        assert_eq!(
            base_name("=dev-libs/openssl-1.1*:0/1.1="),
            "dev-libs/openssl"
        );
        assert_eq!(base_name("app-crypt/swtpm"), "app-crypt/swtpm");
    }

    #[test]
    fn use_change_from_the_real_swtpm_error() {
        let found = explain(&fixture("use-change-swtpm.txt"));
        assert_eq!(found.len(), 1, "{found:#?}");
        let e = &found[0];
        assert_eq!(e.severity, Severity::Problem);
        assert_eq!(
            e.title,
            "swtpm needs gnutls built with these features turned on: pkcs11, tools."
        );
        let fix = e.fix.as_deref().unwrap();
        assert!(fix.contains("[pkg.\"net-libs/gnutls\"]"), "{fix}");
        assert!(fix.contains("use = [\"pkcs11\", \"tools\"]"), "{fix}");
    }

    #[test]
    fn use_change_list_form() {
        let found = explain(&fixture("use-change-list.txt"));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].title.starts_with("swtpm needs gnutls"));
    }

    #[test]
    fn live_version_is_flagged_as_risky() {
        let found = explain(&fixture("keyword-live-openssl.txt"));
        let e = &found[0];
        assert_eq!(e.severity, Severity::Warning);
        assert!(
            e.title.contains("live development version of openssl"),
            "{}",
            e.title
        );
        assert!(e.details.iter().any(|d| d.contains("Don't accept")));
        assert!(
            e.details
                .iter()
                .any(|d| d.contains("mit-krb5 → krb5 → cifs-utils")),
            "{:?}",
            e.details
        );
        let fix = e.fix.as_deref().unwrap();
        assert!(fix.contains("--autounmask=n"));
        assert!(fix.contains("._cfg"), "{fix}");
        // And the waiting config files are a warning, not routine.
        assert!(found
            .iter()
            .any(|e| e.severity == Severity::Warning && e.title.contains("waiting config files")));
    }

    #[test]
    fn testing_version() {
        let found = explain(&fixture("keyword-testing.txt"));
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].title,
            "qtbase is only available as a testing version (~amd64) right now."
        );
        assert!(found[0]
            .fix
            .as_deref()
            .unwrap()
            .contains("package.accept_keywords/qtbase"));
    }

    #[test]
    fn masked_package_with_reason() {
        let found = explain(&fixture("masked.txt"));
        assert_eq!(found.len(), 1);
        assert!(found[0].title.contains("blocked oldbrowser on purpose"));
        assert_eq!(
            found[0].details,
            ["Gentoo's reason: Unmaintained, many open security bugs. Removal on 2026-11-30."]
        );
    }

    #[test]
    fn no_such_package() {
        let found = explain(&fixture("no-such-package.txt"));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].title, "There's no package called app-misc/tmuxx.");
        // The USE-flag variant of the message is not mistaken for this.
        assert!(explain(&fixture("use-change-swtpm.txt"))
            .iter()
            .all(|e| !e.title.starts_with("There's no package")));
    }

    #[test]
    fn skipped_world_entry_from_the_vm() {
        let found = explain(&fixture("world-missing-package.txt"));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].severity, Severity::Warning);
        assert_eq!(
            found[0].title,
            "emerge skipped app-misc/tmuxx because it couldn't find it."
        );
    }

    #[test]
    fn slot_conflict() {
        let found = explain(&fixture("slot-conflict.txt"));
        assert_eq!(found.len(), 1);
        let e = &found[0];
        assert_eq!(
            e.title,
            "Two packages need different, incompatible versions of openssl."
        );
        assert_eq!(
            e.details,
            [
                "• curl needs >=dev-libs/openssl-3.0",
                "• old-app needs =dev-libs/openssl-1.1*"
            ]
        );
        assert!(e.fix.as_deref().unwrap().contains("old-app"));
    }

    #[test]
    fn blocker() {
        let found = explain(&fixture("blocker.txt"));
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].title,
            "fish and fishfetch can't be installed at the same time."
        );
    }

    #[test]
    fn fetch_failure() {
        let found = explain(&fixture("fetch-failed.txt"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Couldn't download the source code for foo.");
    }

    #[test]
    fn notices_are_info() {
        let found = explain(&fixture("news-and-depclean.txt"));
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|e| e.severity == Severity::Info));
        assert!(found[0].title.contains("26 announcements"));
    }

    #[test]
    fn info_items_get_a_tip() {
        let text = explain(&fixture("news-and-depclean.txt"))[0].render();
        assert!(
            text.contains("  Tip: run `eselect news read` to read them."),
            "{text}"
        );
    }

    #[test]
    fn unknown_output_explains_nothing() {
        assert!(explain("Calculating dependencies... done!\n").is_empty());
    }

    #[test]
    fn renders_with_icons_and_aligned_fix() {
        let text = explain(&fixture("use-change-swtpm.txt"))[0].render();
        assert!(text.starts_with("✗ swtpm needs gnutls"));
        assert!(text.contains("\n  Fix: add this to your bunny.toml"));
        assert!(
            text.contains("\n       [pkg.\"net-libs/gnutls\"]\n"),
            "{text}"
        );
    }
}
