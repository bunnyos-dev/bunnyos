//! `bunny switch`: make a system match its config.
//!
//! Every change is backed up first, and commands go through a [`Runner`] so
//! tests (and `--root` on a fake system) can see exactly what would run
//! without running it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::import;
use crate::init;
use crate::notes::{self, Kind, Note, State};
use crate::plan::Plan;
use crate::portage;

/// Runs external commands.
pub trait Runner {
    /// Runs a command, showing its output to the user. Returns whether it succeeded.
    fn run(&mut self, cmd: &[String]) -> Result<bool>;

    /// Runs a command and returns what it printed, or `None` if it failed.
    /// Only for commands that just read information.
    fn output(&mut self, _cmd: &[String]) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Runs commands for real.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        let status = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .status()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        Ok(status.success())
    }

    fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
        let out = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .output()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        Ok(out
            .status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned()))
    }
}

/// Prints commands instead of running them (for `--root` on a fake system).
pub struct PrintRunner;

impl Runner for PrintRunner {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        println!("  (would run) {}", cmd.join(" "));
        Ok(true)
    }
}

#[derive(Clone)]
pub struct Options {
    /// Don't let emerge ask before installing. Removing packages always asks.
    pub yes: bool,
    /// Used to name the snapshot and the backup folder, e.g. `20261116-203000`.
    pub stamp: String,
    /// Run the world update even if the plan has no package changes
    /// (for `bunny update`).
    pub update_all: bool,
}

/// What happened, for the summary at the end.
#[derive(Debug, Default)]
pub struct Outcome {
    pub snapshot: Option<PathBuf>,
    pub backup_dir: PathBuf,
    pub notes: Vec<String>,
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Applies `plan` (made from `config`) to the system at `root`.
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

    // 1. Snapshot first, so there's always a way back.
    let snapshots = root.join(".snapshots");
    if snapshots.is_dir() {
        let target = snapshots.join(format!("bunny-{}", opts.stamp));
        let ok = runner.run(&cmd(&[
            "btrfs",
            "subvolume",
            "snapshot",
            "-r",
            &root.to_string_lossy(),
            &target.to_string_lossy(),
        ]))?;
        if !ok {
            bail!(
                "Couldn't take a snapshot before changing anything, so nothing was changed.\n  \
                 bunny needs / on btrfs with a /.snapshots folder to be able to undo changes."
            );
        }
        outcome.snapshot = Some(target);
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

    std::fs::create_dir_all(&outcome.backup_dir)
        .with_context(|| format!("Couldn't create {}", outcome.backup_dir.display()))?;

    // 2. USE flags: bunny takes over package.use.
    adopt_package_use(config, root, &outcome.backup_dir)?;

    // 3. World set.
    let world = portage::world_path(root);
    if world.exists() {
        std::fs::copy(&world, outcome.backup_dir.join("world"))
            .context("Couldn't back up the world file")?;
    } else if let Some(parent) = world.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&world, portage::render_world(config))
        .context("Couldn't write the world file")?;

    // 4. Install and update.
    if opts.update_all || !plan.add_packages.is_empty() || !plan.use_changes.is_empty() {
        let mut emerge = cmd(&["emerge", "--update", "--deep", "--newuse", "--verbose"]);
        if !opts.yes {
            emerge.push("--ask".into());
        }
        emerge.push("@world".into());
        if !runner.run(&emerge)? {
            bail!(
                "emerge didn't finish, so some changes weren't installed.\n  \
                 Your old files are backed up in {}.{}\n  \
                 To understand emerge's message, copy it and run `bunny explain`, then paste it.\n  \
                 Fix the problem, then run `bunny switch` again.",
                outcome.backup_dir.display(),
                outcome
                    .snapshot
                    .as_ref()
                    .map(|s| format!(
                        "\n  The system before this switch is saved in {}.",
                        s.display()
                    ))
                    .unwrap_or_default()
            );
        }
    }

    // 5. Remove packages that aren't in the config. Always asks.
    if !plan.remove_packages.is_empty() && !runner.run(&cmd(&["emerge", "--depclean", "--ask"]))? {
        outcome.notes.push(
            "Unused packages weren't removed. Run `doas emerge --depclean --ask` when you're ready."
                .into(),
        );
    }

    // 6. Services. Check again: packages installed above may have added them.
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
                 Check it with: doas rc-service {service} status"
            ));
        }
    }

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

/// package.use files that aren't bunny's (they'll be moved to the backup).
pub fn foreign_package_use_files(root: &Path) -> Vec<String> {
    let dir = portage::package_use_dir(root);
    if dir.is_file() {
        return vec![portage::PACKAGE_USE_DIR.to_string()];
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != portage::BUNNY_USE_FILE && n != portage::KEPT_USE_FILE)
        .collect();
    names.sort();
    names
}

