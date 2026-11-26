//! `bunny switch` and `bunny update`, and what they print afterwards.

use std::path::Path;
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
    let real_system = root == Path::new("/");
    let _changing = super::changing(real_system, false)?;

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
    report_lock_change(&change);
    print_messages(&outcome);
    print_tips(root, &outcome.tips);
    keep_applied(root, config);
    Ok(Switched::Applied)
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
/// it when the number of unread items changes, not every time.
pub(crate) fn fresh_tips(root: &Path, tips: &[String]) -> Vec<String> {
    let seen_file = root.join("var/lib/bunny/news-seen");
    let seen = std::fs::read_to_string(&seen_file).unwrap_or_default();
    let mut out = Vec::new();
    for tip in tips {
        let first = tip.lines().next().unwrap_or("");
        if first.contains("announcements you haven't read")
            || first.contains("announcement you haven't read")
        {
            if seen.trim() == first.trim() {
                continue;
            }
            let _ = std::fs::create_dir_all(root.join("var/lib/bunny"));
            let _ = std::fs::write(&seen_file, format!("{}\n", first.trim()));
        }
        out.push(tip.clone());
    }
    out
}

pub(crate) fn report_lock_change(change: &lock::LockChange) {
    if change.created {
        println!(
            "  Created {}: the exact versions installed now.",
            change.path.display()
        );
        return;
    }
    let d = &change.drift;
    if d.is_empty() {
        println!("  {} is unchanged.", change.path.display());
        return;
    }
    println!("  Updated {}:", change.path.display());
    for (name, old, new) in &d.changed {
        println!("    ~ {name}: {old} → {new}");
    }
    for (name, v) in &d.extra {
        println!("    + {name} {v}");
    }
    for (name, v) in &d.missing {
        println!("    - {name} {v}");
    }
    for (repo, old, new) in &d.repos {
        println!("    ~ {repo} package tree: {old} → {new}");
    }
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
    report_lock_change(&change);
    print_messages(&outcome);
    print_tips(root, &outcome.tips);
    keep_applied(root, config);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::fresh_tips;

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
}
