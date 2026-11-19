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
    /// Why a package is built again at the same version (rebuilds only).
    pub why: Why,
}

/// Why emerge installs the same version again.
#[derive(Debug, Default, PartialEq, Clone)]
pub struct Why {
    /// USE flags that changed, as `+flag` (turned on) or `-flag` (off).
    pub use_flags: Vec<String>,
    /// Libraries it uses that changed in a way that needs it rebuilt
    /// (Gentoo's "sub-slot" changes), by `category/name`, when known.
    pub libraries: Vec<String>,
    /// emerge marked it as a forced reinstall (`r`), which is what a
    /// library change looks like in its screen text (it doesn't say which).
    pub library_changed: bool,
}

impl Why {
    /// Short enough for the plan table: `USE +adns -http2`,
    /// `libunistring changed`, `a library it uses changed`. Empty when the
    /// reason isn't known.
    pub fn short(&self) -> String {
        let mut parts = Vec::new();
        if !self.use_flags.is_empty() {
            const MAX: usize = 3;
            let mut flags = self.use_flags[..self.use_flags.len().min(MAX)].join(" ");
            if self.use_flags.len() > MAX {
                flags.push_str(" …");
            }
            parts.push(format!("USE {flags}"));
        }
        if let Some(first) = self.libraries.first() {
            let name = first.rsplit('/').next().unwrap_or(first);
            parts.push(match self.libraries.len() {
                1 => format!("{name} changed"),
                n => format!("{name} and {} more changed", n - 1),
            });
        } else if self.library_changed {
            parts.push("a library it uses changed".into());
        }
        parts.join(", ")
    }
}

/// The USE flags emerge marks as changed in a `--verbose` merge-list line:
/// `USE="adns* -http2% ssl" CURL_SSL="openssl -gnutls*"` gives
/// `+adns`, `-http2`, `-curl_ssl_gnutls`. `*` means the flag's setting
/// changed since the installed version, `%` that it's new or gone.
fn changed_use(tail: &str) -> Vec<String> {
    let mut changed = Vec::new();
    let mut rest = tail;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].rsplit(' ').next().unwrap_or("");
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        for token in after[..end].split_whitespace() {
            if !token.contains(['*', '%']) {
                continue;
            }
            let off = token.starts_with('-') || token.starts_with("(-");
            let flag = token.trim_matches(|c: char| "-*%(){}".contains(c));
            if flag.is_empty() {
                continue;
            }
            let flag = if key == "USE" || key.is_empty() {
                flag.to_string()
            } else {
                format!("{}_{flag}", key.to_lowercase())
            };
            changed.push(format!("{}{flag}", if off { '-' } else { '+' }));
        }
        rest = &after[end + 1..];
    }
    changed
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
            let forced = flags.contains('r');
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
            let why = match action {
                Action::Rebuild => Why {
                    use_flags: changed_use(tail),
                    libraries: Vec::new(),
                    library_changed: forced,
                },
                _ => Why::default(),
            };
            Some(Merge {
                name,
                version,
                from,
                action,
                binary,
                why,
            })
        })
        .collect()
}

/// Dependencies shown in full up to this many; more are folded.
const SHOWN_NEEDED: usize = 15;

