//! `bunny switch` and `bunny update`, and what they print afterwards.

use std::path::Path;

use anyhow::Context;
use std::process::ExitCode;

use super::{ask, confirm, elevate, report_problems, runner_for, timestamp, Default};
use crate::config::Config;
use crate::{applied, apply, lock, plan, VERBOSE};

/// How a switch ended, so `add` and `remove` know whether to keep their edit.
#[derive(Debug, PartialEq)]
pub(crate) enum Switched {
    /// Changes were applied (or at least started).
    Applied,
    /// The system already matched.
    NothingToDo,
    /// Stopped before changing anything (problems found, or cancelled).
    NotApplied(ExitCode),
}

pub(crate) fn switch(config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    Ok(match switch_inner(config, yes, root, false)? {
        Switched::Applied | Switched::NothingToDo => ExitCode::SUCCESS,
        Switched::NotApplied(code) => code,
    })
}

/// The switch itself, for a caller that's already root.
pub(crate) fn switch_inner(
    config: &Path,
    yes: bool,
    root: &Path,
    removal_requested: bool,
) -> anyhow::Result<Switched> {
    switch_with(config, yes, root, removal_requested, 0)
}

/// How many times a switch may add the resolver's settings and try again.
/// One normally does it; a second covers what only shows up once the
/// first set is in (the resolver works from the config as it was).
const RESOLVE_TRIES: u8 = 2;

fn switch_with(
    config: &Path,
    yes: bool,
    root: &Path,
    removal_requested: bool,
    tries: u8,
) -> anyhow::Result<Switched> {
    let real_system = root == Path::new("/");
    let changing = super::changing(real_system, false)?;

    let cfg = Config::load(config).map_err(apply::nothing_changed)?;
    if report_problems(&cfg, config) != ExitCode::SUCCESS {
        eprintln!("Fix these first, then run `bunny switch` again.");
        return Ok(Switched::NotApplied(super::your_input()));
    }
    let mut plan = plan::make(&cfg, root).map_err(apply::nothing_changed)?;
    plan.removal_requested = removal_requested;
    print!("{}", plan.render());
    if !plan.unknown_packages.is_empty() {
        eprintln!(
            "Nothing was changed. Fix the package names in {}, then run `bunny switch` again.",
            config.display()
        );
        return Ok(Switched::NotApplied(super::your_input()));
    }
    if !plan.security.problems.is_empty() {
        eprintln!(
            "Nothing was changed. Fix [security] in {}, then run `bunny switch` again.",
            config.display()
        );
        return Ok(Switched::NotApplied(ExitCode::FAILURE));
    }
    let lock_path = lock::path_for(config);
    if plan.is_empty() {
        for message in lock::ensure(root, &lock_path)? {
            println!("{message}");
        }
        keep_applied(root, config);
        return Ok(Switched::NothingToDo);
    }

    let foreign = apply::foreign_package_use_files(root);
    if !foreign.is_empty() {
        println!(
            "\nbunny will take over your USE flag settings. These package.use files will be\n\
             moved to a backup and replaced by one generated from your config:\n  {}\n\
             Lines bunny can't manage yet are kept as they are.",
            foreign.join(", ")
        );
    }
    if lock_path.exists() {
        println!(
            "\nPackages stay at the versions in {}. To move to newer versions, use `bunny update`.",
            lock_path.display()
        );
    }
    if !real_system {
        println!("\nThis is a test root, so commands are only printed, not run.");
    }

    if !yes
        && !confirm(
            "\nApply these changes?",
            // Enter only means yes when a snapshot makes this undoable.
            if root.join(".snapshots").is_dir() {
                Default::Yes
            } else {
                println!(
                    "\nNote: there's no /.snapshots folder, so this switch can't be rolled back."
                );
                Default::No
            },
        )?
    {
        println!("Nothing was changed.");
        return Ok(Switched::NotApplied(ExitCode::SUCCESS));
    }

    let opts = apply::Options {
        yes,
        stamp: timestamp(),
        update_all: false,
        color: std::io::IsTerminal::is_terminal(&std::io::stdout()),
        verbose: VERBOSE.load(std::sync::atomic::Ordering::Relaxed),
        confirm: ask,
    };
    let mut runner = runner_for(real_system);
    let (outcome, change) =
        match lock::switch(&cfg, root, &plan, &lock_path, runner.as_mut(), &opts) {
            Err(e) if e.is::<apply::Cancelled>() => {
                println!("Nothing was changed.");
                return Ok(Switched::NotApplied(ExitCode::SUCCESS));
            }
            Err(e) if e.is::<apply::Refused>() => {
                eprintln!("✗ {e}");
                return Ok(Switched::NotApplied(ExitCode::FAILURE));
            }
            Err(e) if e.is::<apply::NeedsChanges>() => {
                let Ok(apply::NeedsChanges(resolution)) = e.downcast::<apply::NeedsChanges>()
                else {
                    unreachable!("checked just above");
                };
                drop(changing);
                return add_resolution(config, yes, root, removal_requested, tries, &resolution);
            }
            result => result?,
        };

    println!("\n✓ This system now matches {}.", config.display());
    if let (true, Some(snapshot)) = (real_system, &outcome.snapshot) {
        println!(
            "  The system before this switch is saved in {}.",
            snapshot.display()
        );
    }
    println!(
        "  Old files are backed up in {}.",
        outcome.backup_dir.display()
    );
    for note in &outcome.notes {
        println!("{}", crate::progress::note(note));
    }
    report_lock_change(&change, false);
    print_messages(&outcome);
    print_tips(root, &outcome.tips);
    keep_applied(root, config);
    Ok(Switched::Applied)
}