/// Moves the user's package.use files into the backup folder and replaces
/// them with bunny's file. Nothing is lost: the flags are in the config, and
/// lines bunny can't manage yet are kept word for word in their own file.
fn adopt_package_use(config: &Config, root: &Path, backup: &Path) -> Result<()> {
    let kept = import::read_package_use(root)?.skipped;
    let dir = portage::package_use_dir(root);
    let backup_dir = backup.join("package.use");

    if dir.is_file() {
        std::fs::create_dir_all(backup)?;
        std::fs::rename(&dir, &backup_dir).context("Couldn't back up package.use")?;
    } else if dir.is_dir() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == portage::BUNNY_USE_FILE || name == portage::KEPT_USE_FILE {
                continue;
            }
            std::fs::create_dir_all(&backup_dir)?;
            std::fs::rename(entry.path(), backup_dir.join(&name)).with_context(|| {
                format!("Couldn't move {} to the backup", entry.path().display())
            })?;
        }
    }
    std::fs::create_dir_all(&dir)?;

    let kept_path = dir.join(portage::KEPT_USE_FILE);
    if kept.is_empty() {
        let _ = std::fs::remove_file(&kept_path);
    } else {
        let mut text = String::from(
            "# Kept by bunny: lines bunny can't manage yet (version ranges, wildcards).\n\
             # They work as before. Edit them here.\n",
        );
        for (file, line) in &kept {
            text.push_str(&format!("# from {}\n{line}\n", file.display()));
        }
        std::fs::write(&kept_path, text)?;
    }

    std::fs::write(
        dir.join(portage::BUNNY_USE_FILE),
        portage::render_package_use(config),
    )
    .context("Couldn't write bunny's package.use file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan;

    /// Records commands instead of running them.
    #[derive(Default)]
    struct Recorder {
        commands: Vec<String>,
        fail: Option<&'static str>,
    }

    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            let line = cmd.join(" ");
            let ok = self.fail.is_none_or(|f| !line.contains(f));
            self.commands.push(line);
            Ok(ok)
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fake_system() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            portage::WORLD_FILE,
            "app-editors/neovim\napp-misc/old\n",
        );
        write(
            root,
            "etc/portage/package.use/mine",
            "www-client/firefox pgo\n",
        );
        write(root, "etc/portage/package.use/odd", "<dev-libs/x-2 y\n");
        write(root, "etc/init.d/sshd", "");
        std::fs::create_dir_all(root.join(".snapshots")).unwrap();
        dir
    }

    fn opts() -> Options {
        Options {
            yes: false,
            stamp: "20261116-120000".into(),
            update_all: false,
        }
    }

    const CONFIG: &str = r#"
        packages = ["app-editors/neovim", "app-shells/zsh"]
        services = ["sshd", "bluetooth"]
        [pkg."www-client/firefox"]
        use = ["pgo", "-telemetry"]
    "#;

    #[test]
    fn switches_a_system() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder::default();
        let outcome = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap();

        // Commands, in order: snapshot, install, remove (asks), enable + start service.
        assert_eq!(runner.commands.len(), 5, "{:#?}", runner.commands);
        assert!(runner.commands[0].starts_with("btrfs subvolume snapshot -r "));
        assert!(runner.commands[0].ends_with(".snapshots/bunny-20261116-120000"));
        assert_eq!(
            runner.commands[1],
            "emerge --update --deep --newuse --verbose --ask @world"
        );
        assert_eq!(runner.commands[2], "emerge --depclean --ask");
        assert_eq!(runner.commands[3], "rc-update add sshd default");
        assert_eq!(runner.commands[4], "rc-service sshd start");

        // bluetooth isn't installed: noted, not enabled.
        assert!(outcome.notes.iter().any(|n| n.contains("bluetooth")));

        // Files: world written, package.use adopted, old files backed up.
        let r = root.path();
        assert_eq!(
            std::fs::read_to_string(r.join(portage::WORLD_FILE)).unwrap(),
            "app-editors/neovim\napp-shells/zsh\n"
        );
        let use_dir = r.join(portage::PACKAGE_USE_DIR);
        let bunny_file = std::fs::read_to_string(use_dir.join(portage::BUNNY_USE_FILE)).unwrap();
        assert!(bunny_file.contains("www-client/firefox pgo -telemetry"));
        let kept = std::fs::read_to_string(use_dir.join(portage::KEPT_USE_FILE)).unwrap();
        assert!(kept.contains("<dev-libs/x-2 y"));
        assert!(!use_dir.join("mine").exists());
        assert!(outcome.backup_dir.join("package.use/mine").exists());
        assert!(outcome.backup_dir.join("world").exists());

        // Afterwards the plan has no USE or world differences left.
        let after = plan::make(&config, r).unwrap();
        assert!(after.use_changes.is_empty(), "{:#?}", after.use_changes);
        assert!(after.add_packages.is_empty() && after.remove_packages.is_empty());
    }

    #[test]
    fn yes_skips_emerge_questions_but_never_for_removal() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder::default();
        let opts = Options {
            yes: true,
            ..opts()
        };
        switch(&config, root.path(), &plan, &mut runner, &opts).unwrap();
        assert!(!runner.commands[1].contains("--ask"));
        assert_eq!(runner.commands[2], "emerge --depclean --ask");
    }

    #[test]
    fn failed_snapshot_changes_nothing() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder {
            fail: Some("snapshot"),
            ..Default::default()
        };
        let err = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap_err();
        assert!(err.to_string().contains("nothing was changed"));
        assert_eq!(runner.commands.len(), 1);
        assert!(root.path().join("etc/portage/package.use/mine").exists());
    }

    #[test]
    fn failed_emerge_points_to_the_backup() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder {
            fail: Some("@world"),
            ..Default::default()
        };
        let err = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("backed up in"), "{text}");
        assert!(text.contains("bunny-20261116-120000"), "{text}");
    }

    #[test]
    fn running_twice_is_safe() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        for stamp in ["a", "b"] {
            let plan = plan::make(&config, root.path()).unwrap();
            let opts = Options {
                yes: true,
                stamp: stamp.into(),
                update_all: false,
            };
            switch(&config, root.path(), &plan, &mut Recorder::default(), &opts).unwrap();
        }
        let kept = std::fs::read_to_string(
            root.path()
                .join(portage::PACKAGE_USE_DIR)
                .join(portage::KEPT_USE_FILE),
        )
        .unwrap();
        // The kept line is still there exactly once.
        assert_eq!(kept.matches("<dev-libs/x-2 y").count(), 1, "{kept}");
    }
}
