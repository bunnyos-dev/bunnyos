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
/// The start of the explanation for emerge's "remove obsolete packages"
/// reminder (left out of switch tips, since bunny just did that).
pub const DEPCLEAN_REMINDER: &str = "emerge reminds you to remove unused packages";

pub fn explain(output: &str) -> Vec<Explanation> {
    finish(recognise(output).into_iter().map(|(_, e)| e).collect())
}

/// Which recogniser an explanation came from. The first three are the kinds
/// bunny's Portage helper also reports as data.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Use,
    Keywords,
    Masked,
    Other,
}

/// Everything recognised in emerge's text, with the kind of each.
fn recognise(output: &str) -> Vec<(Kind, Explanation)> {
    // Color codes are invisible on screen but break reading the text.
    let output = strip_ansi(output);
    let lines: Vec<&str> = output.lines().map(str::trim_end).collect();
    let mut found = Vec::new();
    let mut add = |kind, list: Vec<Explanation>| found.extend(list.into_iter().map(|e| (kind, e)));
    add(Kind::Use, use_changes(&lines));
    add(Kind::Keywords, keyword_changes(&lines));
    add(Kind::Masked, masked(&lines));
    add(Kind::Other, no_such_package(&lines));
    add(Kind::Other, skipped_world_entries(&lines));
    add(Kind::Other, slot_conflicts(&lines));
    add(Kind::Other, blockers(&lines));
    add(Kind::Other, fetch_failures(&lines));
    add(Kind::Other, config_updates(&lines));
    add(Kind::Other, notices(&lines));
    found
}

/// Explains a refused plan: the problems bunny's Portage helper reported as
/// data, plus anything else recognised in emerge's text. For each kind the
/// helper reported, its data replaces the text (which stays the fallback for
/// kinds it didn't report). One exception: when emerge's text says its own
/// USE suggestion breaks a package's rules, the text's explanation of that
/// is kept, since the helper doesn't report those rules yet.
pub fn explain_with_problems(
    output: &str,
    problems: &[crate::helper::Problem],
) -> Vec<Explanation> {
    let text = recognise(output);
    let mut data = from_problems(problems);
    if text
        .iter()
        .any(|(kind, e)| *kind == Kind::Use && is_rules_conflict(e))
    {
        data.retain(|(kind, _)| *kind != Kind::Use);
    }
    let covered: Vec<Kind> = data.iter().map(|(kind, _)| *kind).collect();
    let mut found: Vec<Explanation> = data.into_iter().map(|(_, e)| e).collect();
    found.extend(
        text.into_iter()
            .filter(|(kind, _)| !covered.contains(kind))
            .map(|(_, e)| e),
    );
    finish(found)
}

/// Explanations built from the helper's data.
fn from_problems(problems: &[crate::helper::Problem]) -> Vec<(Kind, Explanation)> {
    use crate::helper::Problem;
    let chain =
        |by: &[String]| -> Vec<String> { by.iter().map(|n| short(n).to_string()).collect() };
    problems
        .iter()
        .map(|p| match p {
            Problem::Keywords {
                name,
                version,
                keyword,
                required_by,
            } => {
                let e = if version.contains("9999") || keyword == "**" {
                    live_version(
                        name,
                        &format!("={name}-{version} {keyword}"),
                        &chain(required_by),
                        false,
                    )
                } else {
                    testing_version(name, keyword, &chain(required_by))
                };
                (Kind::Keywords, e)
            }
            Problem::Masked {
                name,
                reasons,
                comment,
                required_by,
            } => {
                let keyword = reasons
                    .iter()
                    .find_map(|r| r.strip_suffix(" keyword").filter(|k| k.starts_with('~')));
                let e = if reasons.iter().any(|r| r == "package.mask") {
                    masked_on_purpose(name, comment.as_deref())
                } else if let Some(keyword) = keyword {
                    testing_version(name, keyword, &chain(required_by))
                } else if reasons.iter().any(|r| r == "missing keyword") {
                    not_for_this_computer(name)
                } else {
                    Explanation::new(
                        Severity::Problem,
                        format!(
                            "{} can't be installed: Gentoo marks it as masked ({}).",
                            short(name),
                            reasons.join(", ")
                        ),
                    )
                    .fix("run the same bunny command with --verbose to see emerge's full message.")
                };
                (Kind::Masked, e)
            }
            Problem::Use {
                name,
                changes,
                required_by,
                ..
            } => {
                let flags = changes
                    .iter()
                    .map(|(flag, on)| {
                        if *on {
                            flag.clone()
                        } else {
                            format!("-{flag}")
                        }
                    })
                    .collect();
                (
                    Kind::Use,
                    use_change(name, flags, required_by.first().cloned()),
                )
            }
        })
        .collect()
}

