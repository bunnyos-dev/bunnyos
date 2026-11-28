//! The command handlers behind `main.rs`'s command list, one module per
//! group, plus what they share: asking questions, running as root, and
//! printing config problems.

mod inspect;
mod install;
mod packages;
mod publish;
mod report;
mod switch;
mod undo;

pub(crate) use inspect::{diff_cmd, explain_cmd, hardware_cmd, lock_cmd};
pub(crate) use install::{install_cmd, Args as InstallArgs};
pub(crate) use packages::{add_cmd, remove_cmd, search_cmd};
pub(crate) use publish::publish;
pub(crate) use report::report;
pub(crate) use switch::{switch, update_cmd};
pub(crate) use undo::{clean_cmd, finish_undo_cmd, generations, rollback_cmd};

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use crate::config::Config;
use crate::{apply, dev, interrupt, rollback, security};

/// `confirm` in the shape apply::Options wants.
pub(crate) fn ask(question: &str, enter_means_yes: bool) -> anyhow::Result<bool> {
    confirm(
        question,
        if enter_means_yes {
            Default::Yes
        } else {
            Default::No
        },
    )
}

/// Re-runs bunny through sudo or doas (whichever this system uses) when
/// changing the real system without root, so nobody has to type
/// `sudo bunny`. Returns the exit code if it did.
///
/// sudo and doas both exit with 1 when they don't run the command (no
/// password, a wrong one, or not allowed; sudo also for Ctrl+C at the
/// question). doas is stopped by Ctrl+C at its question instead. bunny run
/// with `--elevated` never exits with 1 (see `elevated_exit`) and handles
/// Ctrl+C itself, so either means bunny never started as root and nothing was
/// changed: we say so in plain words instead of leaving sudo's or doas's own
/// line (or nothing) as the only explanation.
pub(crate) fn elevate(real_system: bool) -> anyhow::Result<Option<ExitCode>> {
    if !real_system || is_root() {
        return Ok(None);
    }
    let admin = security::admin();
    // The admin tool and bunny under it get Ctrl+C themselves; this copy
    // only waits for them, so it mustn't stop first and leave them talking.
    let _careful = interrupt::Careful::start();
    let status = std::process::Command::new(admin)
        .arg(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        .arg("--elevated")
        .args(
            std::env::var_os(dev::SESSION_ENV)
                .is_some()
                .then_some("--in-session"),
        )
        .status()
        .with_context(|| {
            format!("bunny needs to run as root to change the system, and couldn't start {admin}")
        })?;
    use std::os::unix::process::ExitStatusExt;
    if admin_refused(status.code(), status.signal()) {
        eprintln!("{}", refused_message(admin));
    }
    Ok(Some(passed_on(status.code(), status.signal())))
}

/// The exit code for the bunny that asked sudo or doas to run it as root:
/// success, [`YOUR_INPUT`] (bunny as root stopped because of what you typed,
/// or sudo or doas got no password), or a failure.
fn passed_on(code: Option<i32>, signal: Option<i32>) -> ExitCode {
    match code {
        Some(0) => ExitCode::SUCCESS,
        Some(n) if n == i32::from(YOUR_INPUT) => your_input(),
        _ if admin_refused(code, signal) => your_input(),
        _ => ExitCode::FAILURE,
    }
}

/// Held while a command changes the system: only one at a time, and none
/// while a rollback waits for a restart.
pub(crate) struct Changing {
    _lock: Option<std::fs::File>,
}

/// Checks that the system can be changed now, and keeps others out until
/// the returned value is dropped. Only on the real system. `pending_ok`:
/// fine to run while a rollback waits for a restart (`bunny clean`).
pub(crate) fn changing(real_system: bool, pending_ok: bool) -> anyhow::Result<Changing> {
    if !real_system {
        return Ok(Changing { _lock: None });
    }
    // A rollback swaps the system at the next start. Changes made before
    // that would go into the system being replaced, and be lost.
    if !pending_ok {
        if let Ok(info) = std::fs::read_to_string("/proc/self/mountinfo") {
            if let Ok((_, subvol)) = rollback::root_mount(&info) {
                if subvol.starts_with("@replaced-") {
                    return Err(apply::nothing_changed(anyhow::anyhow!(
                        "A rollback is waiting for a restart, so nothing was changed.\n  \
                         Changes made now would go into the system that's being replaced.\n  \
                         Restart first: {} reboot",
                        security::admin()
                    )));
                }
            }
        }
    }
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open("/run/bunny.lock")
        .context("Couldn't create /run/bunny.lock")?;
    // SAFETY: flock on a file descriptor we own.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(apply::nothing_changed(anyhow::anyhow!(
            "Another bunny command is changing this system right now, so nothing was changed.\n  \
             Wait for it to finish, then try again."
        )));
    }
    Ok(Changing { _lock: Some(file) })
}

