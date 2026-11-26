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
    // --ask=n: EMERGE_DEFAULT_OPTS may say --ask, but bunny asks itself and
    // emerge has no terminal here, so it would refuse.
    let update = [
        "emerge",
        "--update",
        "--deep",
        "--newuse",
        "--color=n",
        "--ask=n",
    ];
    let (list, dry_run_log) = ask_portage(root, runner, opts, outcome, &update)?;
    let left_out = left_out(root, plan, &list);
    if !left_out.is_empty() {
        return explain_left_out(root, &left_out, runner, opts, outcome);
    }
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
    // --verbose shows each package's USE flags, with the changed ones
    // marked, so a rebuild can say why it happens.
    pretend.push("--verbose".into());
    pretend.push("@world".into());
    // Portage's own answer, as data. emerge's screen text only when that
    // isn't available (and to explain a refusal).
    let problems = match crate::helper::plan(runner) {
        crate::helper::PlanAnswer::List(list) => {
            return Ok((
                list,
                "(the install list came from bunny's Portage helper)\n".to_string(),
            ))
        }
        crate::helper::PlanAnswer::Refused(problems) => problems,
        crate::helper::PlanAnswer::Unavailable => Vec::new(),
    };
    // emerge's text: the install list when the helper couldn't answer, and
    // the full message for the log (and for problems the helper doesn't
    // report as data yet) when Portage refused.
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
        let text = refusal_message(&output, &problems, log);
        return Err(Refused(text).into());
    }
    let list = crate::emerge::parse_merge_list(&output);
    Ok((list, format!("$ {}\n{output}", pretend.join(" "))))
}

/// Packages you added that emerge's plan doesn't install. Updating
/// `@world`, emerge skips a package it can't install (only a testing
/// version, masked) with a warning instead of failing, so the plan alone
/// would say there's nothing to do.
///
/// A package that's already installed isn't left out: it's new to the
/// world file only (pulled in before as a dependency, like `sys-apps/dbus`
/// for `[desktop]`), so emerge rightly has nothing to do for it.
fn left_out(root: &Path, plan: &Plan, list: &[Merge]) -> Vec<String> {
    plan.add_packages
        .iter()
        .filter(|p| !list.iter().any(|m| &m.name == *p))
        .filter(|p| crate::portage::is_installed(root, p) != Some(true))
        .cloned()
        .collect()
}

/// Asks emerge about the left-out packages by name, which fails with the
/// reason instead of skipping them, and explains it. Nothing is changed.
fn explain_left_out(
    root: &Path,
    left_out: &[String],
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    let mut pretend = cmd(&["emerge", "--pretend", "--verbose", "--color=n", "--ask=n"]);
    pretend.extend(left_out.iter().cloned());
    let (_ok, raw) = runner.run_quiet(&pretend, &mut |_| {})?;
    let output = explain::strip_ansi(&raw);
    discard(root, runner, outcome)?;
    let log = save_log(
        root,
        &opts.stamp,
        "emerge",
        &format!("$ {}\n{raw}", pretend.join(" ")),
    );
    let mut text = refusal_message(&output, &[], log);
    if explain::explain_with_problems(&output, &[]).is_empty() {
        text = format!(
            "emerge's plan leaves out {}, although your config asks for {}.\n{text}",
            left_out.join(", "),
            if left_out.len() == 1 { "it" } else { "them" },
        );
    }
    Err(Refused(text).into())
}