/// Most important first, each once.
fn finish(mut found: Vec<Explanation>) -> Vec<Explanation> {
    found.sort_by_key(|e| match e.severity {
        Severity::Problem => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    });
    // emerge repeats some notices (news at the start and the end), so show
    // each explanation once.
    let mut seen = std::collections::HashSet::new();
    found.retain(|e| seen.insert(e.title.clone()));
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
    let form1_start = found.len();
    let mut form1_conflict = None;
    for (i, line) in lines.iter().enumerate() {
        let Some(start) = line.find("(Change USE:") else {
            continue;
        };
        let atom = line
            .trim_start_matches(['-', ' '])
            .split_whitespace()
            .next();
        let Some(atom) = atom else { continue };
        let inside = &line[start + "(Change USE:".len()..];
        // "(Change USE: -X, this change violates use flag constraints defined
        // by …: '<rules>')": emerge's own suggestion breaks the rules.
        if let Some(at) = inside.find("this change violates") {
            let (suggested, rest) = inside.split_at(at);
            let flags = suggested
                .split_whitespace()
                .map(|f| f.trim_end_matches(',').trim_start_matches('+').to_string())
                .filter(|f| !f.is_empty())
                .collect();
            let rule = match (rest.find('\''), rest.rfind('\'')) {
                (Some(a), Some(b)) if b > a => &rest[a + 1..b],
                _ => "",
            };
            form1_conflict = Some(flag_rules_conflict(&base_name(atom), flags, rule));
            continue;
        }
        let inside = inside.trim_end().trim_end_matches(')');
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
    if let Some(c) = form1_conflict {
        let others = found.drain(form1_start..).collect();
        found.push(fold_attempt(c, others));
    }

    // Form 2: the list after "The following USE changes are necessary".
    for block in blocks_after(lines, "The following USE changes are necessary") {
        let mut needed_by = None;
        // If emerge's suggestion breaks a package's rules, the other changes
        // in the same suggestion are part of an attempt that doesn't work.
        let start = found.len();
        let mut conflict = None;
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
            // emerge's own suggestion breaks the package's rules: it couldn't
            // find a working set of flags at all.
            if let Some(at) = line.find("this change violates") {
                let (suggested, rest) = line.split_at(at);
                let flags = suggested
                    .split_whitespace()
                    .skip(1)
                    .map(|f| f.trim_end_matches(',').to_string())
                    .filter(|f| !f.is_empty())
                    .collect();
                let rule = rest
                    .split_once('\'')
                    .map(|(_, r)| r.trim_end().trim_end_matches('\''))
                    .unwrap_or("");
                conflict = Some(flag_rules_conflict(&base_name(atom), flags, rule));
                needed_by.take();
                continue;
            }
            let flags = words.map(String::from).collect();
            found.push(use_change(&base_name(atom), flags, needed_by.take()));
        }
        if let Some(c) = conflict {
            let others = found.drain(start..).collect();
            found.push(fold_attempt(c, others));
        }
    }
    // emerge often prints the same suggestion in both forms. If any of them
    // broke a package's rules, every other suggestion is part of an attempt
    // that doesn't work: fold them all into the first conflict, once each.
    if let Some(first) = found.iter().position(is_rules_conflict) {
        let mut conflict = found.remove(first);
        let mut others = Vec::new();
        found.retain(|e| {
            if is_rules_conflict(e) {
                return true;
            }
            others.push(e.title.clone());
            false
        });
        let mentioned = conflict.details.join("\n");
        others.retain(|t| !mentioned.contains(t.trim_end_matches('.')));
        others.dedup();
        if !others.is_empty() {
            conflict = fold_attempt(
                conflict,
                others
                    .into_iter()
                    .map(|t| Explanation::new(Severity::Problem, t))
                    .collect(),
            );
        }
        found.insert(0, conflict);
    }
    found
}

