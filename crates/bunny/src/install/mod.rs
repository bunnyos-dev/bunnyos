//! `bunny install`: put Bunny Linux on a computer, from the live USB.
//!
//! Part 1, guided: pick a disk, see exactly what will be erased, type the
//! disk's name to confirm, and get the standard layout (encrypted by
//! default) with a minimal system on it: OpenRC, a kernel, networking, one
//! user with sudo, and bunny. Packages come from Gentoo's binary host.
//!
//! Part 2, the config: the new system gets a `bunny.toml` (a starter one from
//! your answers, or yours with `--config`) and `hosts/<host name>.toml`, and
//! its first `bunny switch` and `bunny lock` run before you restart.
//!
//! - `disks`: which disks can be used, and what's on them
//! - `layout`: the partitions, encryption, subvolumes and fstab
//! - `system`: the base system, its settings, and the rest inside it
//! - `starter`: the starter `bunny.toml` and this computer's host file
//! - `first_config`: your config in the new system, its first switch, and
//!   its git history
//! - `mounted`: `--mounted`, partitions you made and mounted yourself
//! - `quiet`: a real install's runner: one line per step, the tools'
//!   output in the install's log

pub mod disks;
pub mod first_config;
pub mod layout;
pub mod mounted;
pub mod questions;
pub mod quiet;
pub mod starter;
pub mod system;

use anyhow::Result;

use crate::apply::Runner;
use layout::Choices;
use system::Account;

/// Does the whole install, in order. On a failure after the disk was
/// touched, everything is unmounted and locked again, so running it again
/// starts cleanly.
pub fn run(
    choices: &Choices,
    account: &Account,
    password: &str,
    config: &first_config::Source,
    edit: bool,
    runner: &mut dyn Runner,
) -> Result<Finished> {
    let home_key = system::home_key_path();
    let result = (|| -> Result<Finished> {
        let uuids = match &choices.mounted {
            // Your own partitions: nothing to prepare.
            Some(mounted) => mounted.uuids(),
            None => {
                if choices.home_disk.is_some() && choices.encrypt {
                    make_key(&home_key, runner)?;
                }
                layout::prepare(choices, password, &home_key, runner)?;
                layout::read_uuids(choices, runner)?
            }
        };
        let offset = system::resume_offset(choices, runner)?;
        let cmdline = choices.cmdline(&uuids, offset.as_deref());
        system::unpack_stage3(choices, runner)?;
        system::configure(choices, account, &uuids, &cmdline, &home_key, runner)?;
        system::install_inside(choices, account, password, runner)?;
        system::tidy_boot_menu(choices, runner)?;
        let config_path = first_config::put_in_place(config, choices, account, &uuids, runner)?;
        let mut edited_ok = true;
        if edit {
            // Problems left after editing: the starter settings already in
            // the new system still work, so this doesn't stop the install.
            if let Err(err) = first_config::edit(&config_path, &account.hostname, runner) {
                println!("\n⚠ {err:#}. bunny didn't apply it; fix it after restarting.");
                edited_ok = false;
            }
        }
        let switched =
            edited_ok && first_config::first_switch(choices, account, &config_path, runner)?;
        // After the switch, so the first commit has bunny.lock too.
        first_config::start_history(config, choices, account, runner)?;
        Ok(Finished { switched })
    })();
    runner.steps_finished(result.is_ok());
    if result.is_ok() {
        keep_log(runner.log_file().as_deref(), choices, runner);
        let args: Vec<String> = std::env::args().skip(1).collect();
        keep_recording(
            crate::dev::current_recording().as_deref(),
            &args,
            choices,
            account,
            runner,
        );
    }
    let _ = system::finish(choices, runner);
    first_config::clean_up(config, runner);
    if !runner.dry() {
        let _ = std::fs::remove_file(&home_key);
    }
    result
}

/// Copies the install's log (what the tools printed) into the new system's
/// `/var/log/bunny/`, so it's still there after restarting. Never stops the
/// install.
fn keep_log(log: Option<&std::path::Path>, choices: &Choices, runner: &dyn Runner) {
    let (Some(log), false) = (log, runner.dry()) else {
        return;
    };
    let Some(name) = log.file_name() else {
        return;
    };
    let dir = choices.mnt.join("var/log/bunny");
    if std::fs::create_dir_all(&dir).is_ok() && std::fs::copy(log, dir.join(name)).is_ok() {
        println!(
            "\n  What each tool printed is in /var/log/bunny/{} (in the new system too).",
            name.to_string_lossy()
        );
    }
}

/// When this install is being recorded (developer mode, on by default on a
/// preview USB), copies the recording into your home folder in the new
/// system, so `bunny report` there sends it after the first start. Never
/// stops the install.
fn keep_recording(
    log: Option<&std::path::Path>,
    args: &[String],
    choices: &Choices,
    account: &Account,
    runner: &mut dyn Runner,
) {
    let Some(log) = log else {
        return;
    };
    let user = &account.username;
    let home = format!("/home/{user}");
    let sessions = ".local/state/bunny/sessions";
    say(runner, "Keeping the recording of this install (developer mode)", "Copies the recording of this install into your home folder in the new system,\nso `bunny report` can send it after the first start.");
    if runner.dry() {
        would(
            runner,
            "copy",
            &format!("{} to {home}/{sessions}/", log.display()),
        );
        return;
    }
    let to = choices.mnt.join(&home[1..]).join(sessions);
    let copied = crate::dev::copy_recording(log, &to, args).and_then(|_| {
        let owned = runner.run(&[
            "chroot".into(),
            choices.mnt.to_string_lossy().into_owned(),
            "chown".into(),
            "-R".into(),
            format!("{user}:"),
            format!("{home}/.local"),
        ])?;
        if owned {
            Ok(())
        } else {
            anyhow::bail!("couldn't give it to {user}")
        }
    });
    match copied {
        Ok(()) => println!(
            "  ✓ It's in {home}/{sessions}/. After restarting, `bunny report` sends it\n    \
             (it shows what it sends and asks first)."
        ),
        Err(err) => println!(
            "  ⚠ Couldn't copy it into the new system ({err:#}).\n    \
             It's still on the USB, in {}, until you restart.",
            log.display()
        ),
    }
}