/// The exit code bunny uses for a failure when it was re-run as root by
/// `elevate`: anything but 1, which sudo and doas use for "didn't run it".
pub(crate) const ELEVATED_FAILURE: u8 = 3;

/// The exit code for "stopped because of what you typed, not a problem in
/// bunny": a package name that doesn't exist, a mistake in the config, or
/// no password at sudo's or doas's question (Ctrl+C included). Nothing was
/// changed. Developer mode doesn't report these by itself.
pub(crate) const YOUR_INPUT: u8 = 4;

/// [`YOUR_INPUT`] as an exit code.
pub(crate) fn your_input() -> ExitCode {
    ExitCode::from(YOUR_INPUT)
}

/// bunny's exit code when it was re-run as root (`--elevated`): a failure
/// becomes `ELEVATED_FAILURE`, so `elevate` can tell it apart from sudo or
/// doas refusing. [`YOUR_INPUT`] is kept, so the bunny that asked for admin
/// rights can pass it on.
pub(crate) fn elevated_exit(code: ExitCode, elevated: bool) -> ExitCode {
    if elevated && code != ExitCode::SUCCESS && code != your_input() {
        ExitCode::from(ELEVATED_FAILURE)
    } else {
        code
    }
}

/// Whether sudo or doas refused to run bunny (exit code 1, or doas stopped by
/// Ctrl+C at its password question), rather than bunny itself failing as root.
fn admin_refused(code: Option<i32>, signal: Option<i32>) -> bool {
    code == Some(1) || signal == Some(libc::SIGINT)
}

/// What to say when sudo or doas didn't run bunny as root.
fn refused_message(admin: &str) -> String {
    format!(
        "✗ bunny needs admin rights to change the system, and {admin} didn't give them,\n  \
         so nothing was changed.\n  \
         Run the same command again and type your password when {admin} asks for it.\n  \
         If your account isn't an admin on this computer, ask someone who is."
    )
}

/// Whether we're running as root (checks who owns our own /proc entry).
pub(crate) fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// What pressing Enter means at a yes/no question.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Default {
    /// For changes that can be undone (a snapshot is taken first).
    Yes,
    /// For changes that can't be undone (no snapshot, erasing a disk).
    No,
}

/// Asks a yes/no question. Anything that isn't clearly yes or no (like a
/// pasted command) is asked again instead of being guessed.
pub(crate) fn confirm(question: &str, default: Default) -> anyhow::Result<bool> {
    use std::io::Write;
    let hint = match default {
        Default::Yes => "[Y/n]",
        Default::No => "[y/N]",
    };
    print!("{question} {hint} ");
    loop {
        std::io::stdout().flush()?;
        let Some(answer) = interrupt::read_line()? else {
            if interrupt::happened() {
                println!("\n(Ctrl+C: taken as no.)");
            }
            return Ok(false); // no more input, or Ctrl+C: never assume yes
        };
        match parse_answer(&answer, default) {
            Some(false) if default == Default::No && answer.trim().is_empty() => {
                println!(
                    "(Enter means no here, because this can't be undone. \
                     To go ahead, run it again and type y.)"
                );
                return Ok(false);
            }
            Some(yes) => return Ok(yes),
            None => print!(
                "Didn't catch that (got \"{}\"). Type y to continue or n to cancel: ",
                answer.trim()
            ),
        }
    }
}

