//! Reading emerge's output, so bunny can show a short summary and a progress
//! line instead of emerge's full scroll. The full output is still saved to a
//! log, and `--verbose` shows it live as before.

use std::fmt::Write as _;

/// One package in emerge's merge list.
#[derive(Debug, PartialEq)]
pub struct Merge {
    /// `category/name`
    pub name: String,
    pub version: String,
    /// The version installed now, for updates and downgrades.
    pub from: Option<String>,
    pub action: Action,
    /// A prebuilt package (no compiling).
    pub binary: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Action {
    New,
    Update,
    Downgrade,
    /// Same version again (usually for changed USE flags).
    Rebuild,
}

/// Splits `cat/pkg-1.2.3-r1::gentoo` into (`cat/pkg`, `1.2.3-r1`).
/// Prebuilt packages from a binary host can carry a build number at the end
/// (`app-editors/vim-9.2.1202-6`); it's dropped, since it isn't the version.
fn split_cpv(cpv: &str) -> (String, String) {
    use crate::import::strip_version;
    let cpv = cpv.split("::").next().unwrap_or(cpv);
    let Some(mut name) = strip_version(cpv) else {
        return (cpv.to_string(), String::new());
    };
    // If what's left still ends in a version, the last part was a build number.
    if let Some(shorter) = strip_version(name) {
        if cpv[name.len() + 1..].bytes().all(|b| b.is_ascii_digit()) {
            let version = name[shorter.len() + 1..].to_string();
            name = shorter;
            return (name.to_string(), version);
        }
    }
    (name.to_string(), cpv[name.len() + 1..].to_string())
}

/// The packages from `emerge --pretend` output (lines like
/// `[binary   N    ] app-admin/sudo-1.9.17_p2::gentoo` or
/// `[ebuild     U  ] sys-libs/glibc-2.42-r1::gentoo [2.41-r6::gentoo]`).
pub fn parse_merge_list(output: &str) -> Vec<Merge> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix('[')?;
            let (head, tail) = rest.split_once(']')?;
            let mut words = head.split_whitespace();
            let kind = words.next()?;
            let binary = match kind {
                "binary" => true,
                "ebuild" => false,
                _ => return None, // blockers, uninstalls, nomerge
            };
            let flags: String = words.collect();
            let action = if flags.contains('N') {
                Action::New
            } else if flags.contains('D') {
                Action::Downgrade // shown as "UD"
            } else if flags.contains('U') {
                Action::Update
            } else {
                Action::Rebuild
            };
            let mut tokens = tail.split_whitespace();
            let (name, version) = split_cpv(tokens.next()?);
            let from = tokens
                .next()
                .and_then(|t| t.strip_prefix('[')?.strip_suffix(']'))
                .map(|old| old.split("::").next().unwrap_or(old).to_string())
                .filter(|old| {
                    !old.is_empty() && old.chars().next().is_some_and(|c| c.is_ascii_digit())
                });
            Some(Merge {
                name,
                version,
                from,
                action,
                binary,
            })
        })
        .collect()
}

/// The merge list, explained. `asked` are the packages the user asked for,
/// so the rest can be labelled as needed by them.
pub fn render_merge_list(list: &[Merge], asked: &[String]) -> String {
    let mut out = String::new();
    let build = list.iter().filter(|m| !m.binary).count();
    let _ = write!(
        out,
        "emerge will install {} package{}",
        list.len(),
        if list.len() == 1 { "" } else { "s" }
    );
    match (build, list.len() - build) {
        (0, _) => out.push_str(", all prebuilt:\n"),
        (b, 0) => {
            let _ = writeln!(out, ", {b} to compile:");
        }
        (b, p) => {
            let _ = writeln!(out, " ({p} prebuilt, {b} to compile):");
        }
    }
    let width = list.iter().map(|m| m.name.len()).max().unwrap_or(0);
    for m in list {
        let what = match (m.action, &m.from) {
            (Action::New, _) => m.version.clone(),
            (Action::Update, Some(from)) | (Action::Downgrade, Some(from)) => {
                format!("{from} → {}", m.version)
            }
            (Action::Rebuild, _) => format!("{} (rebuild)", m.version),
            (_, None) => m.version.clone(),
        };
        let mark = match m.action {
            Action::New => "+",
            Action::Rebuild => "~",
            Action::Update | Action::Downgrade => "↑",
        };
        let mark = if m.action == Action::Downgrade {
            "↓"
        } else {
            mark
        };
        let how = if m.binary { "" } else { "   compile" };
        let needed = if m.action == Action::New && !asked.iter().any(|a| a == &m.name) {
            "   (needed by another package)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {mark} {:<width$}  {what}{how}{needed}",
            m.name,
            width = width
        );
    }
    if build > 0 {
        out.push_str("  Compiling can take a while for big packages.\n");
    }
    out
}