fn is_rules_conflict(e: &Explanation) -> bool {
    e.title
        .starts_with("emerge couldn't find a set of features")
}

/// The other changes emerge suggested alongside a broken one belong to the
/// same attempt that doesn't work: mention them inside the conflict rather
/// than offering each as a fix.
fn fold_attempt(mut conflict: Explanation, others: Vec<Explanation>) -> Explanation {
    let titles: Vec<String> = others
        .iter()
        .map(|e| e.title.trim_end_matches('.').to_string())
        .collect();
    if !titles.is_empty() {
        conflict = conflict.detail(format!(
            "emerge also suggested this as part of the same attempt; skip it too: {}.",
            titles.join("; ")
        ));
    }
    conflict
}

/// emerge couldn't find USE flags that satisfy a package's rules
/// (REQUIRED_USE): its suggested change breaks them.
fn flag_rules_conflict(package: &str, suggested: Vec<String>, rule: &str) -> Explanation {
    let name = short(package);
    let describe = |f: &String| match f.strip_prefix('-') {
        Some(off) => format!("turning {off} off"),
        None => format!("turning {f} on"),
    };
    let suggestion: Vec<String> = suggested.iter().map(describe).collect();
    let mut e = Explanation::new(
        Severity::Problem,
        format!("emerge couldn't find a set of features that works for {name}."),
    );
    if !suggestion.is_empty() {
        e = e.detail(format!(
            "Its suggested fix ({}) breaks {name}'s own rules, so don't use it.",
            suggestion.join(", ")
        ));
    }
    let rules = required_use_rules(rule);
    if !rules.is_empty() {
        e = e.detail(format!("{name}'s rules:"));
        for r in rules {
            e = e.detail(format!("  • {r}"));
        }
    }
    let display = ["X", "wayland"]
        .iter()
        .all(|f| rule.split_whitespace().any(|t| t == *f));
    let fix = if display {
        format!(
            "{name} needs a way to show things on screen, and nothing is turned on.\n\
             On a desktop, turn on the one you use (wayland or X), for example:\n\
             [pkg.\"{package}\"]\n\
             use = [\"wayland\"]\n\
             Then run the same bunny command again. If another package then needs the\n\
             same flag, bunny will say so."
        )
    } else {
        format!(
            "turn on features that meet the rules above in [pkg.\"{package}\"], then run the\n\
             same bunny command again."
        )
    };
    e.fix(fix)
}

/// Gentoo's REQUIRED_USE syntax in words, one rule per line:
/// `any-of ( X wayland )` → "at least one of: X, wayland",
/// `egl? ( any-of ( X drm ) )` → "if egl is on: at least one of: X, drm".
fn required_use_rules(rule: &str) -> Vec<String> {
    let tokens: Vec<&str> = rule.split_whitespace().collect();
    let mut pos = 0;
    let mut rules = Vec::new();
    while pos < tokens.len() {
        match required_use_item(&tokens, &mut pos) {
            Some(text) => rules.push(text),
            None => break,
        }
    }
    rules
}

