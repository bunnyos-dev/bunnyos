//! `bunny switch`: make a system match its config.
//!
//! Every change is backed up first, and commands go through a [`Runner`] so
//! tests (and `--root` on a fake system) can see exactly what would run
//! without running it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::explain;
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

    /// Runs a command, showing its output live (including questions it asks)
    /// while keeping a copy. Returns whether it succeeded and what it printed.
    fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
        Ok((self.run(cmd)?, String::new()))
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

    fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
        use std::io::{Read, Write};
        use std::process::Stdio;
        use std::sync::{Arc, Mutex};

        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        let copy = Arc::new(Mutex::new(Vec::new()));

        // Pass output through as it arrives, byte by byte rather than line by
        // line, so questions without a newline ("[Yes/No]") still show up.
        fn pump<R: Read + Send + 'static>(
            mut from: R,
            to_stderr: bool,
            copy: Arc<Mutex<Vec<u8>>>,
        ) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = from.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let _ = if to_stderr {
                        let mut e = std::io::stderr();
                        e.write_all(&buf[..n]).and_then(|_| e.flush())
                    } else {
                        let mut o = std::io::stdout();
                        o.write_all(&buf[..n]).and_then(|_| o.flush())
                    };
                    if let Ok(mut c) = copy.lock() {
                        c.extend_from_slice(&buf[..n]);
                    }
                }
            })
        }
        let out = pump(child.stdout.take().expect("piped"), false, copy.clone());
        let err = pump(child.stderr.take().expect("piped"), true, copy.clone());
        let status = child.wait()?;
        let _ = out.join();
        let _ = err.join();
        let text = String::from_utf8_lossy(&copy.lock().map(|c| c.clone()).unwrap_or_default())
            .into_owned();
        Ok((status.success(), text))
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
    /// Ask emerge for colors (its output is captured, so it can't tell it's
    /// talking to a terminal). Only when bunny's own output is a terminal.
    pub color: bool,
}

/// What happened, for the summary at the end.
#[derive(Debug, Default)]
pub struct Outcome {
    pub snapshot: Option<PathBuf>,
    pub backup_dir: PathBuf,
    pub notes: Vec<String>,
    /// emerge's notices (news, reminders), explained. Shown after the summary.
    pub tips: Vec<String>,
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
        if opts.color {
            emerge.push("--color=y".into());
        }
        emerge.push("@world".into());
        let (ok, output) = runner.run_captured(&emerge)?;
        let explained = explain::explain(&explain::strip_ansi(&output));
        if !ok {
            bail!("{}", emerge_failed(&explained, &outcome));
        }
        outcome.tips.extend(
            explained
                .iter()
                .filter(|e| e.severity != explain::Severity::Problem)
                .map(|e| e.render()),
        );
    }

    // 5. Admin rights. After the install (so the tool and its checker exist)
    //    and before depclean (so the old tool is only removed once the new
    //    one works).
    apply_security(plan, runner, &outcome)?;

    // 6. Remove packages that aren't in the config. Always asks.
    if !plan.remove_packages.is_empty() && !runner.run(&cmd(&["emerge", "--depclean", "--ask"]))? {
        outcome.notes.push(format!(
            "Unused packages weren't removed. Run `{} emerge --depclean --ask` when you're ready.",
            crate::security::admin()
        ));
    }

    // 7. Services. Check again: packages installed above may have added them.
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