/// Starts a step: `==> Partitioning /dev/vda`. On a plain-words dry run
/// (`--dry-run` without `--verbose`) it's followed by what the step does,
/// since its commands aren't shown.
pub(crate) fn say(runner: &dyn Runner, title: &str, what: &str) {
    runner.step_started(title);
    print!("{}", heading(runner.describes_steps(), title, what));
}

fn heading(describe: bool, title: &str, what: &str) -> String {
    let mut out = format!("\n==> {title}\n");
    if describe {
        for line in what.lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out
}

/// `  (would write) /etc/fstab` on a dry run that lists every command and
/// file (`--dry-run --verbose`); nothing on a plain-words one.
pub(crate) fn would(runner: &dyn Runner, verb: &str, what: &str) {
    if !runner.describes_steps() {
        println!("  (would {verb}) {what}");
    }
}

/// How the install ended.
pub struct Finished {
    /// The first `bunny switch` ran and the lock was written.
    pub switched: bool,
}

/// A random key for the /home disk, readable only by root.
fn make_key(path: &std::path::Path, runner: &dyn Runner) -> Result<()> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    if runner.dry() {
        would(
            runner,
            "make",
            &format!("a random key for the /home disk in {}", path.display()),
        );
        return Ok(());
    }
    let mut key = [0u8; 64];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(path)?;
    use std::io::Write;
    file.write_all(&key)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records commands and does nothing.
    #[derive(Default)]
    struct Recorded(Vec<String>, bool);
    impl Runner for Recorded {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.0.push(cmd.join(" "));
            Ok(true)
        }
        fn dry(&self) -> bool {
            self.1
        }
    }

    fn account() -> Account {
        Account {
            username: "you".into(),
            hostname: "laptop".into(),
            ssh: false,
            timezone: "UTC".into(),
            keymap: "us".into(),
        }
    }

    #[test]
    fn a_recorded_install_is_copied_into_your_home_folder() {
        let usb = tempfile::tempdir().unwrap();
        let log = usb.path().join("20261119-201500-install.log");
        std::fs::write(&log, "recorded\n").unwrap();
        let mnt = tempfile::tempdir().unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        let mut r = Recorded::default();
        keep_recording(
            Some(&log),
            &["install".into()],
            &choices,
            &account(),
            &mut r,
        );
        let sessions = mnt.path().join("home/you/.local/state/bunny/sessions");
        assert!(sessions.join("20261119-201500-install.log").is_file());
        assert!(sessions.join("20261119-201500-install.toml").is_file());
        assert_eq!(
            r.0,
            [format!(
                "chroot {} chown -R you: /home/you/.local",
                mnt.path().display()
            )]
        );
    }

    #[test]
    fn nothing_is_copied_when_not_recording_or_on_a_dry_run() {
        let mnt = tempfile::tempdir().unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        let mut r = Recorded::default();
        keep_recording(None, &[], &choices, &account(), &mut r);
        let log = mnt.path().join("x-install.log");
        std::fs::write(&log, "").unwrap();
        let mut dry = Recorded(Vec::new(), true);
        keep_recording(Some(&log), &[], &choices, &account(), &mut dry);
        assert!(r.0.is_empty() && dry.0.is_empty());
        assert!(!mnt.path().join("home").exists());
    }

    #[test]
    fn a_plain_dry_run_says_what_each_step_does() {
        let text = heading(
            true,
            "Partitioning /dev/vda",
            "Erases the old partition table.\nMakes two partitions.",
        );
        assert_eq!(
            text,
            "\n==> Partitioning /dev/vda\n    Erases the old partition table.\n    Makes two partitions.\n"
        );
        // A real install (and --verbose) shows only the step's name.
        assert_eq!(
            heading(false, "Partitioning /dev/vda", "Erases…"),
            "\n==> Partitioning /dev/vda\n"
        );
    }

    #[test]
    fn a_plain_dry_run_goes_through_every_step() {
        // The whole install against nothing: every step runs, no command
        // does anything, and nothing on this computer is touched.
        let choices = Choices {
            disk: "/dev/vda".into(),
            encrypt: true,
            home_disk: Some("/dev/vdb".into()),
            swap_mib: 1024,
            hibernate: true,
            mnt: "/nonexistent/bunny-test".into(),
            ..Default::default()
        };
        let account = Account {
            username: "you".into(),
            hostname: "laptop".into(),
            ssh: false,
            timezone: "UTC".into(),
            keymap: "us".into(),
        };
        let mut runner = crate::apply::StepsRunner;
        let finished = run(
            &choices,
            &account,
            "a password",
            &first_config::Source::Starter,
            false,
            &mut runner,
        )
        .unwrap();
        assert!(finished.switched);
        assert!(!std::path::Path::new("/nonexistent/bunny-test").exists());
    }
}