/// The merge list, explained. `asked` are the packages the user asked for.
/// They come first; new packages they need follow under their own heading
/// (folded when there are many), so every line stays short enough not to
/// wrap in a narrow terminal.
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
    let needed = |m: &Merge| m.action == Action::New && !asked.iter().any(|a| a == &m.name);
    let (main, deps): (Vec<&Merge>, Vec<&Merge>) = list.iter().partition(|m| !needed(m));

    let what = |m: &Merge| match (m.action, &m.from) {
        (Action::New, _) => m.version.clone(),
        (Action::Update, Some(from)) | (Action::Downgrade, Some(from)) => {
            format!("{from} → {}", m.version)
        }
        (Action::Rebuild, _) => match m.why.short() {
            why if why.is_empty() => format!("{} (rebuild)", m.version),
            why => format!("{} (rebuild: {why})", m.version),
        },
        (_, None) => m.version.clone(),
    };
    let shown_deps: Vec<&Merge> = if deps.len() > SHOWN_NEEDED + 3 {
        deps.iter().take(SHOWN_NEEDED).copied().collect()
    } else {
        deps.clone()
    };
    let visible: Vec<&Merge> = main.iter().chain(shown_deps.iter()).copied().collect();
    let width = visible.iter().map(|m| m.name.len()).max().unwrap_or(0);
    let what_width = visible
        .iter()
        .map(|m| what(m).chars().count())
        .max()
        .unwrap_or(0);
    let row = |out: &mut String, m: &Merge| {
        let mark = match m.action {
            Action::New => "+",
            Action::Rebuild => "~",
            Action::Update => "↑",
            Action::Downgrade => "↓",
        };
        let w = what(m);
        let pad = what_width - w.chars().count();
        let how = if m.binary { "" } else { "compile" };
        let line = format!(
            "  {mark} {:<width$}  {w}{}{how}",
            m.name,
            " ".repeat(pad + 3)
        );
        let _ = writeln!(out, "{}", line.trim_end());
    };
    for m in &main {
        row(&mut out, m);
    }
    if !deps.is_empty() {
        if !main.is_empty() {
            let dep_build = deps.iter().filter(|m| !m.binary).count();
            let _ = writeln!(
                out,
                "\n  And {} package{} {} need{}{}:",
                deps.len(),
                if deps.len() == 1 { "" } else { "s" },
                if main.len() == 1 { "it" } else { "they" },
                if main.len() == 1 { "s" } else { "" },
                match dep_build {
                    0 => String::new(),
                    n if n == deps.len() => ", all to compile".to_string(),
                    n => format!(" ({n} to compile)"),
                }
            );
        }
        for m in &shown_deps {
            row(&mut out, m);
        }
        let hidden = deps.len() - shown_deps.len();
        if hidden > 0 {
            let _ = writeln!(
                out,
                "  … and {hidden} more. `--verbose` lists every one, and so does the log afterwards."
            );
        }
    }
    if build > 0 {
        out.push_str("  Compiling can take a while for big packages.\n");
    }
    out
}

/// Asked-for packages that would be compiled although the repositories have
/// a prebuilt version (`<name>-bin`): (package, prebuilt package).
pub fn prebuilt_alternatives(
    list: &[Merge],
    asked: &[String],
    available: &std::collections::BTreeSet<String>,
) -> Vec<(String, String)> {
    list.iter()
        .filter(|m| !m.binary && asked.iter().any(|a| a == &m.name))
        .filter_map(|m| {
            let bin = format!("{}-bin", m.name);
            available.contains(&bin).then(|| (m.name.clone(), bin))
        })
        .collect()
}

/// How far a build is, from the end of its build log: build tools print
/// counters like `[1234/5678] Compiling …` (ninja, meson, cargo-ninja) or
/// `[ 45%] Building …` (make with CMake). Returns a percentage.
pub fn build_percent(log_tail: &str) -> Option<u8> {
    for line in log_tail.lines().rev() {
        let line = line.trim_start();
        let Some(inner) = line.strip_prefix('[').and_then(|l| l.split_once(']')) else {
            continue;
        };
        let inner = inner.0.trim();
        if let Some(pct) = inner.strip_suffix('%') {
            if let Ok(p) = pct.trim().parse::<u8>() {
                if p <= 100 {
                    return Some(p);
                }
            }
        } else if let Some((done, total)) = inner.split_once('/') {
            if let (Ok(done), Ok(total)) = (done.trim().parse::<u64>(), total.trim().parse::<u64>())
            {
                if total > 0 && done <= total {
                    return Some((done * 100 / total) as u8);
                }
            }
        }
    }
    None
}

