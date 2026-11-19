//! Installing and removing packages quietly: emerge's dry run, the
//! summary table, the progress line, and explaining what went wrong.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::package_use::restore_files;
use super::{cmd, Cancelled, Options, Outcome, Refused, Runner};
use crate::emerge::Merge;
use crate::explain;
use crate::plan::Plan;

/// Asks emerge what it would install (changing nothing), shows that as a
/// short list, asks if it's more than the plan showed, then installs with a
/// progress line. The full output goes to /var/log/bunny.
pub(super) fn install_quietly(
    root: &Path,
    plan: &Plan,
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    let update = ["emerge", "--update", "--deep", "--newuse", "--color=n"];
    let (list, dry_run_log) = ask_portage(root, runner, opts, outcome, &update)?;
    if list.is_empty() {
        println!("emerge has nothing to install.");
        return Ok(());
    }
    show_and_confirm(root, plan, &list, runner, opts, outcome)?;
    run_install(
        root,
        plan,
        &list,
        runner,
        opts,
        outcome,
        &update,
        &dry_run_log,
    )
}

/// What emerge would install, from bunny's Portage helper or, when that
/// isn't available, from `emerge --pretend`. If emerge refuses, the files
/// are put back and the refusal is explained. Also returns the text kept
/// for the log.
fn ask_portage(
    root: &Path,
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
    update: &[&str],
) -> Result<(Vec<Merge>, String)> {
    println!("\nAsking Portage what it needs to do…");
    let mut pretend = cmd(update);
    pretend.insert(1, "--pretend".into());
    pretend.push("@world".into());
    // Portage's own answer, as data. emerge's screen text only when that
    // isn't available (and to explain a refusal).
    Ok(match crate::helper::plan(runner) {
        Some(list) => (
            list,
            "(the install list came from bunny's Portage helper)\n".to_string(),
        ),
        None => {
            let (ok, output) = runner.run_quiet(&pretend, &mut |_| {})?;
            // --color=n should mean no color codes, but strip any that slip through:
            // an invisible code breaks reading the message.
            let raw = output;
            let output = explain::strip_ansi(&raw);
            if !ok {
                discard(root, runner, outcome)?;
                let log = save_log(
                    root,
                    &opts.stamp,
                    "emerge",
                    &format!("$ {}\n{raw}", pretend.join(" ")),
                );
                let text = refusal_message(&output, log);
                return Err(Refused(text).into());
            }

            let list = crate::emerge::parse_merge_list(&output);
            (list, format!("$ {}\n{output}", pretend.join(" ")))
        }
    })
}

/// Explains why emerge refused before installing anything.
fn refusal_message(output: &str, log: Option<PathBuf>) -> String {
    let explained = explain::explain(output);
    let mut text = String::from(
        "emerge found a problem before installing anything, so nothing was changed.\n",
    );
    if explained.is_empty() {
        text.push_str("\nbunny doesn't recognise emerge's message yet. The end of it:\n\n");
        text.push_str(&crate::emerge::tail(output, 20));
        text.push_str(
            "\n\nPaste the full message into `bunny explain`, or run with --verbose to see it all.\n\
             If you share it, bunny can learn to explain it.",
        );
    } else {
        text.push_str("\nHere's what happened:\n\n");
        for e in &explained {
            text.push_str(&e.render());
            text.push('\n');
        }
        text.push_str("Fix the problem, then run the same bunny command again.");
    }
    if let Some(log) = log {
        text.push_str(&format!("\nemerge's full output is in {}.", log.display()));
    }
    text
}

/// Shows the install list (and prebuilt tips), and asks again only if
/// emerge wants more than the plan showed. Saying no puts everything back.
fn show_and_confirm(
    root: &Path,
    plan: &Plan,
    list: &[Merge],
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    print!(
        "\n{}",
        crate::emerge::render_merge_list(list, &plan.add_packages)
    );
    // A big compile when a prebuilt version exists: say so before it starts.
    let available: std::collections::BTreeSet<String> =
        crate::repo::all(root).into_iter().map(|p| p.name).collect();
    let alternatives = crate::emerge::prebuilt_alternatives(list, &plan.add_packages, &available);
    for (package, bin) in &alternatives {
        println!(
            "  Tip: {bin} is a prebuilt version of {package}.\n  \
             It installs in a minute instead of compiling. To use it,\n  \
             answer n here, then: bunny add {bin}"
        );
    }
    // Only ask again if emerge wants more than the plan showed (dependencies,
    // rebuilds, updates). Same packages as the plan: the user already said yes.
    let planned = |name: &String| {
        plan.add_packages.contains(name) || plan.use_changes.iter().any(|c| &c.atom == name)
    };
    let more_than_planned = list.iter().any(|m| !planned(&m.name));
    if !opts.yes
        && (more_than_planned || !alternatives.is_empty())
        && !(opts.confirm)("\nInstall these?", true)?
    {
        discard(root, runner, outcome)?;
        return Err(Cancelled.into());
    }
    Ok(())
}