/// What to tell the user when emerge stops: bunny's explanation if it
/// recognised the message, and where their old system is.
fn emerge_failed(explained: &[explain::Explanation], outcome: &Outcome) -> String {
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

/// Writes the sudo/doas file and adds admins to `wheel`. The file is checked
/// with the tool's own checker first; if it fails, nothing is replaced and the
/// switch stops before depclean, so the tool you have keeps working.
fn apply_security(plan: &Plan, runner: &mut dyn Runner, outcome: &Outcome) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (Some(privilege), Some((path, text))) = (plan.security.privilege, &plan.security.file)
    else {
        return add_admins(plan, runner);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Couldn't create {}", parent.display()))?;
    }
    let mut staged = path.clone().into_os_string();
    staged.push(".bunny-new");
    let staged = std::path::PathBuf::from(staged);
    std::fs::write(&staged, text)
        .with_context(|| format!("Couldn't write {}", staged.display()))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(privilege.mode()))?;

    if !runner.run(&privilege.check_command(&staged))? {
        let _ = std::fs::remove_file(&staged);
        bail!(
            "{} rejected the file bunny generated for it, so it wasn't put in place,\n  \
             and no packages were removed. Your current admin setup still works.\n  \
             Please report this: https://github.com/bunnyos-dev/bunnyos/issues",
            privilege.command()
        );
    }
    if path.exists() {
        let name = path.file_name().unwrap_or_default();
        std::fs::copy(path, outcome.backup_dir.join(name))
            .with_context(|| format!("Couldn't back up {}", path.display()))?;
    }
    std::fs::rename(&staged, path)
        .with_context(|| format!("Couldn't put {} in place", path.display()))?;
    add_admins(plan, runner)
}

fn add_admins(plan: &Plan, runner: &mut dyn Runner) -> Result<()> {
    for user in &plan.security.add_to_wheel {
        if !runner.run(&cmd(&["gpasswd", "-a", user, "wheel"]))? {
            bail!("Couldn't add {user} to the wheel group.");
        }
    }
    Ok(())
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
        /// What emerge "prints" when its output is captured.
        emerge_output: String,
    }

    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            let line = cmd.join(" ");
            let ok = self.fail.is_none_or(|f| !line.contains(f));
            self.commands.push(line);
            Ok(ok)
        }

        fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
            Ok((self.run(cmd)?, self.emerge_output.clone()))
        }
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn system_runner_captures_and_passes_through() {
        // A question with no newline (like emerge's "[Yes/No]"), output on
        // both stdout and stderr, and a failing exit code.
        let cmd = cmd(&[
            "sh",
            "-c",
            "printf 'Continue? [Yes/No] '; echo oops >&2; echo done; exit 3",
        ]);
        let (ok, text) = SystemRunner.run_captured(&cmd).unwrap();
        assert!(!ok);
        assert!(text.contains("Continue? [Yes/No] "), "{text}");
        assert!(text.contains("oops"), "{text}");
        assert!(text.contains("done"), "{text}");
    }

    #[test]
    fn failed_emerge_is_explained() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder {
            fail: Some("@world"),
            emerge_output: fixture("use-change-swtpm.txt"),
            ..Default::default()
        };
        let err = switch(&config, root.path(), &plan, &mut runner, &opts())
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("emerge stopped"), "{err}");
        assert!(err.contains("✗ swtpm needs gnutls"), "{err}");
        assert!(err.contains("[pkg.\"net-libs/gnutls\"]"), "{err}");
        assert!(err.contains("Nothing is lost"), "{err}");
        assert!(err.contains("bunny-20261116-120000"), "{err}");
    }

    #[test]
    fn unknown_emerge_failure_says_so() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder {
            fail: Some("@world"),
            emerge_output: "something new and strange\n".into(),
            ..Default::default()
        };
        let err = switch(&config, root.path(), &plan, &mut runner, &opts())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("doesn't recognise emerge's message yet"),
            "{err}"
        );
    }

    #[test]
    fn emerge_notices_become_tips() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder {
            emerge_output: format!("\u{1b}[32m*\u{1b}[0m{}", fixture("news-and-depclean.txt")),
            ..Default::default()
        };
        let outcome = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap();
        assert_eq!(outcome.tips.len(), 2, "{:#?}", outcome.tips);
        assert!(outcome.tips[0].contains("26 announcements"));
    }

    #[test]
    fn asks_emerge_for_colors_on_a_terminal() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Recorder::default();
        let opts = Options {
            color: true,
            ..opts()
        };
        switch(&config, root.path(), &plan, &mut runner, &opts).unwrap();
        assert!(
            runner.commands[1].contains("--color=y @world"),
            "{}",
            runner.commands[1]
        );
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
            color: false,
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
                color: false,
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
