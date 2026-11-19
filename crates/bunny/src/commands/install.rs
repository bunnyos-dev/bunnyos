//! `bunny install`: the questions, the preview and the confirmation. The
//! work itself is in `install/`.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context};

use super::is_root;
use crate::apply::{self, Runner};
use crate::install::disks::{self, Disk};
use crate::install::first_config;
use crate::install::layout::{self, Choices};
use crate::install::system::Account;
use crate::interrupt;

/// What was given on the command line.
pub(crate) struct Args {
    pub disk: Option<String>,
    pub no_encrypt: bool,
    pub efi_size: Option<String>,
    pub swap: Option<String>,
    pub hibernate: bool,
    pub home_disk: Option<String>,
    pub ssh: bool,
    /// Stop so the config can be edited before it's applied.
    pub edit: bool,
    /// Your own config: a bunny.toml, or a git repository.
    pub config: Option<String>,
    pub dry_run: bool,
    /// Read the list of disks from this file instead of lsblk (for testing).
    pub lsblk: Option<PathBuf>,
}

fn ask_line(question: &str) -> anyhow::Result<String> {
    print!("{question}");
    std::io::stdout().flush()?;
    match interrupt::read_line()? {
        Some(line) => Ok(line.trim().to_string()),
        None => bail!("Cancelled. Nothing was changed."),
    }
}

/// Reads a line without showing what's typed.
fn ask_secret(question: &str) -> anyhow::Result<String> {
    print!("{question}");
    std::io::stdout().flush()?;
    let fd = libc::STDIN_FILENO;
    // SAFETY: termios is plain data; tcgetattr fills it in.
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    let is_tty = unsafe { libc::tcgetattr(fd, &mut old) } == 0;
    if is_tty {
        let mut quiet = old;
        quiet.c_lflag &= !libc::ECHO;
        quiet.c_lflag |= libc::ECHONL;
        // SAFETY: setting attributes read from the same terminal.
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) };
    }
    let line = interrupt::read_line();
    if is_tty {
        // SAFETY: as above, restoring what was there.
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &old) };
    }
    match line? {
        Some(l) => Ok(l.trim_end_matches(['\n', '\r']).to_string()),
        None => {
            println!();
            bail!("Cancelled. Nothing was changed.")
        }
    }
}

/// Lowercase letters, digits, `-` and `_`, starting with a letter or `_`.
pub(crate) fn valid_username(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && name.len() <= 32
        && name != "root"
}

/// Letters, digits and `-`, not starting or ending with `-`.
pub(crate) fn valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A time zone name like `America/Toronto`: a file under /usr/share/zoneinfo
/// when that's there to check (it is on the USB).
pub(crate) fn valid_timezone(name: &str, zoneinfo: &std::path::Path) -> bool {
    let shaped = !name.is_empty()
        && !name.starts_with('/')
        && !name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c));
    shaped && (!zoneinfo.is_dir() || zoneinfo.join(name).is_file())
}

/// A console keyboard layout like `us`, `uk` or `de-latin1`.
pub(crate) fn valid_keymap(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// RAM size in MiB, from /proc/meminfo.
fn ram_mib() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info
        .lines()
        .find(|l| l.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    // Rounded up to a whole GiB.
    Some(kb.div_ceil(1024 * 1024) * 1024)
}

fn find<'a>(disks: &'a [Disk], given: &str) -> Option<&'a Disk> {
    disks
        .iter()
        .find(|d| d.path == given || d.name == given || format!("/dev/{}", d.name) == given)
}

fn pick_disk(disks: &[Disk]) -> anyhow::Result<Disk> {
    let usable: Vec<&Disk> = disks.iter().filter(|d| d.unusable().is_none()).collect();
    println!("Disks on this computer:\n");
    for (i, d) in usable.iter().enumerate() {
        println!("  {}. {}", i + 1, d.describe());
    }
    for d in disks.iter().filter(|d| d.unusable().is_some()) {
        println!(
            "     {}   (can't use: {})",
            d.describe(),
            d.unusable().unwrap_or_default()
        );
    }
    match usable.len() {
        0 => bail!(
            "There's no disk Bunny Linux can be installed on.\n  \
             Disks that are in use (mounted, or unlocked) can't be erased: unmount them first."
        ),
        1 => {
            println!();
            Ok(usable[0].clone())
        }
        n => loop {
            let answer = ask_line(&format!("\nWhich disk? (1–{n}): "))?;
            match answer.parse::<usize>() {
                Ok(i) if (1..=n).contains(&i) => break Ok(usable[i - 1].clone()),
                _ => println!("Type a number from the list."),
            }
        },
    }
}