/// Explains why emerge refused before installing anything: from the Portage
/// helper's data where it has some, otherwise from emerge's text.
fn refusal_message(
    output: &str,
    problems: &[crate::helper::Problem],
    log: Option<PathBuf>,
) -> String {
    let explained = explain::explain_with_problems(output, problems);
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
            interrupted_message(
                reached.as_ref(),
                &plan.add_packages,
                outcome,
                if opts.update_all {
                    "bunny update"
                } else {
                    "bunny switch"
                },
            )
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
/// (even with --yes). Returns false if they weren't removed.
///
/// One exception, so a plain `bunny remove` asks once: when the user
/// already said yes to the plan (not `--yes`) and the list is exactly the
/// packages the plan showed as removed (`planned`), nothing more, bunny
/// doesn't ask again.
///
/// The list comes from bunny's Portage helper when it can answer; then
/// exactly the packages shown are removed (`emerge --depclean =cat/pkg-1.0
/// …`, which still keeps any that something needs). Otherwise from
/// `emerge --depclean --pretend`'s text, as before.
pub(super) fn remove_quietly(
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &Outcome,
    planned: &[String],
) -> Result<bool> {
    let (list, from_helper) = match crate::helper::depclean(runner) {
        Some(list) => (list, true),
        None => match depclean_from_emerge(runner)? {
            Removals::List(list) => (list, false),
            Removals::Failed => return Ok(false),
            // emerge's output changed shape: let emerge show its own list and ask.
            Removals::Unreadable => return runner.run(&cmd(&["emerge", "--depclean", "--ask"])),
        },
    };
    if list.is_empty() {
        return Ok(true);
    }
    print!("{}", describe_removals(&list));
    let undoable = outcome.snapshot.is_some();
    if !opts.yes && same_as_planned(&list, planned) {
        println!("  These are the packages the plan showed, so removing them now.");
    } else if !(opts.confirm)("Remove them?", undoable)? {
        return Ok(false);
    }
    let mut progress = crate::progress::Ticker::new();
    let mut on_line = |line: &str| {
        if let Some((n, total, name)) = crate::emerge::parse_unmerge_progress(line) {
            progress.step(&format!("[{n}/{total}] Removing {name}"), None, None);
        }
    };
    let mut remove = cmd(&["emerge", "--depclean", "--color=n", "--ask=n"]);
    if from_helper {
        // Only what was shown and agreed to. emerge checks again that
        // nothing needs them, so a difference between the helper and emerge
        // can only mean removing less, never more.
        remove.extend(list.iter().map(|r| format!("={}", r.cpv())));
    }
    let (ok, _) = runner.run_quiet(&remove, &mut on_line)?;
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

/// Whether emerge wants to remove exactly the packages the plan showed as
/// leaving the config (world entries, maybe with a `:slot`), and nothing
/// more.
pub(super) fn same_as_planned(list: &[crate::helper::Removal], planned: &[String]) -> bool {
    let mut removing: Vec<&str> = list.iter().map(|r| r.name.as_str()).collect();
    let mut shown: Vec<&str> = planned
        .iter()
        .map(|atom| atom.split(':').next().unwrap_or(atom))
        .collect();
    removing.sort_unstable();
    removing.dedup();
    shown.sort_unstable();
    shown.dedup();
    !removing.is_empty() && removing == shown
}

/// What `emerge --depclean --pretend` said it would remove.
enum Removals {
    List(Vec<crate::helper::Removal>),
    /// emerge failed (it explains on its own when run for real).
    Failed,
    /// The list and the count didn't match: emerge's wording changed.
    Unreadable,
}

/// The fallback when the helper can't answer: emerge's own text.
fn depclean_from_emerge(runner: &mut dyn Runner) -> Result<Removals> {
    let (ok, output) = runner.run_quiet(
        &cmd(&["emerge", "--depclean", "--pretend", "--color=n", "--ask=n"]),
        &mut |_| {},
    )?;
    if !ok {
        return Ok(Removals::Failed);
    }
    let list = crate::emerge::parse_depclean(&output).unwrap_or_default();
    let count = crate::emerge::depclean_count(&output).unwrap_or(0);
    if list.len() != count {
        return Ok(Removals::Unreadable);
    }
    Ok(Removals::List(
        list.iter()
            .map(|cpv| {
                let (name, version) = match crate::import::strip_version(cpv) {
                    Some(name) => (name, &cpv[name.len() + 1..]),
                    None => (cpv.as_str(), ""),
                };
                crate::helper::Removal {
                    name: name.to_string(),
                    version: version.to_string(),
                    needed_by: Vec::new(),
                }
            })
            .collect(),
    ))
}

/// The list shown before asking, one package a line, with why it goes when
/// Portage said (another package in the list was what needed it).
pub(super) fn describe_removals(list: &[crate::helper::Removal]) -> String {
    let mut out =
        String::from("\nThese packages aren't needed anymore (nothing you asked for uses them):\n");
    let name_width = list.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let version_width = list.iter().map(|r| r.version.len()).max().unwrap_or(0);
    for r in list {
        let line = if r.needed_by.is_empty() {
            format!("  - {:<name_width$}  {}", r.name, r.version)
        } else {
            format!(
                "  - {:<name_width$}  {:<version_width$}  (only {} used it)",
                r.name,
                r.version,
                r.needed_by.join(", ")
            )
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// What to tell someone who pressed Ctrl+C while emerge was installing.
/// `reached` is (the package emerge was on, of how many, its name).
pub(super) fn interrupted_message(
    reached: Option<&(usize, usize, String)>,
    asked: &[String],
    outcome: &Outcome,
    command: &str,
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
    text.push_str(&format!(
        "  • Finish it later: `{command}` picks up where this stopped."
    ));
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
    drop_snapshot(runner, outcome)
}

/// Deletes the snapshot taken for this switch (and its note), for a switch
/// that stopped before changing anything.
pub(super) fn drop_snapshot(runner: &mut dyn Runner, outcome: &mut Outcome) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(add: &[&str]) -> Plan {
        Plan {
            add_packages: add.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn an_installed_dependency_new_to_world_is_not_left_out() {
        // [desktop] adds sys-apps/dbus to the world file, but the base
        // system already has it, so emerge's @world plan doesn't list it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("var/db/pkg/sys-apps/dbus-1.16.2-r2")).unwrap();
        std::fs::create_dir_all(dir.path().join("var/db/pkg/sys-auth")).unwrap();
        let p = plan(&["sys-apps/dbus", "sys-auth/elogind", "gui-wm/mangowm"]);
        let list = vec![Merge {
            name: "gui-wm/mangowm".into(),
            version: "0.18.6".into(),
            from: None,
            action: crate::emerge::Action::New,
            binary: false,
            why: crate::emerge::Why::default(),
        }];
        assert_eq!(left_out(dir.path(), &p, &list), ["sys-auth/elogind"]);
    }
}