fn required_use_item(tokens: &[&str], pos: &mut usize) -> Option<String> {
    let token = *tokens.get(*pos)?;
    *pos += 1;
    let group = |tokens: &[&str], pos: &mut usize| -> Option<Vec<String>> {
        if tokens.get(*pos) != Some(&"(") {
            return None;
        }
        *pos += 1;
        let mut items = Vec::new();
        while tokens.get(*pos).is_some_and(|t| *t != ")") {
            items.push(required_use_item(tokens, pos)?);
        }
        *pos += 1; // ")"
        Some(items)
    };
    let quantified = |word: &str, tokens: &[&str], pos: &mut usize| {
        group(tokens, pos).map(|items| format!("{word}: {}", items.join(", ")))
    };
    match token {
        "||" | "any-of" => quantified("at least one of", tokens, pos),
        "^^" | "exactly-one-of" => quantified("exactly one of", tokens, pos),
        "??" | "at-most-one-of" => quantified("at most one of", tokens, pos),
        "(" => {
            *pos -= 1;
            group(tokens, pos).map(|items| format!("all of: {}", items.join(", ")))
        }
        t if t.ends_with('?') => {
            let flag = t.trim_end_matches('?');
            let condition = match flag.strip_prefix('!') {
                Some(off) => format!("if {off} is off"),
                None => format!("if {flag} is on"),
            };
            let items = group(tokens, pos)?;
            Some(format!("{condition}: {}", items.join("; ")))
        }
        t => Some(match t.strip_prefix('!') {
            Some(off) => format!("{off} off"),
            None => t.to_string(),
        }),
    }
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
            if live {
                found.push(live_version(&package, line, &chain, written));
            } else {
                found.push(testing_version(&package, keyword, &chain));
            }
        }
    }
    found
}

/// "It's needed because of: a → b."
fn because(chain: &[String]) -> Option<String> {
    (!chain.is_empty()).then(|| format!("It's needed because of: {}.", chain.join(" → ")))
}

/// emerge wants a live (`9999`) or any-keyword (`**`) version: risky.
/// `line` is emerge's suggestion; `written`: emerge saved it in `._cfg` files.
fn live_version(package: &str, line: &str, chain: &[String], written: bool) -> Explanation {
    let mut e = Explanation::new(
        Severity::Warning,
        format!(
            "emerge wants to use the live development version of {} ({line}).",
            short(package)
        ),
    )
    .detail("That's unreleased code built straight from the developers' git repository,")
    .detail("not a tested release. Don't accept this.");
    if let Some(because) = because(chain) {
        e = e.detail(because);
    }
    let mut fix = String::from(
        "run the command again with --autounmask=n to see the real problem,\n\
         and paste that into `bunny explain`.",
    );
    if written {
        let a = crate::security::admin();
        fix.push_str(&format!(
            "\nemerge saved this suggestion in waiting files. Delete them, don't accept them:\n\
             {a} find /etc/portage -name '._cfg*' -exec grep -l '9999' {{}} + | xargs -r {a} rm -v",
        ));
    }
    e.fix(fix)
}

/// Only a testing version would do. An empty `keyword` means `~amd64`.
fn testing_version(package: &str, keyword: &str, chain: &[String]) -> Explanation {
    let title = if keyword.is_empty() {
        format!(
            "{} is only available as a testing version right now.",
            short(package)
        )
    } else {
        format!(
            "{} is only available as a testing version ({keyword}) right now.",
            short(package)
        )
    };
    let mut e = Explanation::new(Severity::Problem, title).detail(
        "Testing versions work for most people but haven't been declared stable by Gentoo yet.",
    );
    if let Some(because) = because(chain) {
        e = e.detail(because);
    }
    let keyword = if keyword.is_empty() {
        "~amd64"
    } else {
        keyword
    };
    e.fix(format!(
        "to allow testing versions of it, add this to bunny.toml, then run\n\
         the command again:\n\
         [pkg.\"{package}\"]\n\
         keywords = [\"{keyword}\"]"
    ))
}

/// No version has a keyword for this kind of computer.
fn not_for_this_computer(package: &str) -> Explanation {
    Explanation::new(
        Severity::Problem,
        format!(
            "{} isn't available for your kind of computer (CPU architecture).",
            short(package)
        ),
    )
}

/// In Gentoo's package.mask, with its comment when there is one.
fn masked_on_purpose(package: &str, comment: Option<&str>) -> Explanation {
    let mut e = Explanation::new(
        Severity::Problem,
        format!(
            "Gentoo has blocked {} on purpose (it's \"masked\").",
            short(package)
        ),
    );
    if let Some(comment) = comment.filter(|c| !c.is_empty()) {
        e = e.detail(format!("Gentoo's reason: {comment}"));
    }
    e.fix(format!(
        "it's usually best not to use it. Remove {package} from your bunny.toml,\n\
         or look for a replacement."
    ))
}