/// Shows what the resolver found, and (if you agree) writes it into the
/// config and runs the switch again. Nothing was changed before this.
fn add_resolution(
    config: &Path,
    yes: bool,
    root: &Path,
    removal_requested: bool,
    tries: u8,
    resolution: &crate::helper::Resolution,
) -> anyhow::Result<Switched> {
    print!("\n{}", render_resolution(resolution));
    if !resolution.resolved {
        eprintln!(
            "\n✗ Even with these, emerge still can't work out a plan, so nothing was changed.\n  \
             The end of its message:\n\n{}\n\n  \
             You can add the settings above to {} yourself and run the switch again,\n  \
             or paste emerge's message into `bunny explain`.",
            indent(resolution.stuck.as_deref().unwrap_or("(no message)")),
            config.display()
        );
        return Ok(Switched::NotApplied(ExitCode::FAILURE));
    }
    if tries >= RESOLVE_TRIES {
        eprintln!(
            "\n✗ bunny already added settings once and emerge still wants more, so it stopped\n  \
             here (nothing was changed). Add the settings above to {} and run the switch again.",
            config.display()
        );
        return Ok(Switched::NotApplied(ExitCode::FAILURE));
    }
    if !yes
        && !confirm(
            &format!("\nAdd these to {} and continue?", config.display()),
            Default::Yes,
        )?
    {
        println!("Nothing was changed.");
        return Ok(Switched::NotApplied(ExitCode::SUCCESS));
    }
    let text = std::fs::read_to_string(config)
        .with_context(|| format!("Couldn't read {}", config.display()))?;
    let new = crate::edit::apply_resolution(&text, resolution)?;
    std::fs::write(config, &new).with_context(|| format!("Couldn't write {}", config.display()))?;
    println!("Added them to {}. Trying again…\n", config.display());
    match switch_with(config, yes, root, removal_requested, tries + 1)? {
        Switched::NotApplied(code) => {
            // Keep the config as it was if nothing came of it.
            std::fs::write(config, &text)
                .with_context(|| format!("Couldn't put {} back", config.display()))?;
            Ok(Switched::NotApplied(code))
        }
        done => Ok(done),
    }
}