/// The real install, with one progress line. Saves emerge's full output to
/// the log, and explains a failure or a Ctrl+C.
#[allow(clippy::too_many_arguments)]
fn run_install(
    root: &Path,
    plan: &Plan,
    list: &[Merge],
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
    update: &[&str],
    dry_run_log: &str,
) -> Result<()> {
    let mut install = cmd(update);
    install.push("--quiet-build=y".into());
    install.push("@world".into());
    // How long each package took last time, for the progress line.
    let usually = std::fs::read_to_string(root.join("var/log/emerge.log"))
        .map(|log| crate::emerge::build_times(&log))
        .unwrap_or_default();
    let mut progress = crate::progress::Ticker::new();
    // Which package emerge was on, for explaining a Ctrl+C.
    let mut reached: Option<(usize, usize, String)> = None;
    let mut on_line = |line: &str| {
        if let Some((n, total, name, version, binary)) = crate::emerge::parse_progress(line) {
            reached = Some((n, total, name.clone()));
            let verb = if binary { "Installing" } else { "Compiling" };
            let build_log = (!binary).then(|| crate::progress::build_log(root, &name, &version));
            progress.step(
                &format!("[{n}/{total}] {verb} {name} {version}"),
                build_log,
                (!binary).then(|| usually.get(&name).copied()).flatten(),
            );
        }
    };
    println!();
    let (ok, install_output) = {
        // Ctrl+C stops emerge; bunny stays to explain where that leaves things.
        let _careful = crate::interrupt::Careful::start();
        runner.run_quiet(&install, &mut on_line)?
    };
    let install_output = explain::strip_ansi(&install_output);
    progress.finish();
    if !ok && crate::interrupt::happened() {
        outcome.log = save_log(
            root,
            &opts.stamp,
            "emerge",
            &format!("{dry_run_log}\n$ {}\n{install_output}", install.join(" ")),
        );
        bail!(
            "{}",
            interrupted_message(reached.as_ref(), &plan.add_packages, outcome)
        );
    }
    let log_text = format!("{dry_run_log}\n$ {}\n{install_output}", install.join(" "));
    outcome.log = save_log(root, &opts.stamp, "emerge", &log_text);

    let explained = explain::explain(&install_output);
    if !ok {
        let mut text = emerge_failed(&explained, outcome);
        if explained.is_empty() {
            text = text.replace(
                "bunny doesn't recognise emerge's message yet (it's shown above).",
                &format!(
                    "bunny doesn't recognise emerge's message yet. The end of it:\n\n{}\n",
                    crate::emerge::tail(&install_output, 20)
                ),
            );
        }
        if let Some(log) = &outcome.log {
            text.push_str(&format!("\nemerge's full output is in {}.", log.display()));
        }
        bail!("{text}");
    }
    println!(
        "  ✓ Installed {} package{}.",
        list.len(),
        if list.len() == 1 { "" } else { "s" }
    );
    outcome.messages = crate::emerge::parse_messages(&install_output);
    outcome
        .tips
        .extend(explained.iter().filter(|e| is_tip(e)).map(|e| e.render()));
    Ok(())
}

/// Shows which packages aren't needed anymore and asks before removing them
/// (always, even with --yes). Returns false if they weren't removed.
pub(super) fn remove_quietly(
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &Outcome,
) -> Result<bool> {
    let (ok, output) = runner.run_quiet(
        &cmd(&["emerge", "--depclean", "--pretend", "--color=n"]),
        &mut |_| {},
    )?;
    if !ok {
        return Ok(false);
    }
    let list = crate::emerge::parse_depclean(&output).unwrap_or_default();
    let count = crate::emerge::depclean_count(&output).unwrap_or(0);
    if list.len() != count {
        // emerge's output changed shape: let emerge show its own list and ask.
        return runner.run(&cmd(&["emerge", "--depclean", "--ask"]));
    }
    if list.is_empty() {
        return Ok(true);
    }
    println!("\nThese packages aren't needed anymore (nothing you asked for uses them):");
    let split: Vec<(&str, &str)> = list
        .iter()
        .map(|cpv| match crate::import::strip_version(cpv) {
            Some(name) => (name, &cpv[name.len() + 1..]),
            None => (cpv.as_str(), ""),
        })
        .collect();
    let width = split.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, version) in split {
        println!("  - {name:<width$}  {version}");
    }
    let undoable = outcome.snapshot.is_some();
    if !(opts.confirm)("Remove them?", undoable)? {
        return Ok(false);
    }
    let mut progress = crate::progress::Ticker::new();
    let mut on_line = |line: &str| {
        if let Some((n, total, name)) = crate::emerge::parse_unmerge_progress(line) {
            progress.step(&format!("[{n}/{total}] Removing {name}"), None, None);
        }
    };
    let (ok, _) = runner.run_quiet(&cmd(&["emerge", "--depclean", "--color=n"]), &mut on_line)?;
    progress.finish();
    if ok {
        println!(
            "  ✓ Removed {} package{}.",
            list.len(),
            if list.len() == 1 { "" } else { "s" }
        );
    }
    Ok(ok)
}

