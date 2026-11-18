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

    /// Runs a command without showing its output, calling `on_line` for each
    /// line (for a progress display). Returns whether it succeeded and
    /// everything it printed.
    fn run_quiet(
        &mut self,
        cmd: &[String],
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<(bool, String)> {
        Ok((self.run(cmd)?, String::new()))
    }

    /// True if commands are only printed, not run (a test root).
    fn dry(&self) -> bool {
        false
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

    fn run_quiet(
        &mut self,
        cmd: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(bool, String)> {
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;

        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        // Read raw bytes per line, so output that isn't valid UTF-8 can't
        // stop the reader (which would leave the command blocked).
        fn reader<R: std::io::Read + Send + 'static>(
            from: R,
            tx: std::sync::mpsc::Sender<String>,
        ) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                let mut from = BufReader::new(from);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match from.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf);
                            let line = line.trim_end_matches(['\n', '\r']).to_string();
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
        }
        let out = reader(child.stdout.take().expect("piped"), tx.clone());
        let err = reader(child.stderr.take().expect("piped"), tx);
        let mut all = String::new();
        for line in rx {
            on_line(&line);
            all.push_str(&line);
            all.push('\n');
        }
        let status = child.wait()?;
        let _ = out.join();
        let _ = err.join();
        Ok((status.success(), all))
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

    fn dry(&self) -> bool {
        true
    }
}

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

    // 0. The boot menu's undo step goes into the startup image *before* the
    //    snapshot, so this very generation can already be undone from the
    //    boot menu. It only adds a module and rebuilds the image (the old
    //    image is backed up); if that fails, nothing else has changed.
    if let Some(versions) = &plan.boot.install_module {
        install_boot_module(root, versions, runner, &outcome.backup_dir)?;
    }

    // 1. Snapshot first, so there's always a way back.
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

    // 2. USE flags: bunny takes over package.use. A full copy is kept first,
    //    so a cancelled switch can put everything back exactly.
    let use_dir = portage::package_use_dir(root);
    if use_dir.exists() {
        copy_tree(&use_dir, &outcome.backup_dir.join(USE_BEFORE))?;
    }
    adopt_package_use(config, root, &outcome.backup_dir)?;

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

    // 4. Install and update.
    let needs_emerge =
        opts.update_all || !plan.add_packages.is_empty() || !plan.use_changes.is_empty();
    if needs_emerge && !opts.verbose && !runner.dry() {
        install_quietly(root, plan, runner, opts, &mut outcome)?;
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
            bail!("{}", emerge_failed(&explained, &outcome));
        }
        outcome
            .tips
            .extend(explained.iter().filter(|e| is_tip(e)).map(|e| e.render()));
    }

    // 5. Admin rights. After the install (so the tool and its checker exist)
    //    and before depclean (so the old tool is only removed once the new
    //    one works).
    apply_security(plan, runner, &outcome)?;

    // 6. Remove packages nothing needs anymore: ones taken out of the config,
    //    and dependencies left over after USE changes or updates (sudo built
    //    without `sendmail` no longer needs a mail server). Always asks.
    let cleanup =
        !plan.remove_packages.is_empty() || !plan.use_changes.is_empty() || opts.update_all;
    if cleanup {
        let removed = if opts.verbose || runner.dry() {
            runner.run(&cmd(&["emerge", "--depclean", "--ask"]))?
        } else {
            remove_quietly(runner, opts, &outcome)?
        };
        if !removed {
            outcome.notes.push(format!(
                "Unused packages weren't removed. Run `{} emerge --depclean --ask` when you're ready.",
                crate::security::admin()
            ));
        }
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

    // 8. The boot menu's undo entry, pointing at this switch's snapshot.
    if config.boot.undo_entry && plan.boot.problems.is_empty() {
        if let Some(note) = write_undo_entry(root, opts, &outcome)? {
            outcome.notes.push(note);
        }
    } else if plan.boot.remove_entry {
        let path = root.join(crate::boot::CUSTOM_CFG);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::write(&path, crate::boot::merge_custom_cfg(&text, ""))
            .context("Couldn't remove the undo entry from the boot menu")?;
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

/// Installs the dracut module and rebuilds the startup image for each kernel.
/// Each new image is built as a separate file and only swapped in once it's
/// complete; the old one is backed up. If anything fails, the module is
/// removed again and the old images stay.
fn install_boot_module(
    root: &Path,
    versions: &[String],
    runner: &mut dyn Runner,
    backup: &Path,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = root.join(crate::boot::MODULE_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    for (name, text) in [
        ("module-setup.sh", crate::boot::MODULE_SETUP),
        ("bunny-rollback.sh", crate::boot::HOOK),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text)
            .with_context(|| format!("Couldn't write {}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    let undo = |why: String| -> anyhow::Error {
        let _ = std::fs::remove_dir_all(&dir);
        anyhow::anyhow!(
            "{why}\n  The startup image wasn't changed, and nothing else was either.\n  \
             To go ahead without the boot menu entry, set `undo_entry = false` under [boot]."
        )
    };

    println!("\nAdding the undo step to the startup image…");
    for version in versions {
        let image = crate::boot::initramfs(root, version);
        let mut staged = image.clone().into_os_string();
        staged.push(".bunny-new");
        let staged = PathBuf::from(staged);
        let (ok, said) = runner.run_quiet(
            &cmd(&["dracut", "--force", &staged.to_string_lossy(), version]),
            &mut |_| {},
        )?;
        if runner.dry() {
            continue;
        }
        if !ok || !staged.is_file() {
            let _ = std::fs::remove_file(&staged);
            return Err(undo(format!(
                "Couldn't rebuild the startup image for kernel {version}. dracut said:\n{}",
                crate::emerge::tail(&said, 10)
            )));
        }
        std::fs::create_dir_all(backup)?;
        if image.exists() {
            std::fs::copy(&image, backup.join(image.file_name().unwrap_or_default()))
                .with_context(|| format!("Couldn't back up {}", image.display()))?;
        }
        std::fs::rename(&staged, &image)
            .with_context(|| format!("Couldn't put the new {} in place", image.display()))?;
    }
    Ok(())
}

/// Points the boot menu's undo entry at this switch's snapshot. Returns a
/// note for the summary.
fn write_undo_entry(root: &Path, opts: &Options, outcome: &Outcome) -> Result<Option<String>> {
    use crate::boot;
    if outcome.snapshot.is_none() {
        return Ok(None);
    }
    let generation = format!("bunny-{}", opts.stamp);
    let generation_dir = root.join(".snapshots").join(&generation);
    let cmdline = std::fs::read_to_string(root.join("proc/cmdline")).unwrap_or_default();
    let running =
        std::fs::read_to_string(root.join("proc/sys/kernel/osrelease")).unwrap_or_default();
    let not_added = |why: &str| {
        Ok(Some(format!(
            "The boot menu's undo entry wasn't updated: {why}."
        )))
    };

    let Some(uuid) = boot::root_uuid(&cmdline) else {
        return not_added("the system isn't started with root=UUID=…");
    };
    let Some(version) = boot::generation_kernel(&generation_dir, running.trim()) else {
        return not_added("couldn't find a kernel in the saved generation");
    };
    if !boot::generation_has_module(&generation_dir, &version) {
        return not_added("the saved generation's startup image doesn't have the undo step yet");
    }
    let when = crate::rollback::Generation {
        name: generation.clone(),
    }
    .when();
    let block = boot::render_entry(
        &generation,
        &when,
        &version,
        uuid,
        &boot::undo_cmdline(&cmdline, &generation),
    );
    let path = root.join(boot::CUSTOM_CFG);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, boot::merge_custom_cfg(&text, &block))
        .with_context(|| format!("Couldn't write {}", path.display()))?;
    Ok(Some(
        "If this change stops the system from starting, pick \"Bunny Linux: undo the last change\" \
         in the boot menu (hold Shift or Esc while starting if the menu is hidden)."
            .to_string(),
    ))
}

/// What to tell the user when emerge stops: bunny's explanation if it
/// recognised the message, and where their old system is.
/// What to tell someone who pressed Ctrl+C while emerge was installing.
/// `reached` is (the package emerge was on, of how many, its name).
fn interrupted_message(
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
            "  Your config still lists {}, so `bunny status` will show it as not applied yet.\n",
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

/// The copy of package.use taken before the takeover, inside the backup folder.
const USE_BEFORE: &str = "package.use.before";

/// Copies a file or a folder (recursively).
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to).with_context(|| format!("Couldn't copy {}", from.display()))?;
    }
    Ok(())
}

/// Puts the world file and package.use back as they were before this switch.
/// Used when emerge refuses or the user cancels before anything is installed.
fn restore_files(root: &Path, backup: &Path) -> Result<()> {
    let world = portage::world_path(root);
    let saved = backup.join("world");
    if saved.exists() {
        std::fs::copy(&saved, &world).context("Couldn't put the world file back")?;
    } else {
        let _ = std::fs::remove_file(&world);
    }
    let use_dir = portage::package_use_dir(root);
    if use_dir.is_dir() {
        std::fs::remove_dir_all(&use_dir)?;
    } else if use_dir.exists() {
        std::fs::remove_file(&use_dir)?;
    }
    let before = backup.join(USE_BEFORE);
    if before.exists() {
        copy_tree(&before, &use_dir).context("Couldn't put package.use back")?;
    }
    Ok(())
}

/// Nothing was installed after all (emerge refused, or the user said no):
/// put the files back and delete the snapshot taken for this switch, so it
/// doesn't show up as a generation that changed nothing.
fn discard(root: &Path, runner: &mut dyn Runner, outcome: &mut Outcome) -> Result<()> {
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
fn save_log(root: &Path, stamp: &str, what: &str, text: &str) -> Option<PathBuf> {
    let dir = root.join("var/log/bunny");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{stamp}-{what}.log"));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// Asks emerge what it would install (changing nothing), shows that as a
/// short list, asks if it's more than the plan showed, then installs with a
/// progress line. The full output goes to /var/log/bunny.
fn install_quietly(
    root: &Path,
    plan: &Plan,
    runner: &mut dyn Runner,
    opts: &Options,
    outcome: &mut Outcome,
) -> Result<()> {
    let update = ["emerge", "--update", "--deep", "--newuse", "--color=n"];

    println!("\nAsking Portage what it needs to do…");
    let mut pretend = cmd(&update);
    pretend.insert(1, "--pretend".into());
    pretend.push("@world".into());
    // Portage's own answer, as data. emerge's screen text only when that
    // isn't available (and to explain a refusal).
    let (list, dry_run_log) = match crate::helper::plan(runner) {
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
                let explained = explain::explain(&output);
                let mut text = String::from(
                    "emerge found a problem before installing anything, so nothing was changed.\n",
                );
                if explained.is_empty() {
                    text.push_str(
                        "\nbunny doesn't recognise emerge's message yet. The end of it:\n\n",
                    );
                    text.push_str(&crate::emerge::tail(&output, 20));
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
                return Err(Refused(text).into());
            }

            let list = crate::emerge::parse_merge_list(&output);
            (list, format!("$ {}\n{output}", pretend.join(" ")))
        }
    };
    if list.is_empty() {
        println!("emerge has nothing to install.");
        return Ok(());
    }
    print!(
        "\n{}",
        crate::emerge::render_merge_list(&list, &plan.add_packages)
    );
    // A big compile when a prebuilt version exists: say so before it starts.
    let available: std::collections::BTreeSet<String> =
        crate::repo::all(root).into_iter().map(|p| p.name).collect();
    let alternatives = crate::emerge::prebuilt_alternatives(&list, &plan.add_packages, &available);
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

    let mut install = cmd(&update);
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
fn remove_quietly(runner: &mut dyn Runner, opts: &Options, outcome: &Outcome) -> Result<bool> {
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

    let (ok, said) = runner.run_quiet(&privilege.check_command(&staged), &mut |_| {})?;
    if !ok {
        let _ = std::fs::remove_file(&staged);
        bail!(
            "{} rejected the file bunny generated for it, so it wasn't put in place,\n  \
             and no packages were removed. Your current admin setup still works.\n  \
             It said: {}\n  \
             Please report this: https://github.com/bunnyos-dev/bunnyos/issues",
            privilege.command(),
            said.trim()
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

/// emerge's notices worth passing on after a switch. Its reminder to remove
/// unused packages isn't: bunny just did that.
fn is_tip(e: &explain::Explanation) -> bool {
    e.severity != explain::Severity::Problem && !e.title.starts_with(explain::DEPCLEAN_REMINDER)
}

/// Moves the user's package.use files into the backup folder and replaces
/// them with bunny's file. Nothing is lost: the flags are in the config, and
/// lines bunny can't manage yet are kept word for word in their own file.
fn adopt_package_use(config: &Config, root: &Path, backup: &Path) -> Result<()> {
    let kept = import::read_package_use(root)?.skipped;
    let dir = portage::package_use_dir(root);
    // Lines already in bunny's kept file remember where they first came from.
    let origins = kept_origins(&dir.join(portage::KEPT_USE_FILE));
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
            let from = if file
                .file_name()
                .is_some_and(|n| n == portage::KEPT_USE_FILE)
            {
                origins
                    .get(line.as_str())
                    .cloned()
                    .unwrap_or_else(|| file.display().to_string())
            } else {
                file.display().to_string()
            };
            text.push_str(&format!("# from {from}\n{line}\n"));
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

/// In bunny's kept file, each line follows a "# from <file>" comment: which
/// file it first came from.
fn kept_origins(path: &Path) -> std::collections::HashMap<String, String> {
    let mut origins = std::collections::HashMap::new();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut from: Option<&str> = None;
    for line in text.lines() {
        if let Some(f) = line.strip_prefix("# from ") {
            from = Some(f);
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            if let Some(f) = from.take() {
                origins.insert(line.to_string(), f.to_string());
            }
        }
    }
    origins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_a_ctrl_c_during_install() {
        let outcome = Outcome {
            snapshot: Some(PathBuf::from("/.snapshots/bunny-20261118-042343")),
            log: Some(PathBuf::from("/var/log/bunny/20261118-042343-emerge.log")),
            ..Outcome::default()
        };
        let text = interrupted_message(
            Some(&(102, 102, "www-client/firefox".to_string())),
            &["www-client/firefox".to_string()],
            &outcome,
        );
        assert!(
            text.starts_with("Stopped with Ctrl+C while installing."),
            "{text}"
        );
        assert!(
            text.contains(
                "101 of 102 packages finished installing; www-client/firefox was in progress"
            ),
            "{text}"
        );
        assert!(
            text.contains("Your config still lists www-client/firefox"),
            "{text}"
        );
        assert!(text.contains("`bunny rollback`, then restart"), "{text}");
        assert!(text.contains("/.snapshots/bunny-20261118-042343"), "{text}");
        assert!(
            text.contains("`bunny switch` picks up where this stopped"),
            "{text}"
        );
        assert!(
            text.contains("emerge's output so far is in /var/log/bunny/"),
            "{text}"
        );

        let early = interrupted_message(None, &[], &Outcome::default());
        assert!(
            early.contains("hadn't started installing anything"),
            "{early}"
        );
        assert!(
            !early.contains("rollback"),
            "no snapshot, so no rollback to offer: {early}"
        );
    }
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
        // The news is passed on; emerge's "remove unused packages" reminder
        // isn't, since bunny just did that.
        assert_eq!(outcome.tips.len(), 1, "{:#?}", outcome.tips);
        assert!(outcome.tips[0].contains("26 announcements"));
    }

    #[test]
    fn kept_lines_remember_their_file_on_later_switches() {
        let root = fake_system();
        let r = root.path();
        let use_dir = r.join(portage::PACKAGE_USE_DIR);
        std::fs::create_dir_all(&use_dir).unwrap();
        std::fs::write(use_dir.join("ranges"), "<dev-lang/rust-2 nightly\n").unwrap();
        let config = Config::parse(CONFIG).unwrap();
        for _ in 0..2 {
            let plan = plan::make(&config, r).unwrap();
            let mut runner = Recorder::default();
            switch(&config, r, &plan, &mut runner, &opts()).unwrap();
        }
        let kept = std::fs::read_to_string(use_dir.join(portage::KEPT_USE_FILE)).unwrap();
        assert!(
            kept.contains("# from /etc/portage/package.use/ranges\n<dev-lang/rust-2 nightly"),
            "{kept}"
        );
        assert!(!kept.contains(portage::KEPT_USE_FILE), "{kept}");
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
            // These tests cover emerge's own output and questions; the quiet
            // path has its own tests below.
            verbose: true,
            confirm: |_, _| Ok(true),
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
                verbose: true,
                confirm: |_, _| Ok(true),
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

    /// Answers each command with canned output (matched by a piece of the
    /// command line), and feeds it line by line like emerge would.
    #[derive(Default)]
    struct Scripted {
        commands: Vec<String>,
        replies: Vec<(&'static str, bool, String)>,
    }

    impl Scripted {
        fn reply(&mut self, line: &str) -> (bool, String) {
            self.commands.push(line.to_string());
            self.replies
                .iter()
                .find(|(pat, _, _)| line.contains(pat))
                .map(|(_, ok, out)| (*ok, out.clone()))
                .unwrap_or((true, String::new()))
        }
    }

    impl Runner for Scripted {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            Ok(self.reply(&cmd.join(" ")).0)
        }

        fn run_quiet(
            &mut self,
            cmd: &[String],
            on_line: &mut dyn FnMut(&str),
        ) -> Result<(bool, String)> {
            let (ok, out) = self.reply(&cmd.join(" "));
            out.lines().for_each(on_line);
            Ok((ok, out))
        }
    }

    fn quiet(yes: bool, answer: bool) -> Options {
        Options {
            yes,
            verbose: false,
            confirm: if answer {
                |_, _| Ok(true)
            } else {
                |_, _| Ok(false)
            },
            ..opts()
        }
    }

    const PRETEND: &str = "[binary   N     ] acct-group/nullmail-0-r2::gentoo\n\
                           [binary   N     ] app-misc/tmux-3.6a::gentoo\n";
    const INSTALL: &str = ">>> Emerging binary (1 of 2) acct-group/nullmail-0-r2::gentoo\n\
                           >>> Emerging binary (2 of 2) app-misc/tmux-3.6a::gentoo\n\
                           \n * Messages for package app-misc/tmux-3.6a:\n\n * Read the manual.\n";

    #[test]
    fn quiet_install_asks_emerge_first_then_installs_without_its_questions() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Scripted {
            replies: vec![
                ("--pretend --update", true, PRETEND.into()),
                ("--quiet-build=y", true, INSTALL.into()),
            ],
            ..Scripted::default()
        };
        let outcome = switch(
            &config,
            root.path(),
            &plan,
            &mut runner,
            &quiet(false, true),
        )
        .unwrap();
        let emerges: Vec<&String> = runner
            .commands
            .iter()
            .filter(|c| c.starts_with("emerge --"))
            .collect();
        assert!(emerges[0].starts_with("emerge --pretend --update --deep --newuse"));
        assert!(emerges[1].contains("--quiet-build=y @world"));
        assert!(!emerges[1].contains("--ask"), "{}", emerges[1]);
        assert_eq!(
            outcome.messages,
            [(
                "app-misc/tmux".to_string(),
                vec!["Read the manual.".to_string()]
            )]
        );
        let log = outcome.log.expect("log saved");
        assert!(std::fs::read_to_string(log)
            .unwrap()
            .contains(">>> Emerging binary (2 of 2)"));
    }

    #[test]
    fn saying_no_to_extra_packages_puts_everything_back() {
        let root = fake_system();
        let use_dir = root.path().join(portage::PACKAGE_USE_DIR);
        let world = portage::world_path(root.path());
        let world_before = std::fs::read_to_string(&world).unwrap();
        let files_before: Vec<_> = std::fs::read_dir(&use_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();

        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Scripted {
            replies: vec![("--pretend --update", true, PRETEND.into())],
            ..Scripted::default()
        };
        let err = switch(
            &config,
            root.path(),
            &plan,
            &mut runner,
            &quiet(false, false),
        )
        .unwrap_err();
        assert!(err.is::<Cancelled>());
        assert!(
            runner
                .commands
                .iter()
                .any(|c| c.starts_with("btrfs subvolume delete")),
            "the empty generation is deleted: {:?}",
            runner.commands
        );
        assert!(!runner.commands.iter().any(|c| c.contains("--quiet-build")));
        assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
        let mut files_after: Vec<_> = std::fs::read_dir(&use_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let mut files_before = files_before;
        files_before.sort();
        files_after.sort();
        assert_eq!(files_after, files_before);
    }

    #[test]
    fn a_refusal_before_installing_changes_nothing_and_is_explained() {
        let root = fake_system();
        let world = portage::world_path(root.path());
        let world_before = std::fs::read_to_string(&world).unwrap();
        let config = Config::parse(CONFIG).unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Scripted {
            replies: vec![("--pretend --update", false, fixture("use-change-swtpm.txt"))],
            ..Scripted::default()
        };
        let err = switch(&config, root.path(), &plan, &mut runner, &quiet(true, true))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("before installing anything, so nothing was changed"),
            "{err}"
        );
        assert!(err.contains("gnutls"), "{err}");
        assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
    }

    #[test]
    fn quiet_removal_lists_and_always_asks() {
        let root = fake_system();
        let config = Config::parse("packages = []").unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        assert!(!plan.remove_packages.is_empty());
        let depclean = "All selected packages: =app-text/tree-2.2.1\n\nNumber to remove:     1\n";
        // --yes, but the user says no at the removal question.
        let mut runner = Scripted {
            replies: vec![("--depclean --pretend", true, depclean.into())],
            ..Scripted::default()
        };
        let outcome = switch(
            &config,
            root.path(),
            &plan,
            &mut runner,
            &quiet(true, false),
        )
        .unwrap();
        assert!(!runner
            .commands
            .iter()
            .any(|c| c == "emerge --depclean --color=n"));
        assert!(outcome.notes.iter().any(|n| n.contains("weren't removed")));

        let mut runner = Scripted {
            replies: vec![("--depclean --pretend", true, depclean.into())],
            ..Scripted::default()
        };
        switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
        assert!(runner
            .commands
            .iter()
            .any(|c| c == "emerge --depclean --color=n"));
    }

    #[test]
    fn unreadable_depclean_output_falls_back_to_emerges_own_question() {
        let root = fake_system();
        let config = Config::parse("packages = []").unwrap();
        let plan = plan::make(&config, root.path()).unwrap();
        let mut runner = Scripted {
            replies: vec![("--depclean --pretend", true, "Number to remove: 3\n".into())],
            ..Scripted::default()
        };
        switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
        assert!(runner
            .commands
            .iter()
            .any(|c| c == "emerge --depclean --ask"));
    }

    #[test]
    fn system_runner_runs_quietly_line_by_line() {
        let cmd = cmd(&[
            "sh",
            "-c",
            "echo '>>> Emerging (1 of 2) a/b-1'; printf 'bad \\377 byte\\n'; echo err >&2; echo '>>> Emerging (2 of 2) a/c-2'; exit 1",
        ]);
        let mut seen = Vec::new();
        let (ok, text) = SystemRunner
            .run_quiet(&cmd, &mut |l| seen.push(l.to_string()))
            .unwrap();
        assert!(!ok);
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert!(seen.contains(&">>> Emerging (2 of 2) a/c-2".to_string()));
        assert!(text.contains("bad"));
        assert!(text.contains("err"));
    }

    #[test]
    fn no_second_question_when_emerge_wants_only_what_the_plan_showed() {
        let root = fake_system();
        let config = Config::parse(CONFIG).unwrap();
        let mut plan = plan::make(&config, root.path()).unwrap();
        plan.add_packages.clear();
        plan.use_changes = vec![crate::plan::UseChange {
            atom: "app-admin/sudo".into(),
            from: vec![],
            to: vec!["-sendmail".into()],
        }];
        let mut runner = Scripted {
            replies: vec![(
                "--pretend --update",
                true,
                "[ebuild   R    ] app-admin/sudo-1.9.17_p2::gentoo\n".into(),
            )],
            ..Scripted::default()
        };
        // The answer would be "no" if asked; it must not be asked.
        switch(
            &config,
            root.path(),
            &plan,
            &mut runner,
            &quiet(false, false),
        )
        .unwrap();
        assert!(runner
            .commands
            .iter()
            .any(|c| c.contains("--quiet-build=y")));
    }

    /// A system set up for the boot menu's undo entry: GRUB, dracut, one
    /// kernel, booted with root=UUID=.
    fn bootable_system() -> tempfile::TempDir {
        let dir = fake_system();
        let r = dir.path();
        write(r, "boot/grub/grub.cfg", "");
        write(r, "usr/bin/dracut", "");
        write(r, "boot/vmlinuz-6.12.58", "kernel");
        write(r, "boot/initramfs-6.12.58.img", "old image");
        std::fs::create_dir_all(r.join("lib/modules/6.12.58")).unwrap();
        write(
            r,
            "proc/cmdline",
            "BOOT_IMAGE=/@/boot/vmlinuz-6.12.58 root=UUID=1234-abcd ro rootflags=subvol=@\n",
        );
        write(r, "proc/sys/kernel/osrelease", "6.12.58\n");
        dir
    }

    const BOOT_CONFIG: &str = "packages = [\"app-editors/neovim\"]\n[boot]\nundo_entry = true\n";

    /// Pretends to be btrfs and dracut: the snapshot copies the system's
    /// /boot and dracut module into /.snapshots, and dracut writes an image.
    struct Builder {
        root: PathBuf,
        dracut_ok: bool,
        commands: Vec<String>,
    }

    impl Runner for Builder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.commands.push(cmd.join(" "));
            Ok(true)
        }

        fn run_quiet(&mut self, cmd: &[String], _: &mut dyn FnMut(&str)) -> Result<(bool, String)> {
            self.commands.push(cmd.join(" "));
            match cmd[0].as_str() {
                "dracut" if self.dracut_ok => {
                    std::fs::write(&cmd[2], "new image with the undo step").unwrap();
                    Ok((true, String::new()))
                }
                "dracut" => Ok((false, "dracut: something went wrong".into())),
                "btrfs" => {
                    let target = PathBuf::from(&cmd[5]);
                    // Module first: copying sets new times, and the image must
                    // look newer than the module, as it is on a real snapshot.
                    for sub in [crate::boot::MODULE_DIR, "boot"] {
                        let from = self.root.join(sub);
                        if from.exists() {
                            copy_tree(&from, &target.join(sub)).unwrap();
                        }
                    }
                    Ok((true, String::new()))
                }
                _ => Ok((true, String::new())),
            }
        }
    }

    #[test]
    fn undo_entry_is_set_up_in_one_switch() {
        let root = bootable_system();
        let r = root.path();
        let config = Config::parse(BOOT_CONFIG).unwrap();
        let plan = plan::make(&config, r).unwrap();
        assert!(plan.boot.install_module.is_some(), "{:?}", plan.boot);
        let mut runner = Builder {
            root: r.to_path_buf(),
            dracut_ok: true,
            commands: vec![],
        };
        let outcome = switch(&config, r, &plan, &mut runner, &opts()).unwrap();

        // Module first, then the snapshot (so the snapshot has it).
        let dracut = runner
            .commands
            .iter()
            .position(|c| c.starts_with("dracut"))
            .unwrap();
        let snap = runner
            .commands
            .iter()
            .position(|c| c.starts_with("btrfs"))
            .unwrap();
        assert!(dracut < snap, "{:?}", runner.commands);
        assert_eq!(
            std::fs::read_to_string(r.join("boot/initramfs-6.12.58.img")).unwrap(),
            "new image with the undo step"
        );
        assert!(outcome.backup_dir.join("initramfs-6.12.58.img").exists());

        let cfg = std::fs::read_to_string(r.join(crate::boot::CUSTOM_CFG)).unwrap();
        assert!(cfg.contains("linux /@snapshots/bunny-20261116-120000/boot/vmlinuz-6.12.58 root=UUID=1234-abcd ro rootflags=subvol=@ bunny.rollback=bunny-20261116-120000"), "{cfg}");
        assert!(
            outcome
                .notes
                .iter()
                .any(|n| n.contains("undo the last change")),
            "{:?}",
            outcome.notes
        );

        // Next time the module is already there: nothing to plan for [boot].
        assert!(plan::make(&config, r).unwrap().boot.is_empty());
    }

    #[test]
    fn a_failed_startup_image_build_changes_nothing() {
        let root = bootable_system();
        let r = root.path();
        let config = Config::parse(BOOT_CONFIG).unwrap();
        let plan = plan::make(&config, r).unwrap();
        let mut runner = Builder {
            root: r.to_path_buf(),
            dracut_ok: false,
            commands: vec![],
        };
        let err = switch(&config, r, &plan, &mut runner, &opts())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Couldn't rebuild the startup image"), "{err}");
        assert!(!runner.commands.iter().any(|c| c.starts_with("btrfs")));
        assert!(!r.join(crate::boot::MODULE_DIR).exists());
        assert_eq!(
            std::fs::read_to_string(r.join("boot/initramfs-6.12.58.img")).unwrap(),
            "old image"
        );
        assert!(!r.join(crate::boot::CUSTOM_CFG).exists());
    }
}