/// The resolver's settings, grouped, each with why in a few words.
pub(crate) fn render_resolution(r: &crate::helper::Resolution) -> String {
    let why = |atom: &str| r.why.get(atom).cloned().unwrap_or_default();
    let mut out = String::from(
        "emerge needs these settings to install what you asked for. bunny worked them\n\
         out the way you would by hand:\n",
    );
    let section = |title: &str, items: Vec<(String, String, String)>, out: &mut String| {
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("\n  {title}\n"));
        let w0 = items.iter().map(|i| i.0.len()).max().unwrap_or(0);
        let w1 = items.iter().map(|i| i.1.len()).max().unwrap_or(0);
        for (a, b, c) in items {
            let line = format!("    {a:w0$}  {b:w1$}  {c}");
            out.push_str(line.trim_end());
            out.push('\n');
        }
    };
    let flags: Vec<_> = r
        .use_flags
        .iter()
        .map(|(atom, flags)| {
            let written: Vec<String> = flags
                .iter()
                .map(|(f, on)| if *on { f.clone() } else { format!("-{f}") })
                .collect();
            (atom.clone(), written.join(" "), why(atom))
        })
        .collect();
    section("USE flags", flags, &mut out);
    let testing: Vec<_> = r
        .keywords
        .iter()
        .map(|(atom, k)| (atom.clone(), k.clone(), why(atom)))
        .collect();
    section("Testing versions", testing, &mut out);
    let compile: Vec<_> = r
        .compile
        .iter()
        .map(|atom| (atom.clone(), "prebuilt = false".to_string(), why(atom)))
        .collect();
    section("Compiled instead of downloaded", compile, &mut out);
    out
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keeps a copy of the config that now matches the system, so undoing a
/// later change can put the config back too.
pub(crate) fn keep_applied(root: &Path, config: &Path) {
    if let Err(e) = applied::save(root, config) {
        eprintln!(
            "  Note: couldn't keep a copy of the config ({e:#}). Undoing a later change\n  \
             will restore the system, but not bunny.toml."
        );
    }
}

/// Messages packages left after installing, a few lines each, with the rest
/// in the log.
pub(crate) fn print_messages(outcome: &apply::Outcome) {
    if outcome.messages.is_empty() {
        if let Some(log) = &outcome.log {
            println!("  emerge's full output is in {}.", log.display());
        }
        return;
    }
    const SHOWN: usize = 4;
    println!("\nMessages from installed packages:");
    for (name, lines) in &outcome.messages {
        println!("  {name}");
        for line in lines.iter().take(SHOWN) {
            println!("    {line}");
        }
        if lines.len() > SHOWN {
            println!("    … {} more line(s)", lines.len() - SHOWN);
        }
    }
    if let Some(log) = &outcome.log {
        println!(
            "  The full messages and emerge's output are in {}.",
            log.display()
        );
    }
}

pub(crate) fn print_tips(root: &Path, tips: &[String]) {
    let tips = fresh_tips(root, tips);
    if tips.is_empty() {
        return;
    }
    println!("\nFrom emerge:");
    for tip in tips {
        for line in tip.lines() {
            println!("  {line}");
        }
    }
}

/// The unread-news tip repeats on every switch until the news is read. Show
/// it when the number of unread items changes, not every time, and once per
/// switch: emerge says it at the start and the end of each run (and a switch
/// can run emerge more than once), so only the last count is kept. Other
/// tips said twice are shown once too.
pub(crate) fn fresh_tips(root: &Path, tips: &[String]) -> Vec<String> {
    let is_news = |tip: &String| {
        let first = tip.lines().next().unwrap_or("");
        first.contains("announcements you haven't read")
            || first.contains("announcement you haven't read")
    };
    let last_news = tips.iter().rposition(is_news);
    let seen_file = root.join("var/lib/bunny/news-seen");
    let seen = std::fs::read_to_string(&seen_file).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for (i, tip) in tips.iter().enumerate() {
        if is_news(tip) {
            if Some(i) != last_news {
                continue;
            }
            let first = tip.lines().next().unwrap_or("");
            if seen.trim() == first.trim() {
                continue;
            }
            let _ = std::fs::create_dir_all(root.join("var/lib/bunny"));
            let _ = std::fs::write(&seen_file, format!("{}\n", first.trim()));
        }
        if !out.contains(tip) {
            out.push(tip.clone());
        }
    }
    out
}