/// How long each package took to build last time, from Portage's own
/// history (`/var/log/emerge.log`): `category/name` → seconds.
pub fn build_times(emerge_log: &str) -> std::collections::BTreeMap<String, u64> {
    let mut started: std::collections::BTreeMap<String, u64> = Default::default();
    let mut took = std::collections::BTreeMap::new();
    for line in emerge_log.lines() {
        let Some((time, rest)) = line.split_once(':') else {
            continue;
        };
        let Ok(time) = time.trim().parse::<u64>() else {
            continue;
        };
        let rest = rest.trim();
        let (done, rest) = if let Some(r) = rest.strip_prefix(">>> emerge (") {
            (false, r)
        } else if let Some(r) = rest.strip_prefix("::: completed emerge (") {
            (true, r)
        } else {
            continue;
        };
        let Some(cpv) = rest
            .split_once(") ")
            .and_then(|(_, r)| r.split_whitespace().next())
        else {
            continue;
        };
        if done {
            if let Some(start) = started.remove(cpv) {
                let (name, _) = split_cpv(cpv);
                took.insert(name, time.saturating_sub(start));
            }
        } else {
            started.insert(cpv.to_string(), time);
        }
    }
    took
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
            // A blank line after the messages ends the block; whatever
            // follows (" * GNU info directory index is up-to-date.") is
            // emerge's own housekeeping, not the package talking.
            if !lines.is_empty() {
                if let Some(done) = current.take() {
                    found.push(done);
                }
            }
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
                why: Why::default(),
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
        // "compile" lines up in one column, whatever the version's length.
        let column = |needle: &str, what: &str| {
            text.lines()
                .find(|l| l.contains(needle))
                .and_then(|l| l.find(what))
        };
        assert_eq!(
            column("nullmailer", "compile"),
            column("sudo", "compile"),
            "{text}"
        );
        assert!(text.contains("+ acct-group/nullmail  0-r2"), "{text}");
        assert!(text.contains("↑ sys-libs/glibc"));
        assert!(text.contains("2.41-r6 → 2.42-r1"));
        assert!(text.contains("↓ dev-lang/python"));
        assert!(text.contains("~ app-admin/sudo"));
        assert!(
            text.contains("1.9.17_p2 (rebuild: USE -sendmail)"),
            "{text}"
        );
        assert!(text.contains("Compiling can take a while"));
        // What was asked for (and updates) first; new dependencies after.
        let sudo = text.find("app-admin/sudo").unwrap();
        let heading = text.find("And 2 packages they need").expect(&text);
        assert!(sudo < heading && heading < text.find("acct-group/nullmail").unwrap());
        assert!(!text.contains("needed by"), "{text}");
    }

    #[test]
    fn many_dependencies_are_folded() {
        let mut pretend = String::from("[ebuild  N     ] www-client/firefox-153.7.0::gentoo\n");
        for i in 0..40 {
            pretend.push_str(&format!("[ebuild  N     ] dev-libs/lib{i}-1.0::gentoo\n"));
        }
        let text = render_merge_list(&parse_merge_list(&pretend), &["www-client/firefox".into()]);
        assert!(
            text.contains("And 40 packages it needs, all to compile:"),
            "{text}"
        );
        assert!(text.contains("dev-libs/lib14 "), "{text}");
        assert!(!text.contains("dev-libs/lib15 "), "{text}");
        assert!(text.contains("… and 25 more."), "{text}");
        let longest = text.lines().map(|l| l.chars().count()).max().unwrap();
        assert!(
            longest <= 80,
            "a line would wrap in an 80-column terminal: {text}"
        );
    }

    #[test]
    fn says_why_a_package_is_rebuilt() {
        // From `bunny add dev-util/github-cli` in the dry run: rebuilds
        // shown as "(rebuild)" with no reason.
        let list = parse_merge_list(
            "[binary   R    ] net-misc/curl-8.16.0-1::gentoo  USE=\"adns* http2 -ldap ssl\" CURL_SSL=\"openssl -gnutls*\" 0 KiB\n\
             [ebuild  rR    ] sys-devel/gettext-0.23.1::gentoo  USE=\"acl nls -static-libs\" 0 KiB\n\
             [ebuild   R    ] net-misc/iputils-20250605::gentoo  USE=\"(-caps%) filecaps\" 0 KiB\n\
             [binary   R    ] app-arch/libb2-0.98.1-r3-2::gentoo  0 KiB\n",
        );
        assert_eq!(list[0].why.use_flags, ["+adns", "-curl_ssl_gnutls"]);
        assert!(!list[0].why.library_changed);
        assert!(list[1].why.use_flags.is_empty());
        assert!(list[1].why.library_changed, "rR: a forced reinstall");
        assert_eq!(list[2].why.use_flags, ["-caps"]);
        assert_eq!(list[3].why, Why::default());

        let text = render_merge_list(&list, &[]);
        assert!(
            text.contains("8.16.0 (rebuild: USE +adns -curl_ssl_gnutls)"),
            "{text}"
        );
        assert!(
            text.contains("0.23.1 (rebuild: a library it uses changed)"),
            "{text}"
        );
        assert!(text.contains("20250605 (rebuild: USE -caps)"), "{text}");
        // No reason emerge gave: just "(rebuild)", never an invented one.
        assert!(text.contains("0.98.1-r3 (rebuild)"), "{text}");
        let longest = text.lines().map(|l| l.chars().count()).max().unwrap();
        assert!(longest <= 80, "a line would wrap: {text}");
    }

    #[test]
    fn short_reasons_stay_short() {
        let why = Why {
            use_flags: vec!["+a".into(), "+b".into(), "-c".into(), "+d".into()],
            libraries: vec!["dev-libs/libunistring".into(), "dev-libs/icu".into()],
            library_changed: true,
        };
        assert_eq!(
            why.short(),
            "USE +a +b -c …, libunistring and 1 more changed"
        );
        let one = Why {
            libraries: vec!["dev-libs/openssl".into()],
            ..Why::default()
        };
        assert_eq!(one.short(), "openssl changed");
        assert_eq!(Why::default().short(), "");
    }

    #[test]
    fn unchanged_flags_arent_reasons() {
        assert!(changed_use(" USE=\"acl nls -static-libs\" 1,234 KiB").is_empty());
        assert_eq!(changed_use(" USE=\"-sendmail*\""), ["-sendmail"]);
        assert_eq!(
            changed_use(" PYTHON_TARGETS=\"python3_13% -python3_12%\""),
            ["+python_targets_python3_13", "-python_targets_python3_12"]
        );
    }

    #[test]
    fn suggests_a_prebuilt_version() {
        let list = parse_merge_list(
            "[ebuild  N     ] www-client/firefox-153.7.0::gentoo\n\
             [ebuild  N     ] llvm-core/clang-22.1.8::gentoo\n",
        );
        let available: std::collections::BTreeSet<String> =
            ["www-client/firefox-bin", "llvm-core/clang"]
                .map(String::from)
                .into();
        assert_eq!(
            prebuilt_alternatives(&list, &["www-client/firefox".into()], &available),
            vec![("www-client/firefox".into(), "www-client/firefox-bin".into())]
        );
        // Only for what was asked for: a dependency isn't something to swap.
        assert!(prebuilt_alternatives(&list, &[], &available).is_empty());
    }

    #[test]
    fn reads_build_counters() {
        assert_eq!(
            build_percent("make stuff\n[1234/5678] Compiling foo.o\nwarning: x\n"),
            Some(21)
        );
        assert_eq!(build_percent("[ 45%] Building CXX object a.o\n"), Some(45));
        assert_eq!(
            build_percent("[ok] fine\n[2026-11-18] date\nno counters\n"),
            None
        );
        assert_eq!(build_percent("[5/0] nonsense\n[10/5] also\n"), None);
    }

    #[test]
    fn reads_build_times_from_portage_history() {
        let log = "1700000000:  >>> emerge (1 of 2) www-client/firefox-150.0::gentoo to /\n\
                   1700000001:  === (1 of 2) Compiling/Merging (www-client/firefox-150.0::gentoo)\n\
                   1700004800:  ::: completed emerge (1 of 2) www-client/firefox-150.0::gentoo to /\n\
                   1700004801:  >>> emerge (2 of 2) app-misc/tmux-3.6a::gentoo to /\n\
                   1700009000:  >>> emerge (1 of 1) www-client/firefox-153.7.0::gentoo to /\n\
                   1700012000:  ::: completed emerge (1 of 1) www-client/firefox-153.7.0::gentoo to /\n";
        let times = build_times(log);
        assert_eq!(
            times.get("www-client/firefox"),
            Some(&3000),
            "the newest build counts"
        );
        assert_eq!(times.get("app-misc/tmux"), None, "never finished");
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

    #[test]
    fn housekeeping_after_the_messages_is_left_out() {
        let out = "
 * Messages for package sys-process/htop-3.5.3:

 * Install additional packages for optional runtime features:
 *   sys-process/lsof for Viewing processes accessing certain files

 * GNU info directory index is up-to-date.
";
        let msgs = parse_messages(out);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].1.len(), 2, "{:?}", msgs[0].1);
        assert!(!msgs[0].1.iter().any(|l| l.contains("GNU info")));
    }
}
