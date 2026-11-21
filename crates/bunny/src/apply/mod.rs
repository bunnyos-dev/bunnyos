//! `bunny switch`: make a system match its config.
//!
//! Every change is backed up first, and commands go through a [`Runner`] so
//! tests (and `--root` on a fake system) can see exactly what would run
//! without running it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::explain;
use crate::init;
use crate::notes::{self, Kind, Note, State};
use crate::plan::Plan;
use crate::portage;

mod admin;
mod boot_menu;
mod install;
mod package_use;
mod repositories;
mod runner;
#[cfg(test)]
mod tests;

pub use boot_menu::save_kernels;
pub use package_use::foreign_package_use_files;
pub use runner::{PrintRunner, Runner, SystemRunner};

use admin::apply_security;
use boot_menu::{install_boot_module, write_undo_entry};
use install::{drop_snapshot, emerge_failed, install_quietly, is_tip, remove_quietly};
use package_use::{adopt_package_use, copy_tree, write_keywords, USE_BEFORE};
use repositories::set_up_repositories;

/// The user said no at a question, so the switch stopped and its file
/// changes were put back. Not an error to report, just "nothing changed".
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nothing was changed.")
    }
}

impl std::error::Error for Cancelled {}

/// Something failed before anything on the system was changed (the
/// snapshot, the startup image, reading the system). `bunny add`/`remove`
/// put the config back when they see it.
#[derive(Debug)]
pub struct NothingChanged(pub String);

impl std::fmt::Display for NothingChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NothingChanged {}

/// Marks an error as coming from before anything was changed.
pub fn nothing_changed(e: anyhow::Error) -> anyhow::Error {
    if e.is::<NothingChanged>() || e.is::<Cancelled>() || e.is::<Refused>() {
        return e;
    }
    anyhow::Error::new(NothingChanged(format!("{e:#}")))
}

/// Whether nothing on the system was changed when `e` happened.
pub fn changed_nothing(e: &anyhow::Error) -> bool {
    e.is::<NothingChanged>() || e.is::<Cancelled>() || e.is::<Refused>()
}

/// emerge refused before installing anything (its dry run failed), and the
/// files were put back, so nothing changed. Holds the explanation to show.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

#[derive(Clone)]
pub struct Options {
    /// Don't let emerge ask before installing. Removing packages always asks.
    pub yes: bool,
    /// Used to name the snapshot and the backup folder, e.g. `20261116-203000`.
    pub stamp: String,
    /// Run the world update even if the plan has no package changes
    /// (for `bunny update`).
    pub update_all: bool,
    /// Ask emerge for colors (its output is captured, so it can't tell it's
    /// talking to a terminal). Only when bunny's own output is a terminal.
    pub color: bool,
    /// Show emerge's full output live, and let emerge ask its own questions,
    /// instead of bunny's summary and progress line.
    pub verbose: bool,
    /// Asks the user a yes/no question; the bool is the answer Enter gives.
    pub confirm: fn(&str, bool) -> Result<bool>,
}

/// What happened, for the summary at the end.
#[derive(Debug, Default)]
pub struct Outcome {
    pub snapshot: Option<PathBuf>,
    pub backup_dir: PathBuf,
    pub notes: Vec<String>,
    /// emerge's notices (news, reminders), explained. Shown after the summary.
    pub tips: Vec<String>,
    /// Messages packages left after installing, by package.
    pub messages: Vec<(String, Vec<String>)>,
    /// Where emerge's full output was saved (quiet mode).
    pub log: Option<PathBuf>,
}