/// Prints what changed in bunny.lock: every package when `list` (after an
/// update) or with `--verbose`, otherwise a count.
pub(crate) fn report_lock_change(change: &lock::LockChange, list: bool) {
    let verbose = VERBOSE.load(std::sync::atomic::Ordering::Relaxed);
    print!("{}", lock_change_text(change, list || verbose));
}

/// What changed in bunny.lock: a one-line count, or (`list`) every
/// package. A desktop install adds well over a hundred, and they're all in
/// the lock file anyway.
pub(crate) fn lock_change_text(change: &lock::LockChange, list: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let path = change.path.display();
    if change.created {
        let _ = writeln!(out, "  Created {path}: the exact versions installed now.");
        return out;
    }
    let d = &change.drift;
    if d.is_empty() {
        let _ = writeln!(out, "  {path} is unchanged.");
        return out;
    }
    if !list {
        let mut parts = Vec::new();
        if !d.extra.is_empty() {
            let n = d.extra.len();
            parts.push(format!("+ {n} package{}", if n == 1 { "" } else { "s" }));
        }
        if !d.missing.is_empty() {
            parts.push(format!("- {}", d.missing.len()));
        }
        if !d.changed.is_empty() {
            parts.push(format!("~ {} updated", d.changed.len()));
        }
        if !d.repos.is_empty() {
            let names: Vec<&str> = d.repos.iter().map(|(r, _, _)| r.as_str()).collect();
            parts.push(format!("newer package tree ({})", names.join(", ")));
        }
        let _ = writeln!(out, "  Updated {path}: {}", parts.join(", "));
        out.push_str("    The list is in the lock file; add --verbose to see it here.\n");
        return out;
    }
    let _ = writeln!(out, "  Updated {path}:");
    for (name, old, new) in &d.changed {
        let _ = writeln!(out, "    ~ {name}: {old} → {new}");
    }
    for (name, v) in &d.extra {
        let _ = writeln!(out, "    + {name} {v}");
    }
    for (name, v) in &d.missing {
        let _ = writeln!(out, "    - {name} {v}");
    }
    for (repo, old, new) in &d.repos {
        let _ = writeln!(out, "    ~ {repo} package tree: {old} → {new}");
    }
    out
}