/// `Some(true)` for yes, `Some(false)` for no, Enter means `default`,
/// `None` for anything else.
pub(crate) fn parse_answer(answer: &str, default: Default) -> Option<bool> {
    match answer.trim().to_lowercase().as_str() {
        "" => Some(default == Default::Yes),
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// `YYYYMMDD-HHMMSS` in UTC, for naming snapshots and backups.
pub(crate) fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Says that this computer's host file isn't part of the plan.
pub(crate) fn host_left_out_note(config: &Path, left_out: &Path) -> String {
    let folder = config
        .parent()
        .unwrap_or(Path::new("."))
        .join("hosts")
        .display()
        .to_string();
    let name = left_out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "  ⚠ This computer's host file ({}) isn't included: {folder} has no {name}.\n    \
         Packages only this computer has (like GRUB and the kernel) will show as\n    \
         \"would be removed\". Copy it there to include it.",
        left_out.display()
    )
}

/// Prints any problems in the config. Returns failure if there were some.
pub(crate) fn report_problems(cfg: &Config, path: &std::path::Path) -> ExitCode {
    let problems = cfg.problems();
    let shown = match &cfg.host {
        Some(host) if host.borrowed => format!(
            "{} (with this computer's {}, since there's no hosts/ folder next to it)",
            path.display(),
            host.path.display()
        ),
        Some(host) => format!(
            "{} (with {}, for this computer)",
            path.display(),
            host.path.display()
        ),
        None => path.display().to_string(),
    };
    if problems.is_empty() {
        println!("✓ {shown} looks good.");
        if let Some(left_out) = &cfg.host_left_out {
            println!("{}", host_left_out_note(path, left_out));
        }
        return ExitCode::SUCCESS;
    }
    let count = problems.len();
    eprintln!(
        "✗ Found {count} problem{} in {shown}:\n",
        if count == 1 { "" } else { "s" },
    );
    for problem in problems {
        eprintln!("  • {problem}\n");
    }
    ExitCode::FAILURE
}

pub(crate) fn runner_for(real_system: bool) -> Box<dyn apply::Runner> {
    if real_system {
        Box::new(apply::SystemRunner)
    } else {
        Box::new(apply::PrintRunner)
    }
}

pub(crate) fn read_config_text(config: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(config).with_context(|| {
        format!(
            "Couldn't read {}.\n  If you don't have a config yet, make one from this system: bunny import -o {}",
            config.display(),
            config.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        admin_refused, elevated_exit, parse_answer, passed_on, refused_message, your_input,
        Default, ELEVATED_FAILURE, YOUR_INPUT,
    };
    use std::process::ExitCode;

    #[test]
    fn says_when_this_computers_host_file_is_left_out() {
        let note = super::host_left_out_note(
            std::path::Path::new("/home/you/test/bunny.toml"),
            std::path::Path::new("/etc/bunny/hosts/desktop.toml"),
        );
        assert!(
            note.contains(
                "This computer's host file (/etc/bunny/hosts/desktop.toml) isn't included: \
                 /home/you/test/hosts has no desktop.toml."
            ),
            "{note}"
        );
        assert!(note.contains("would be removed"), "{note}");
    }

    #[test]
    fn sudo_refusing_is_told_apart_from_bunny_failing() {
        // sudo and doas exit with 1 when they don't run the command.
        assert!(admin_refused(Some(1), None));
        // doas is stopped by Ctrl+C at its password question.
        assert!(admin_refused(None, Some(libc::SIGINT)));
        // bunny as root: success, its own failure, a crash, killed.
        assert!(!admin_refused(Some(0), None));
        assert!(!admin_refused(Some(ELEVATED_FAILURE as i32), None));
        assert!(!admin_refused(Some(101), None));
        assert!(!admin_refused(None, Some(libc::SIGKILL)));
        assert!(!admin_refused(None, Some(libc::SIGTERM)));
    }

    #[test]
    fn your_own_mistakes_keep_their_exit_code_through_sudo() {
        // bunny as root stopped over a name that doesn't exist: passed on.
        assert_eq!(elevated_exit(your_input(), true), your_input());
        assert_eq!(passed_on(Some(i32::from(YOUR_INPUT)), None), your_input());
        // sudo got no password (Ctrl+C at its question): your input too.
        assert_eq!(passed_on(Some(1), None), your_input());
        // bunny's own failures and crashes stay failures.
        assert_eq!(
            passed_on(Some(i32::from(ELEVATED_FAILURE)), None),
            ExitCode::FAILURE
        );
        assert_eq!(passed_on(Some(101), None), ExitCode::FAILURE);
        assert_eq!(passed_on(None, None), ExitCode::FAILURE);
        assert_eq!(passed_on(Some(0), None), ExitCode::SUCCESS);
        // doas stopped by Ctrl+C at its password question: your input, too.
        assert_eq!(passed_on(None, Some(libc::SIGINT)), your_input());
    }

    #[test]
    fn bunny_as_root_never_fails_with_1() {
        assert_eq!(
            elevated_exit(ExitCode::FAILURE, true),
            ExitCode::from(ELEVATED_FAILURE)
        );
        assert_eq!(elevated_exit(ExitCode::SUCCESS, true), ExitCode::SUCCESS);
        // Run directly (not through elevate), failures stay 1.
        assert_eq!(elevated_exit(ExitCode::FAILURE, false), ExitCode::FAILURE);
    }

    #[test]
    fn refused_message_says_nothing_changed_and_what_to_do() {
        let text = refused_message("sudo");
        assert!(text.contains("sudo didn't give them"));
        assert!(text.contains("nothing was changed"));
        assert!(text.contains("type your password when sudo asks"));
        assert!(refused_message("doas").contains("when doas asks"));
    }

    #[test]
    fn answers() {
        assert_eq!(parse_answer("y\n", Default::No), Some(true));
        assert_eq!(parse_answer("YES", Default::No), Some(true));
        assert_eq!(parse_answer("n", Default::Yes), Some(false));
        assert_eq!(
            parse_answer("doas rc-service cronie status", Default::Yes),
            None
        );
    }

    #[test]
    fn enter_means_the_default() {
        assert_eq!(parse_answer("\n", Default::Yes), Some(true));
        assert_eq!(parse_answer("\n", Default::No), Some(false));
    }
}