/// What to tell someone who pressed Ctrl+C while emerge was installing.
/// `reached` is (the package emerge was on, of how many, its name).
pub(super) fn interrupted_message(
    reached: Option<&(usize, usize, String)>,
    asked: &[String],
    outcome: &Outcome,
) -> String {
    let mut text = String::from("Stopped with Ctrl+C while installing.\n");
    match reached {
        Some((n, total, name)) => {
            let done = n.saturating_sub(1);
            text.push_str(&format!(
                "  {done} of {total} package{} finished installing.\n  \
                 {name} was in progress and didn't.\n",
                if *total == 1 { "" } else { "s" }
            ));
        }
        None => text.push_str("  emerge hadn't started installing anything yet.\n"),
    }
    if !asked.is_empty() {
        text.push_str(&format!(
            "  Your config still lists {}, so `bunny status`\n  will show it as not applied yet.\n",
            asked.join(", ")
        ));
    }
    text.push_str("\nTwo ways on from here:\n");
    if let Some(snapshot) = &outcome.snapshot {
        text.push_str(&format!(
            "  • Undo the whole attempt: `bunny rollback`, then restart. It goes back to\n    \
             {} (the system just before this).\n",
            snapshot.display()
        ));
    }
    text.push_str("  • Finish it later: `bunny switch` picks up where this stopped.");
    if let Some(log) = &outcome.log {
        text.push_str(&format!(
            "\nemerge's output so far is in {}.",
            log.display()
        ));
    }
    text
}

/// What to tell the user when emerge stops: bunny's explanation if it
/// recognised the message, and where their old system is.
pub(super) fn emerge_failed(explained: &[explain::Explanation], outcome: &Outcome) -> String {
    let mut text = String::from("emerge stopped, so the changes weren't installed.\n");
    if explained.is_empty() {
        text.push_str(
            "\nbunny doesn't recognise emerge's message yet (it's shown above).\n\
             If you share it, bunny can learn to explain it.\n",
        );
    } else {
        text.push_str("\nHere's what happened:\n\n");
        for e in explained {
            text.push_str(&e.render());
            text.push('\n');
        }
    }
    text.push_str(&format!(
        "Nothing is lost: your old files are backed up in {}.",
        outcome.backup_dir.display()
    ));
    if let Some(snapshot) = &outcome.snapshot {
        text.push_str(&format!(
            "\nThe system before this change is saved in {} (see `bunny rollback list`).",
            snapshot.display()
        ));
    }
    text.push_str("\nFix the problem, then run the same bunny command again.");
    text
}

/// emerge's notices worth passing on after a switch. Its reminder to remove
/// unused packages isn't: bunny just did that.
pub(super) fn is_tip(e: &explain::Explanation) -> bool {
    e.severity != explain::Severity::Problem && !e.title.starts_with(explain::DEPCLEAN_REMINDER)
}

/// Nothing was installed after all (emerge refused, or the user said no):
/// put the files back and delete the snapshot taken for this switch, so it
/// doesn't show up as a generation that changed nothing.
pub(super) fn discard(root: &Path, runner: &mut dyn Runner, outcome: &mut Outcome) -> Result<()> {
    restore_files(root, &outcome.backup_dir)?;
    if let Some(snapshot) = outcome.snapshot.take() {
        let (ok, _) = runner.run_quiet(
            &cmd(&["btrfs", "subvolume", "delete", &snapshot.to_string_lossy()]),
            &mut |_| {},
        )?;
        if ok {
            let mut note = snapshot.into_os_string();
            note.push(".toml");
            let _ = std::fs::remove_file(note);
        }
    }
    Ok(())
}

/// Saves emerge's full output, so nothing is lost by not showing it.
pub(super) fn save_log(root: &Path, stamp: &str, what: &str, text: &str) -> Option<PathBuf> {
    let dir = root.join("var/log/bunny");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{stamp}-{what}.log"));
    std::fs::write(&path, text).ok()?;
    Some(path)
}