pub(super) fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Applies `plan` (made from `config`) to the system at `root`.
///
/// The steps run in order, each only once the one before worked. The
/// snapshot comes before anything that changes the system, so there's always
/// a way back.
pub fn switch(
    config: &Config,
    root: &Path,
    plan: &Plan,
    runner: &mut dyn Runner,
    opts: &Options,
) -> Result<Outcome> {
    let mut outcome = Outcome {
        backup_dir: root.join("var/lib/bunny/backups").join(&opts.stamp),
        ..Default::default()
    };

    // 0. The boot menu's undo step goes into the startup image *before* the
    //    snapshot, so this very generation can already be undone from the
    //    boot menu. It only adds a module and rebuilds the image (the old
    //    image is backed up); if that fails, nothing else has changed.
    if let Some(versions) = &plan.boot.install_module {
        install_boot_module(root, versions, runner, &outcome.backup_dir)
            .map_err(nothing_changed)?;
    }
    // 1. Snapshot first, so there's always a way back.
    take_snapshot(root, plan, runner, opts, &mut outcome).map_err(nothing_changed)?;
    std::fs::create_dir_all(&outcome.backup_dir)
        .with_context(|| format!("Couldn't create {}", outcome.backup_dir.display()))?;
    // 1b. Repositories, so packages from new ones can be checked and installed.
    //     If this stops the switch, nothing else changed yet: the snapshot
    //     goes too, so it isn't left as a generation that changed nothing.
    if let Err(err) = set_up_repositories(config, root, plan, runner, &mut outcome) {
        let _ = drop_snapshot(runner, &mut outcome);
        return Err(err);
    }
    // 2–3. package.use, package.accept_keywords and the world file, each
    //      backed up first.
    write_package_files(config, root, &outcome)?;
    // 4. Install and update.
    install_and_update(root, plan, runner, opts, &mut outcome)?;
    // 5. Admin rights. After the install (so the tool and its checker exist)
    //    and before depclean (so the old tool is only removed once the new
    //    one works).
    apply_security(plan, runner, &outcome)?;
    // 6. Remove what nothing needs any more. Always asks.
    remove_unused(plan, runner, opts, &mut outcome)?;
    // 7. Services.
    enable_services(config, root, plan, runner, &mut outcome)?;
    // 8. The boot menu's undo entry, pointing at this switch's snapshot.
    update_boot_menu(config, root, plan, opts, &mut outcome)?;

    notes::write_state(
        root,
        &State {
            last: Kind::Switch,
            at: opts.stamp.clone(),
            to: None,
        },
    )?;
    Ok(outcome)
}

/// Step 1: a read-only snapshot of `/` in `/.snapshots/bunny-<stamp>`, with
/// its note (and, when `/boot` is its own partition, a copy of the kernel).
/// Without `/.snapshots` the switch goes ahead, with a note that it can't be
/// undone.
fn take_snapshot(
    root: &Path,
    plan: &Plan,
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    let snapshots = root.join(".snapshots");
    if snapshots.is_dir() {
        let target = snapshots.join(format!("bunny-{}", opts.stamp));
        // btrfs's own "Create readonly snapshot…" line isn't shown; bunny
        // reports the snapshot itself at the end (and btrfs's words on failure).
        let (ok, said) = runner.run_quiet(
            &cmd(&[
                "btrfs",
                "subvolume",
                "snapshot",
                "-r",
                &root.to_string_lossy(),
                &target.to_string_lossy(),
            ]),
            &mut |_| {},
        )?;
        if !ok {
            bail!(
                "Couldn't take a snapshot before changing anything, so nothing was changed.\n  \
                 bunny needs / on btrfs with a /.snapshots folder to be able to undo changes.\n  \
                 btrfs said: {}",
                said.trim()
            );
        }
        outcome.snapshot = Some(target);
        // /boot on its own filesystem (an encrypted system) isn't in the
        // snapshot: keep this generation's kernel next to it on /boot.
        if !runner.dry() && crate::kernels::separate_boot(&crate::kernels::mountinfo(root)) {
            if let Some(note) = save_kernels(root, &format!("bunny-{}", opts.stamp)) {
                outcome.notes.push(note);
            }
        }
        notes::write_note(
            root,
            &format!("bunny-{}", opts.stamp),
            &Note {
                kind: Kind::Switch,
                summary: plan.summary(),
                to: None,
            },
        )?;
    } else {
        outcome.notes.push(
            "No /.snapshots folder, so no snapshot was taken. Undo would need the backups.".into(),
        );
    }
    Ok(())
}