/// Progress from a line like `>>> Emerging (3 of 9) app-admin/sudo-1.9::gentoo`
/// or `>>> Emerging binary (1 of 9) …`: (n, total, name, version, binary).
pub fn parse_progress(line: &str) -> Option<(usize, usize, String, String, bool)> {
    let rest = line.trim().strip_prefix(">>> Emerging ")?;
    let (binary, rest) = match rest.strip_prefix("binary ") {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let rest = rest.strip_prefix('(')?;
    let (count, rest) = rest.split_once(')')?;
    let (n, total) = count.split_once(" of ")?;
    let (name, version) = split_cpv(rest.split_whitespace().next()?);
    Some((
        n.trim().parse().ok()?,
        total.trim().parse().ok()?,
        name,
        version,
        binary,
    ))
}

/// Progress from `>>> Unmerging (1 of 9) app-text/tree-2.2.1...`.
pub fn parse_unmerge_progress(line: &str) -> Option<(usize, usize, String)> {
    let rest = line.trim().strip_prefix(">>> Unmerging (")?;
    let (count, rest) = rest.split_once(')')?;
    let (n, total) = count.split_once(" of ")?;
    let cpv = rest.split_whitespace().next()?.trim_end_matches('.');
    let (name, _) = split_cpv(cpv);
    Some((n.trim().parse().ok()?, total.trim().parse().ok()?, name))
}

/// Packages `emerge --depclean --pretend` would remove, from its
/// `All selected packages: =cat/pkg-1.0 =cat/other-2.0` line.
pub fn parse_depclean(output: &str) -> Option<Vec<String>> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("All selected packages:")?;
        Some(
            rest.split_whitespace()
                .map(|t| t.trim_start_matches('='))
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect(),
        )
    })
}

/// How many packages depclean says it would remove (`Number to remove: 3`),
/// to check the list above was read correctly.
pub fn depclean_count(output: &str) -> Option<usize> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Number to remove:")?
            .trim()
            .parse()
            .ok()
    })
}

/// Messages packages left for you after installing (emerge's "elog"),
/// from blocks like ` * Messages for package app-admin/sudo-1.9.17_p2:`.
pub fn parse_messages(output: &str) -> Vec<(String, Vec<String>)> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("* Messages for package ") {
            if let Some(done) = current.take() {
                found.push(done);
            }
            let cpv = rest
                .trim_end_matches(':')
                .split_whitespace()
                .next()
                .unwrap_or("");
            let (name, _) = split_cpv(cpv);
            current = Some((name, Vec::new()));
            continue;
        }
        let Some((_, lines)) = current.as_mut() else {
            continue;
        };
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.strip_prefix('*') {
            Some(text) => {
                let text = text.trim();
                if !text.is_empty() {
                    lines.push(text.to_string());
                }
            }
            None => {
                if let Some(done) = current.take() {
                    found.push(done);
                }
            }
        }
    }
    if let Some(done) = current {
        found.push(done);
    }
    // The same package can appear twice (once per phase); merge them.
    let mut merged: Vec<(String, Vec<String>)> = Vec::new();
    for (name, lines) in found.into_iter().filter(|(_, l)| !l.is_empty()) {
        match merged.iter_mut().find(|(n, _)| *n == name) {
            Some((_, existing)) => {
                for l in lines {
                    if !existing.contains(&l) {
                        existing.push(l);
                    }
                }
            }
            None => merged.push((name, lines)),
        }
    }
    merged
}