pub(crate) fn install_cmd(args: Args) -> anyhow::Result<ExitCode> {
    if !args.dry_run {
        if !is_root() {
            bail!(
                "bunny install needs to run as root.\n  \
                 On the Bunny Linux USB you already are; elsewhere, use sudo."
            );
        }
        if !std::path::Path::new("/sys/firmware/efi").is_dir() {
            bail!(
                "This computer started the USB in the old BIOS mode, but Bunny Linux needs UEFI.\n  \
                 Restart, open the boot menu (often F12, F11 or Esc) and pick the USB entry\n  \
                 that starts with \"UEFI\"."
            );
        }
    }

    // Your own config is fetched and checked before anything is erased.
    let mut check_runner: Box<dyn Runner> = if args.dry_run {
        Box::new(apply::PrintRunner)
    } else {
        Box::new(apply::SystemRunner)
    };
    let source = first_config::prepare(args.config.as_deref(), check_runner.as_mut())?;

    let json = match &args.lsblk {
        Some(file) => std::fs::read_to_string(file)
            .with_context(|| format!("Couldn't read {}", file.display()))?,
        None => apply::SystemRunner
            .output(
                &disks::LSBLK
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
            )?
            .context("Couldn't list the disks (lsblk failed)")?,
    };
    let all = disks::parse(&json)?;

    let disk = match &args.disk {
        Some(given) => match find(&all, given) {
            Some(d) => d.clone(),
            None => bail!(
                "There's no disk called {given}. Run bunny install without --disk to see them."
            ),
        },
        None => pick_disk(&all)?,
    };
    if let Some(why) = disk.unusable() {
        bail!("{} can't be used: {why}.", disk.path);
    }
    let home_disk = match &args.home_disk {
        None => None,
        Some(given) => {
            let Some(d) = find(&all, given) else {
                bail!("There's no disk called {given} for /home.");
            };
            if d.path == disk.path {
                bail!("/home needs a different disk from the system's.");
            }
            if let Some(why) = d.unusable() {
                bail!("{} can't be used for /home: {why}.", d.path);
            }
            Some(d.clone())
        }
    };

    let mut choices = Choices {
        disk: disk.path.clone(),
        encrypt: !args.no_encrypt,
        home_disk: home_disk.as_ref().map(|d| d.path.clone()),
        ..Default::default()
    };
    if let Some(size) = &args.efi_size {
        choices.efi_mib = layout::parse_size(size)
            .filter(|m| *m >= 512)
            .with_context(|| {
                format!("--efi-size: \"{size}\" isn't a size of at least 512M (try 2G)")
            })?;
    }
    if let Some(size) = &args.swap {
        choices.swap_mib = layout::parse_size(size)
            .with_context(|| format!("--swap: \"{size}\" isn't a size (try 8G)"))?;
    }
    if args.hibernate {
        choices.hibernate = true;
        if choices.swap_mib == 0 {
            // Hibernating writes memory to the swapfile, so it needs to fit.
            choices.swap_mib = ram_mib().unwrap_or(8192);
        }
    }

    // The preview: exactly what goes, and what you get.
    println!("This ERASES everything on {}:", disk.describe());
    for line in disk.contents() {
        println!("  - {line}");
    }
    if let Some(home) = &home_disk {
        println!("\nAnd everything on {} (for /home):", home.describe());
        for line in home.contents() {
            println!("  - {line}");
        }
    }
    println!("\nWhat you'll get:");
    for line in choices.summary() {
        println!("  {line}");
    }
    println!(
        "  A minimal system: OpenRC, a kernel, networking, your account with sudo, and bunny.\n  \
         No desktop yet: adding one with bunny is the first thing the Guide shows."
    );
    match &source {
        first_config::Source::Starter => println!(
            "  A starter config in /etc/bunny/bunny.toml, applied before you restart{}.",
            if args.edit {
                " (you edit it first)"
            } else {
                ""
            }
        ),
        first_config::Source::File(path) => println!(
            "  Your config from {}, applied before you restart{}.",
            path.display(),
            if args.edit {
                " (you edit it first)"
            } else {
                ""
            }
        ),
        first_config::Source::Repo { url, .. } => println!(
            "  Your config from {url}, applied before you restart{}.",
            if args.edit {
                " (you edit it first)"
            } else {
                ""
            }
        ),
    }

    if args.dry_run {
        println!("\nThis is a dry run: nothing will be erased, and commands are only printed.");
    } else {
        println!("\nThis can't be undone.");
        let answer = ask_line(&format!(
            "To erase {} and install, type its name ({}): ",
            disk.path, disk.name
        ))?;
        if answer != disk.name && answer != disk.path {
            println!("That's not the disk's name, so nothing was changed.");
            return Ok(ExitCode::SUCCESS);
        }
    }

    // The account.
    println!();
    let username = loop {
        let name = ask_line("Your user name (lowercase, e.g. you): ")?;
        if valid_username(&name) {
            break name;
        }
        println!("Use lowercase letters, digits, - and _, starting with a letter (and not root).");
    };
    let hostname = loop {
        let name = ask_line("A name for this computer (e.g. laptop): ")?;
        if valid_hostname(&name) {
            break name;
        }
        println!("Use letters, digits and -, like laptop or my-desktop.");
    };
    let zoneinfo = std::path::Path::new("/usr/share/zoneinfo");
    let timezone = loop {
        let name = ask_line("Time zone (e.g. America/Toronto, Europe/London; Enter for UTC): ")?;
        let name = if name.is_empty() {
            "UTC".to_string()
        } else {
            name
        };
        if valid_timezone(&name, zoneinfo) {
            break name;
        }
        println!("That isn't a time zone name. Use Region/City, like America/Toronto.");
    };
    let keymap = loop {
        let name = ask_line("Keyboard layout (e.g. us, uk, de, fr; Enter for us): ")?;
        let name = if name.is_empty() {
            "us".to_string()
        } else {
            name
        };
        if valid_keymap(&name) {
            break name;
        }
        println!("Use a layout name like us, uk, de or fr.");
    };
    if choices.encrypt {
        println!(
            "\nOne password, for your account and for unlocking the disk when the computer starts."
        );
    }
    let password = loop {
        let first = ask_secret("Password: ")?;
        if first.len() < 8 {
            println!("Use at least 8 characters.");
            continue;
        }
        if ask_secret("Again: ")? == first {
            break first;
        }
        println!("Those didn't match. Try again.");
    };

    let account = Account {
        username,
        hostname,
        ssh: args.ssh,
        timezone,
        keymap,
    };
    let mut runner: Box<dyn Runner> = if args.dry_run {
        Box::new(apply::PrintRunner)
    } else {
        Box::new(apply::SystemRunner)
    };
    let finished = crate::install::run(
        &choices,
        &account,
        &password,
        &source,
        args.edit,
        runner.as_mut(),
    )?;

    if args.dry_run {
        println!("\n✓ Dry run finished. Nothing was changed.");
    } else {
        println!("\n✓ Bunny Linux is installed on {}.", disk.path);
        if finished.switched {
            println!(
                "Your config is in /etc/bunny/bunny.toml (and this computer's settings in\n\
                 /etc/bunny/hosts/{}.toml). It's already applied: bunny status shows ✓.",
                account.hostname
            );
        }
        println!("Next:");
        println!("  1. Restart: reboot");
        println!("  2. Take out the USB stick when the screen goes dark.");
        if choices.encrypt {
            println!(
                "  3. Type your password to unlock the disk, then log in as {}.",
                account.username
            );
        } else {
            println!("  3. Log in as {}.", account.username);
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_names() {
        assert!(valid_username("you"));
        assert!(valid_username("_build-1"));
        assert!(!valid_username("You"));
        assert!(!valid_username("1you"));
        assert!(!valid_username("root"));
        assert!(!valid_username("a b"));
        assert!(valid_hostname("laptop"));
        assert!(valid_hostname("my-desktop2"));
        assert!(!valid_hostname("-laptop"));
        assert!(!valid_hostname("lap top"));
        assert!(!valid_hostname(""));
    }

    #[test]
    fn checks_time_zones_and_keyboards() {
        let zoneinfo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(zoneinfo.path().join("America")).unwrap();
        std::fs::write(zoneinfo.path().join("America/Toronto"), "").unwrap();
        std::fs::write(zoneinfo.path().join("UTC"), "").unwrap();
        assert!(valid_timezone("America/Toronto", zoneinfo.path()));
        assert!(valid_timezone("UTC", zoneinfo.path()));
        assert!(!valid_timezone("America/Atlantis", zoneinfo.path()));
        assert!(!valid_timezone("../etc/passwd", zoneinfo.path()));
        assert!(!valid_timezone("/UTC", zoneinfo.path()));
        // Without zoneinfo to check against, the shape is enough.
        let none = std::path::Path::new("/nonexistent");
        assert!(valid_timezone("Europe/London", none));
        assert!(!valid_timezone("Europe London", none));
        assert!(valid_keymap("us") && valid_keymap("de-latin1"));
        assert!(!valid_keymap("") && !valid_keymap("us uk") && !valid_keymap("a/b"));
    }
}