/// Steps 2–3. USE flags: bunny takes over package.use. A full copy is kept
/// first, so a cancelled switch can put everything back exactly. Then the
/// world set, written from `packages`.
fn write_package_files(config: &Config, root: &Path, outcome: &Outcome) -> Result<()> {
    let use_dir = portage::package_use_dir(root);
    if use_dir.exists() {
        copy_tree(&use_dir, &outcome.backup_dir.join(USE_BEFORE))?;
    }
    adopt_package_use(config, root, &outcome.backup_dir)?;
    write_keywords(config, root, &outcome.backup_dir)?;

    // 3. World set.
    let world = portage::world_path(root);
    if world.exists() {
        std::fs::copy(&world, outcome.backup_dir.join("world"))
            .context("Couldn't back up the world file")?;
    } else if let Some(parent) = world.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&world, portage::render_world(config, root))
        .context("Couldn't write the world file")?;
    Ok(())
}

/// Step 4: emerge installs and updates, quietly (bunny's summary and
/// progress line) or, with `--verbose` or a test root, with its own output.
fn install_and_update(
    root: &Path,
    plan: &Plan,
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    let needs_emerge = opts.update_all
        || !plan.add_packages.is_empty()
        || !plan.use_changes.is_empty()
        || !plan.keyword_changes.is_empty();
    if needs_emerge && !opts.verbose && !runner.dry() {
        install_quietly(root, plan, runner, opts, outcome)?;
    } else if needs_emerge {
        let mut emerge = cmd(&["emerge", "--update", "--deep", "--newuse", "--verbose"]);
        if !opts.yes {
            emerge.push("--ask".into());
        }
        if opts.color {
            emerge.push("--color=y".into());
        }
        emerge.push("@world".into());
        let (ok, output) = runner.run_captured(&emerge)?;
        let explained = explain::explain(&explain::strip_ansi(&output));
        if !ok {
            bail!("{}", emerge_failed(&explained, outcome));
        }
        outcome
            .tips
            .extend(explained.iter().filter(|e| is_tip(e)).map(|e| e.render()));
    }
    Ok(())
}

/// Step 6: remove packages nothing needs anymore: ones taken out of the
/// config, and dependencies left over after USE changes or updates (sudo
/// built without `sendmail` no longer needs a mail server). Always asks.
fn remove_unused(
    plan: &Plan,
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    let cleanup =
        !plan.remove_packages.is_empty() || !plan.use_changes.is_empty() || opts.update_all;
    if cleanup {
        let removed = if opts.verbose || runner.dry() {
            runner.run(&cmd(&["emerge", "--depclean", "--ask"]))?
        } else {
            remove_quietly(runner, opts, outcome)?
        };
        if !removed {
            outcome.notes.push(format!(
                "Unused packages weren't removed. Run `{} emerge --depclean --ask` when you're ready.",
                crate::security::admin()
            ));
        }
    }
    Ok(())
}

/// Step 7: enable and start services. Checked again here: packages
/// installed above may have added them.
fn enable_services(
    config: &Config,
    root: &Path,
    plan: &Plan,
    runner: &mut dyn Runner,
    outcome: &mut Outcome,
) -> Result<()> {
    let init = init::for_config(config.system.init, root);
    for service in &plan.enable_services {
        if !init.service_exists(service) {
            outcome.notes.push(format!(
                "\"{service}\" isn't installed, so it wasn't enabled. \
                 Check which package provides it."
            ));
            continue;
        }
        if !runner.run(&cmd(&["rc-update", "add", service, "default"]))? {
            outcome
                .notes
                .push(format!("Couldn't enable \"{service}\" at boot."));
            continue;
        }
        // Start it now too, so it doesn't wait for the next reboot.
        if !runner.run(&cmd(&["rc-service", service, "start"]))? {
            outcome.notes.push(format!(
                "\"{service}\" will start at boot, but didn't start now. \
                 Check it with: {} rc-service {service} status",
                crate::security::admin()
            ));
        }
    }
    Ok(())
}

/// Step 8: point the boot menu's undo entry at this switch's snapshot, or
/// remove it if `undo_entry` was turned off.
fn update_boot_menu(
    config: &Config,
    root: &Path,
    plan: &Plan,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    if config.boot.undo_entry && plan.boot.problems.is_empty() {
        if let Some(note) = write_undo_entry(root, opts, outcome)? {
            outcome.notes.push(note);
        }
    } else if plan.boot.remove_entry {
        let path = root.join(crate::boot::CUSTOM_CFG);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::write(&path, crate::boot::merge_custom_cfg(&text, ""))
            .context("Couldn't remove the undo entry from the boot menu")?;
    }
    Ok(())
}