/// The last lines of output, for when emerge failed in a way bunny doesn't
/// recognise (in quiet mode the user hasn't seen any of it).
pub fn tail(output: &str, n: usize) -> String {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRETEND: &str = "
These are the packages that would be merged, in order:

Calculating dependencies  ... done!
Dependency resolution took 2.31 s (backtrack: 0/20).

[binary   N     ] acct-group/nullmail-0-r2::gentoo
[ebuild  N      ] mail-mta/nullmailer-2.2-r2::gentoo  USE=\"-ssl\"
[binary     U   ] sys-libs/glibc-2.42-r1::gentoo [2.41-r6::gentoo]
[ebuild     UD  ] dev-lang/python-3.12.7::gentoo [3.12.8::gentoo]
[ebuild   R     ] app-admin/sudo-1.9.17_p2::gentoo  USE=\"-sendmail*\"
[binary   N     ] app-admin/sudo-1.9.17_p2::gentoo
[blocks B      ] app-admin/doas (\"app-admin/doas\" is soft blocking app-admin/sudo-1.9.17_p2)
";

    #[test]
    fn reads_the_merge_list() {
        let list = parse_merge_list(PRETEND);
        assert_eq!(list.len(), 6);
        assert_eq!(
            list[0],
            Merge {
                name: "acct-group/nullmail".into(),
                version: "0-r2".into(),
                from: None,
                action: Action::New,
                binary: true,
            }
        );
        assert!(!list[1].binary);
        assert_eq!(list[2].action, Action::Update);
        assert_eq!(list[2].from.as_deref(), Some("2.41-r6"));
        assert_eq!(list[3].action, Action::Downgrade);
        assert_eq!(list[4].action, Action::Rebuild);
    }

    #[test]
    fn explains_the_merge_list() {
        let list = parse_merge_list(PRETEND);
        let text = render_merge_list(&list[..5], &["app-admin/sudo".into()]);
        assert!(text.starts_with("emerge will install 5 packages (2 prebuilt, 3 to compile):\n"));
        assert!(text.contains("+ acct-group/nullmail  0-r2   (needed by another package)"));
        assert!(text.contains("↑ sys-libs/glibc"));
        assert!(text.contains("2.41-r6 → 2.42-r1"));
        assert!(text.contains("↓ dev-lang/python"));
        assert!(text.contains("~ app-admin/sudo"));
        assert!(text.contains("1.9.17_p2 (rebuild)   compile"));
        assert!(text.contains("Compiling can take a while"));
    }

    #[test]
    fn all_prebuilt() {
        let list = parse_merge_list("[binary   N     ] app-text/tree-2.2.1::gentoo\n");
        let text = render_merge_list(&list, &["app-text/tree".into()]);
        assert_eq!(
            text,
            "emerge will install 1 package, all prebuilt:\n  + app-text/tree  2.2.1\n"
        );
    }

    #[test]
    fn reads_progress() {
        assert_eq!(
            parse_progress(">>> Emerging binary (1 of 9) app-admin/sudo-1.9.17_p2::gentoo"),
            Some((1, 9, "app-admin/sudo".into(), "1.9.17_p2".into(), true))
        );
        assert_eq!(
            parse_progress(">>> Emerging (3 of 9) mail-mta/nullmailer-2.2-r2::gentoo"),
            Some((3, 9, "mail-mta/nullmailer".into(), "2.2-r2".into(), false))
        );
        assert_eq!(parse_progress(">>> Installing (3 of 9) x/y-1"), None);
        assert_eq!(
            parse_unmerge_progress(">>> Unmerging (2 of 9) mail-mta/nullmailer-2.2-r2..."),
            Some((2, 9, "mail-mta/nullmailer".into()))
        );
    }

    #[test]
    fn reads_depclean() {
        let out = "
>>> These are the packages that would be unmerged:

 app-text/tree
    selected: 2.2.1
   protected: none
     omitted: none

All selected packages: =app-text/tree-2.2.1 =mail-mta/nullmailer-2.2-r2

>>> 'Selected' packages are slated for removal.

Number to remove:     2
";
        assert_eq!(
            parse_depclean(out),
            Some(vec![
                "app-text/tree-2.2.1".to_string(),
                "mail-mta/nullmailer-2.2-r2".to_string()
            ])
        );
        assert_eq!(depclean_count(out), Some(2));
        assert_eq!(parse_depclean("Nothing to remove.\n"), None);
    }

    #[test]
    fn reads_package_messages() {
        let out = "
>>> Completed (9 of 9) app-admin/sudo-1.9.17_p2::gentoo

 * Messages for package mail-mta/nullmailer-2.2-r2:

 * To create an initial setup, please do:
 * emerge --config =mail-mta/nullmailer-2.2-r2

 * Messages for package app-admin/sudo-1.9.17_p2:

 * To use the -A (askpass) option, you need to install a compatible
 * password program.
 * Messages for package app-admin/sudo-1.9.17_p2:
 * password program.
>>> Auto-cleaning packages...
 * GNU info directory index is up-to-date.
";
        let msgs = parse_messages(out);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].0, "mail-mta/nullmailer");
        assert_eq!(
            msgs[0].1,
            [
                "To create an initial setup, please do:",
                "emerge --config =mail-mta/nullmailer-2.2-r2"
            ]
        );
        assert_eq!(msgs[1].0, "app-admin/sudo");
        assert_eq!(msgs[1].1.len(), 2);
    }

    #[test]
    fn tail_skips_blank_lines() {
        assert_eq!(tail("a\n\nb\nc\n\n", 2), "b\nc");
        assert_eq!(tail("a\n", 5), "a");
    }

    #[test]
    fn build_numbers_from_the_binary_host_are_not_versions() {
        let list = parse_merge_list(
            "[binary   N     ] app-editors/vim-9.2.1202-6::gentoo\n\
             [binary   N     ] app-eselect/eselect-vi-20221122-1::gentoo\n\
             [binary   N     ] media-fonts/font-adobe-100dpi-1.0.4-r2-3::gentoo\n\
             [ebuild   N     ] dev-util/xxd-2025.08.24-r1::gentoo\n",
        );
        let pairs: Vec<(&str, &str)> = list
            .iter()
            .map(|m| (m.name.as_str(), m.version.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("app-editors/vim", "9.2.1202"),
                ("app-eselect/eselect-vi", "20221122"),
                ("media-fonts/font-adobe-100dpi", "1.0.4-r2"),
                ("dev-util/xxd", "2025.08.24-r1"),
            ]
        );
        let text = render_merge_list(&list[..1], &["app-editors/vim".into()]);
        assert!(!text.contains("needed by another"), "{text}");
        assert_eq!(
            parse_progress(">>> Emerging binary (6 of 6) app-editors/vim-9.2.1202-6::gentoo"),
            Some((6, 6, "app-editors/vim".into(), "9.2.1202".into(), true))
        );
    }
}