pub(crate) fn update_cmd(config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let _changing = super::changing(real_system, false)?;
    let cfg = Config::load(config)?;
    if report_problems(&cfg, config) != ExitCode::SUCCESS {
        eprintln!("Fix these first, then run `bunny update` again.");
        return Ok(super::your_input());
    }
    println!(
        "\nThis will sync the Gentoo package tree, update everything to the newest\n\
         versions, and record them in {}.",
        lock::path_for(config).display()
    );
    let undoable = root.join(".snapshots").is_dir();
    if undoable {
        println!("A snapshot is taken first, so you can roll back if you don't like the result.");
    } else {
        println!("Note: there's no /.snapshots folder, so this update can't be rolled back.");
    }
    if !real_system {
        println!("\nThis is a test root, so commands are only printed, not run.");
    }
    let default = if undoable { Default::Yes } else { Default::No };
    if !yes && !confirm("\nUpdate now?", default)? {
        println!("Nothing was changed.");
        return Ok(ExitCode::SUCCESS);
    }
    let opts = apply::Options {
        yes,
        stamp: timestamp(),
        update_all: true,
        color: std::io::IsTerminal::is_terminal(&std::io::stdout()),
        verbose: VERBOSE.load(std::sync::atomic::Ordering::Relaxed),
        confirm: ask,
    };
    let mut runner = runner_for(real_system);
    let (outcome, change) =
        match lock::update(&cfg, root, &lock::path_for(config), runner.as_mut(), &opts) {
            Err(e) if e.is::<apply::Cancelled>() => {
                println!("Nothing was changed.");
                return Ok(ExitCode::SUCCESS);
            }
            Err(e) if e.is::<apply::Refused>() => {
                eprintln!("✗ {e}");
                return Ok(ExitCode::FAILURE);
            }
            result => result?,
        };

    println!("\n✓ Updated.");
    if let (true, Some(snapshot)) = (real_system, &outcome.snapshot) {
        println!(
            "  The system before this update is saved in {}.",
            snapshot.display()
        );
    }
    for note in &outcome.notes {
        println!("{}", crate::progress::note(note));
    }
    // An update's point is the new versions, so it lists them.
    report_lock_change(&change, true);
    print_messages(&outcome);
    print_tips(root, &outcome.tips);
    keep_applied(root, config);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::{fresh_tips, lock_change_text};
    use crate::lock;

    #[test]
    fn the_lock_summary_counts_unless_verbose() {
        let pkg = |n: usize| (format!("app-misc/p{n}"), "1.0".to_string());
        let change = lock::LockChange {
            path: "/etc/bunny/bunny.lock".into(),
            created: false,
            drift: lock::Drift {
                extra: (0..140).map(pkg).collect(),
                missing: vec![pkg(900), pkg(901)],
                changed: vec![("app-misc/tmux".into(), "3.4".into(), "3.5".into())],
                repos: vec![],
            },
        };
        let short = lock_change_text(&change, false);
        assert_eq!(
            short,
            "  Updated /etc/bunny/bunny.lock: + 140 packages, - 2, ~ 1 updated\n    \
             The list is in the lock file; add --verbose to see it here.\n"
        );
        let long = lock_change_text(&change, true);
        assert_eq!(long.lines().count(), 1 + 140 + 2 + 1, "{long}");
        assert!(long.contains("    ~ app-misc/tmux: 3.4 → 3.5\n"));

        let one = lock::LockChange {
            drift: lock::Drift {
                extra: vec![pkg(1)],
                ..Default::default()
            },
            ..change
        };
        assert!(lock_change_text(&one, false).contains(": + 1 package\n"));
    }

    #[test]
    fn unread_news_tip_shows_when_the_count_changes() {
        let root = tempfile::tempdir().unwrap();
        let news = |n: u32| format!("ℹ Gentoo has {n} announcements you haven't read.\n  Tip: …");
        let other = "ℹ Something else".to_string();
        let tips = vec![news(26), other.clone()];
        assert_eq!(fresh_tips(root.path(), &tips).len(), 2);
        // Same count again: only the other tip.
        assert_eq!(fresh_tips(root.path(), &tips), vec![other.clone()]);
        // A new item arrived: shown again.
        assert_eq!(fresh_tips(root.path(), &[news(27)]).len(), 1);
    }

    #[test]
    fn unread_news_tip_shows_once_per_switch() {
        // From the Saturday test: "1 announcements", then "2 announcements"
        // (emerge says it before and after, and the sync brought one more).
        let root = tempfile::tempdir().unwrap();
        let one = "ℹ Gentoo has 1 announcement you haven't read.\n  Tip: …".to_string();
        let two = "ℹ Gentoo has 2 announcements you haven't read.\n  Tip: …".to_string();
        let other = "ℹ Something else".to_string();
        let tips = vec![one.clone(), other.clone(), two.clone(), other.clone()];
        assert_eq!(fresh_tips(root.path(), &tips), vec![other.clone(), two]);
        // The same count said twice in one run: once.
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            fresh_tips(root.path(), &[one.clone(), one.clone()]),
            vec![one]
        );
    }
}