fn masked(lines: &[&str]) -> Vec<Explanation> {
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("!!! All ebuilds that could satisfy \"") else {
            continue;
        };
        let package = base_name(rest.split('"').next().unwrap_or(rest));
        let following = &lines[i + 1..];
        // One line per version, and they can differ: a live `-9999` version
        // usually has no keyword at all while the releases are testing
        // versions. Explain the easiest way in: a testing version first,
        // then a deliberate mask, and "not for your computer" only when
        // that's true of every version.
        let reasons: Vec<String> = following
            .iter()
            .take_while(|l| !l.trim().is_empty())
            .filter_map(|l| l.split("(masked by: ").nth(1))
            .map(|r| r.trim_end_matches(')').to_string())
            .collect();
        let testing = |r: &&String| r.contains("keyword") && r.contains('~');
        let reason = reasons
            .iter()
            .find(testing)
            .or_else(|| reasons.iter().find(|r| !r.contains("missing keyword")))
            .or(reasons.first())
            .cloned()
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
            let keyword = reason
                .split_whitespace()
                .find(|w| w.starts_with('~'))
                .map(|w| w.trim_matches(|c: char| c == ')' || c == ','))
                .unwrap_or("~amd64")
                .to_string();
            // Same words as the helper's data gives, so the two agree.
            testing_version(&package, &keyword, &[])
        } else if reason.contains("missing keyword") {
            not_for_this_computer(&package)
        } else {
            masked_on_purpose(&package, Some(&comment.join(" ")))
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
                .fix(format!(
                    "run `{} dispatch-conf` to review them.",
                    crate::security::admin()
                ))
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
                format!("{DEPCLEAN_REMINDER}. bunny does that for you during `bunny switch`."),
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_explains_the_testing_version_when_only_the_live_one_has_no_keyword() {
        let text = "!!! All ebuilds that could satisfy \"gui-wm/dwl\" have been masked.\n\
                    !!! One of the following masked packages is required to complete your request:\n\
                    - gui-wm/dwl-9999::gentoo (masked by: missing keyword)\n\
                    - gui-wm/dwl-0.9::gentoo (masked by: ~amd64 keyword)\n\
                    - gui-wm/dwl-0.8::gentoo (masked by: ~amd64 keyword)\n\
                    \n\
                    For more information, see the MASKED PACKAGES section in the emerge\n";
        let found = explain(text);
        assert_eq!(found.len(), 1);
        let shown = found[0].render();
        assert!(
            shown.contains("only available as a testing version"),
            "{shown}"
        );
        assert!(shown.contains("keywords = [\"~amd64\"]"), "{shown}");
        // Only the live version, with no keyword: not for this computer.
        let live = "!!! All ebuilds that could satisfy \"x/y\" have been masked.\n\
                    - x/y-9999::gentoo (masked by: missing keyword)\n\n";
        assert!(explain(live)[0].render().contains("CPU architecture"));
    }

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
        let fix = found[0].fix.as_deref().unwrap();
        assert!(
            fix.contains("[pkg.\"dev-qt/qtbase\"]\nkeywords = [\"~amd64\"]"),
            "{fix}"
        );
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
    fn repeated_notices_are_shown_once() {
        let news = fixture("news-and-depclean.txt");
        let found = explain(&format!("{news}\nlots of output\n{news}"));
        assert_eq!(found.len(), 2, "{found:#?}");
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

    #[test]
    fn a_suggestion_that_breaks_the_rules_is_explained_in_words() {
        let found = explain(&fixture("required-use-mpv.txt"));
        let conflict = found
            .iter()
            .find(|e| e.title.contains("couldn't find a set of features"))
            .expect("conflict explained");
        let text = conflict.render();
        assert!(text.contains("works for mpv."), "{text}");
        assert!(
            text.contains("Its suggested fix (turning X off) breaks mpv's own rules"),
            "{text}"
        );
        assert!(text.contains("• at least one of: cli, libmpv"), "{text}");
        assert!(
            text.contains("• if egl is on: at least one of: X, drm, wayland"),
            "{text}"
        );
        assert!(text.contains("• if xv is on: X"), "{text}");
        assert!(text.contains("use = [\"wayland\"]"), "{text}");
        // No flag salad from the rule text.
        assert!(!found.iter().any(|e| e.render().contains("\"violates\"")));
        // The other change was part of the same failed attempt: folded in,
        // not offered as a fix of its own.
        assert_eq!(
            found
                .iter()
                .filter(|e| e.severity == Severity::Problem)
                .count(),
            1
        );
        assert!(
            text.contains(
                "skip it too: mpv needs vulkan-loader built with these features turned on: X"
            ),
            "{text}"
        );
    }

    #[test]
    fn required_use_in_words() {
        assert_eq!(
            required_use_rules("^^ ( a b ) !c? ( d ) ( e f ) || ( g )"),
            [
                "exactly one of: a, b",
                "if c is off: d",
                "all of: e, f",
                "at least one of: g"
            ]
        );
    }

    #[test]
    fn broken_flag_rules_are_found_through_color_codes_and_spacing() {
        let plain = fixture("required-use-mpv.txt");
        let colored = plain.replace(
            ">=media-video/mpv-0.41.0-r2 -X, this change",
            ">=media-video/mpv-0.41.0-r2 \x1b[31;01m-X\x1b[39;49;00m,  this change",
        );
        for text in [plain, colored] {
            let found = explain(&text);
            let text = found
                .iter()
                .map(Explanation::render)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                text.contains("couldn't find a set of features that works for mpv"),
                "{text}"
            );
            assert!(text.contains("(turning X off)"), "{text}");
            assert!(!text.contains("\"violates\""), "{text}");
        }
    }

    #[test]
    fn broken_flag_rules_in_the_change_use_form() {
        let found = explain(&fixture("required-use-mpv-change-use.txt"));
        assert_eq!(
            found
                .iter()
                .filter(|e| e.severity == Severity::Problem)
                .count(),
            1,
            "{found:?}"
        );
        let text = found[0].render();
        assert!(
            text.contains("couldn't find a set of features that works for mpv"),
            "{text}"
        );
        assert!(text.contains("(turning X off)"), "{text}");
        assert!(text.contains("• if xv is on: X"), "{text}");
        assert!(
            text.contains("skip it too: mpv needs vulkan-loader"),
            "{text}"
        );
        assert!(!text.contains("\"violates\""), "{text}");
    }

    #[test]
    fn the_same_broken_attempt_in_both_forms_is_one_problem() {
        let both = format!(
            "{}\n{}",
            fixture("required-use-mpv.txt"),
            fixture("required-use-mpv-change-use.txt")
        );
        let found = explain(&both);
        let problems: Vec<_> = found
            .iter()
            .filter(|e| e.severity == Severity::Problem)
            .collect();
        assert_eq!(problems.len(), 1, "{found:?}");
        let text = problems[0].render();
        assert_eq!(text.matches("vulkan-loader").count(), 1, "{text}");
    }

    // From the Portage helper's data (`bunny-portage.py plan`, helper 3).

    use crate::helper::Problem;

    fn keywords(name: &str, version: &str, keyword: &str, by: &[&str]) -> Problem {
        Problem::Keywords {
            name: name.into(),
            version: version.into(),
            keyword: keyword.into(),
            required_by: by.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn data_testing_version_matches_the_text_explanation() {
        let data = explain_with_problems(
            "",
            &[keywords(
                "dev-qt/qtbase",
                "6.10.1",
                "~amd64",
                &["gui-apps/quickshell"],
            )],
        );
        let text = explain(&fixture("keyword-testing.txt"));
        assert_eq!(data, text);
        assert!(data[0]
            .fix
            .as_deref()
            .unwrap()
            .contains("[pkg.\"dev-qt/qtbase\"]\nkeywords = [\"~amd64\"]"));
    }

    #[test]
    fn data_live_version_is_a_warning() {
        let found = explain_with_problems(
            "",
            &[keywords(
                "dev-libs/openssl",
                "9999",
                "**",
                &["net-misc/curl"],
            )],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(found[0]
            .title
            .contains("live development version of openssl"));
        assert!(found[0].title.contains("=dev-libs/openssl-9999 **"));
        assert!(found[0].details.iter().any(|d| d.contains("curl")));
    }

    #[test]
    fn data_replaces_the_text_for_its_kind_only() {
        // The text has a keyword change and a slot conflict; the data has
        // the keyword change. It's explained once, from the data, and the
        // slot conflict still comes from the text.
        let text = format!(
            "{}\n\n{}",
            fixture("keyword-testing.txt"),
            fixture("slot-conflict.txt")
        );
        let found =
            explain_with_problems(&text, &[keywords("dev-qt/qtbase", "6.10.1", "~amd64", &[])]);
        let testing: Vec<_> = found
            .iter()
            .filter(|e| e.title.contains("testing version"))
            .collect();
        assert_eq!(testing.len(), 1, "{found:#?}");
        // From the data: no "needed because of" (the data said nothing).
        assert!(testing[0].details.iter().all(|d| !d.contains("because")));
        let from_text = explain(&fixture("slot-conflict.txt"));
        assert!(!from_text.is_empty());
        for e in from_text {
            assert!(found.contains(&e), "{e:#?}");
        }
    }

    #[test]
    fn without_data_the_text_is_used() {
        for name in ["keyword-testing.txt", "masked.txt", "use-change-swtpm.txt"] {
            let text = fixture(name);
            assert_eq!(explain_with_problems(&text, &[]), explain(&text), "{name}");
        }
    }

    #[test]
    fn data_masked_package() {
        let masked = |reasons: &[&str], comment: Option<&str>| Problem::Masked {
            name: "www-client/oldbrowser".into(),
            reasons: reasons.iter().map(|s| s.to_string()).collect(),
            comment: comment.map(String::from),
            required_by: vec![],
        };
        // On purpose, with Gentoo's reason: the same as from the text.
        let data = explain_with_problems(
            "",
            &[masked(
                &["package.mask"],
                Some("Unmaintained, many open security bugs. Removal on 2026-11-30."),
            )],
        );
        assert_eq!(data, explain(&fixture("masked.txt")));

        let testing = explain_with_problems("", &[masked(&["~amd64 keyword"], None)]);
        assert!(testing[0].title.contains("testing version (~amd64)"));
        let cpu = explain_with_problems("", &[masked(&["missing keyword"], None)]);
        assert!(cpu[0].title.contains("kind of computer"));
        let other = explain_with_problems("", &[masked(&["EAPI 99"], None)]);
        assert!(other[0].title.contains("masked (EAPI 99)"), "{other:#?}");
    }

    #[test]
    fn data_use_change_matches_the_text_explanation() {
        let data = explain_with_problems(
            "",
            &[Problem::Use {
                name: "net-libs/gnutls".into(),
                version: "3.8.13".into(),
                changes: [("pkcs11".to_string(), true), ("tools".to_string(), true)]
                    .into_iter()
                    .collect(),
                required_by: vec!["app-crypt/swtpm".into()],
            }],
        );
        assert_eq!(data, explain(&fixture("use-change-swtpm.txt")));

        let off = explain_with_problems(
            "",
            &[Problem::Use {
                name: "media-video/mpv".into(),
                version: "0.40.0".into(),
                changes: [("lua".to_string(), false)].into_iter().collect(),
                required_by: vec![],
            }],
        );
        assert!(off[0].title.contains("turned off: lua"), "{off:#?}");
        assert!(off[0].fix.as_deref().unwrap().contains("use = [\"-lua\"]"));
    }

    #[test]
    fn a_flag_rules_conflict_in_the_text_wins_over_use_data() {
        // emerge's own USE suggestion breaks mpv's rules: only the text says
        // that, so its explanation is kept and the data's plain change isn't
        // shown on its own.
        let text = fixture("required-use-mpv.txt");
        let found = explain_with_problems(
            &text,
            &[Problem::Use {
                name: "media-video/mpv".into(),
                version: "0.40.0".into(),
                changes: [("lua".to_string(), true)].into_iter().collect(),
                required_by: vec![],
            }],
        );
        assert_eq!(found, explain(&text));
    }
}
