//! Putting a system on the prepared disk: Gentoo's stage3 (the base
//! system), Portage settings, fstab, then the rest inside the new system
//! (`inside.sh`, run with chroot): packages, the kernel, the boot menu and
//! the user's account.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::layout::{Choices, Uuids, HOME_KEY};
use super::say;
use crate::apply::Runner;

pub const MIRROR: &str = "https://distfiles.gentoo.org/releases/amd64/autobuilds";
const INSIDE: &str = include_str!("inside.sh");
/// Turns swap on after `/swap` is mounted (OpenRC's swap service runs too early).
const SWAPFILES: &str = include_str!("swapfiles.initd");
const ZRAM: &str = include_str!("zram.initd");

/// What goes into the new system besides the disk layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub username: String,
    pub hostname: String,
    /// Start the SSH server at boot (off unless asked for).
    pub ssh: bool,
    /// e.g. `America/Toronto` (a name under /usr/share/zoneinfo).
    pub timezone: String,
    /// The console keyboard layout, e.g. `us`.
    pub keymap: String,
}

/// The stage3 path from `latest-stage3-amd64-openrc.txt` (a signed text
/// file whose first non-comment line names the newest tarball).
pub fn latest_stage3(listing: &str) -> Option<String> {
    listing
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .find(|p| p.contains("stage3-amd64-openrc-") && p.ends_with(".tar.xz"))
        .map(String::from)
}

/// The SHA512 checksum for `file` in a Gentoo `.DIGESTS` file.
pub fn sha512_for(digests: &str, file: &str) -> Option<String> {
    let mut lines = digests.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "# SHA512 HASH" {
            let next = lines.next()?;
            let mut words = next.split_whitespace();
            let (hash, name) = (words.next()?, words.next()?);
            if name == file && hash.len() == 128 {
                return Some(hash.to_lowercase());
            }
        }
    }
    None
}

/// The packages for a minimal Bunny Linux system: what the computer needs
/// to start, then the rest (the same lists `bunny install` writes into the
/// new system's config).
pub fn packages(choices: &Choices) -> Vec<&'static str> {
    let mut out = super::starter::boot_packages(choices);
    out.extend(super::starter::BASE_PACKAGES);
    out
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// Writes a file in the new system, or says it would on a dry run.
fn put(runner: &dyn Runner, path: &Path, text: &str, mode: Option<u32>) -> Result<()> {
    if runner.dry() {
        super::would(runner, "write", &path.display().to_string());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Couldn't create {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("Couldn't write {}", path.display()))?;
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn append(runner: &dyn Runner, path: &Path, text: &str) -> Result<()> {
    if runner.dry() {
        super::would(runner, "add to", &path.display().to_string());
        return Ok(());
    }
    let mut old = std::fs::read_to_string(path).unwrap_or_default();
    old.push_str(text);
    std::fs::write(path, old).with_context(|| format!("Couldn't write {}", path.display()))
}

/// Copies iwd's known networks (`*.psk`, `*.open`, `*.8021x`) from `from`
/// to `to`, readable by root only, as iwd keeps them. They hold Wi-Fi
/// passwords: they go into the new system and nowhere else. Returns how
/// many were copied (`None` when there were none).
pub fn copy_wifi_networks(from: &Path, to: &Path, runner: &dyn Runner) -> Result<Option<usize>> {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(None);
    };
    let networks: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("psk" | "open" | "8021x")
            )
        })
        .collect();
    if networks.is_empty() {
        return Ok(None);
    }
    if runner.dry() {
        super::would(
            runner,
            "copy",
            &format!("{} Wi-Fi network(s) to {}", networks.len(), to.display()),
        );
        return Ok(Some(networks.len()));
    }
    std::fs::create_dir_all(to).with_context(|| format!("Couldn't create {}", to.display()))?;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o700))?;
    for network in &networks {
        let target = to.join(network.file_name().unwrap_or_default());
        std::fs::copy(network, &target)
            .with_context(|| format!("Couldn't copy a Wi-Fi network to {}", to.display()))?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(Some(networks.len()))
}

/// Downloads the newest OpenRC stage3, checks it, and unpacks it into the
/// new system.
pub fn unpack_stage3(choices: &Choices, runner: &mut dyn Runner) -> Result<()> {
    let mnt = &choices.mnt;
    say(
        runner,
        "Setting the clock (downloads check it)",
        "Sets this computer's clock from the internet: secure downloads and\n\
         the prebuilt packages' signatures can only be checked with the right time.",
    );
    let _ = runner.run(&s(&["chronyd", "-q", "server pool.ntp.org iburst"]));

    say(
        runner,
        "Downloading Gentoo's newest base system (stage3)",
        "Downloads the newest OpenRC stage3 (Gentoo's starting point: a small,\n\
         working system) and its checksums from distfiles.gentoo.org.",
    );
    let listing = runner.output(&s(&[
        "wget",
        "-qO-",
        &format!("{MIRROR}/latest-stage3-amd64-openrc.txt"),
    ]))?;
    let path = match listing.as_deref().and_then(latest_stage3) {
        Some(p) => p,
        None if runner.dry() => "<date>/stage3-amd64-openrc-<date>.tar.xz".to_string(),
        None => bail!(
            "Couldn't find the newest stage3 on Gentoo's mirror ({MIRROR}).\n  \
             Check that this computer is online (ping distfiles.gentoo.org), then run bunny install again."
        ),
    };
    let file = path.rsplit('/').next().unwrap_or(&path).to_string();
    let tarball = mnt.join(&file);
    let digests = mnt.join(format!("{file}.DIGESTS"));
    for (url, to) in [
        (format!("{MIRROR}/{path}"), &tarball),
        (format!("{MIRROR}/{path}.DIGESTS"), &digests),
    ] {
        if !runner.run(&s(&[
            "wget",
            "-q",
            "--show-progress",
            "-O",
            &to.to_string_lossy(),
            &url,
        ]))? {
            bail!("Couldn't download {url}. Check the connection, then run bunny install again.");
        }
    }

    say(
        runner,
        "Checking the download",
        "Compares the download with Gentoo's SHA512 checksum, so a damaged\n\
         file is never used.",
    );
    if !runner.dry() {
        let text = std::fs::read_to_string(&digests).context("Couldn't read the checksum file")?;
        let Some(expected) = sha512_for(&text, &file) else {
            bail!("The checksum file doesn't list {file}, so the download can't be checked.");
        };
        let got = runner
            .output(&s(&["sha512sum", &tarball.to_string_lossy()]))?
            .and_then(|o| o.split_whitespace().next().map(String::from))
            .unwrap_or_default();
        if got != expected {
            bail!("The download is damaged (its checksum doesn't match). Run bunny install again.");
        }
    }

    say(
        runner,
        "Unpacking",
        &format!("Unpacks the base system into {}.", mnt.display()),
    );
    if !runner.run(&s(&[
        "tar",
        "xpf",
        &tarball.to_string_lossy(),
        "--xattrs-include=*.*",
        "--numeric-owner",
        "-C",
        &mnt.to_string_lossy(),
    ]))? {
        bail!("Couldn't unpack the base system. Run bunny install again.");
    }
    if !runner.dry() {
        let _ = std::fs::remove_file(&tarball);
        let _ = std::fs::remove_file(&digests);
    }
    Ok(())
}

/// The settings and files the new system needs before its first start.
pub fn configure(
    choices: &Choices,
    account: &Account,
    uuids: &Uuids,
    cmdline: &str,
    home_key: &Path,
    runner: &mut dyn Runner,
) -> Result<()> {
    let mnt = &choices.mnt;
    say(
        runner,
        "Writing the new system's settings",
        "Writes Portage's settings (prebuilt packages from Gentoo, checked\n\
         signatures), fstab (what to mount at startup), the startup image's\n\
         settings, admin rights for your account with sudo, and copies bunny in.",
    );
    append(
        runner,
        &mnt.join("etc/portage/make.conf"),
        "\n# Added by bunny install: prebuilt packages from Gentoo when they exist.\n\
         FEATURES=\"getbinpkg binpkg-request-signature\"\n\
         ACCEPT_LICENSE=\"-* @FREE @BINARY-REDISTRIBUTABLE\"\n\
         GRUB_PLATFORMS=\"efi-64\"\n",
    )?;
    put(
        runner,
        &mnt.join("etc/portage/package.use/bunny-install"),
        "# Written by bunny install.\n\
         sys-kernel/installkernel dracut grub\n\
         app-admin/sudo -sendmail\n",
        None,
    )?;
    let mut dracut = String::from("# Written by bunny install.\nadd_dracutmodules+=\" btrfs ");
    if choices.encrypt {
        dracut.push_str("crypt ");
    }
    if choices.hibernate {
        dracut.push_str("resume ");
    }
    dracut.push_str("\"\n");
    put(
        runner,
        &mnt.join("etc/dracut.conf.d/bunny.conf"),
        &dracut,
        None,
    )?;
    put(runner, &mnt.join("etc/fstab"), &choices.fstab(uuids), None)?;
    if let Some(conf) = choices.dmcrypt_conf(uuids) {
        // Added to /etc/conf.d/dmcrypt inside, once cryptsetup has
        // installed it (writing it now would leave Portage's copy aside).
        put(
            runner,
            &mnt.join("root/bunny-dmcrypt.conf"),
            &conf,
            Some(0o600),
        )?;
        if runner.dry() {
            super::would(
                runner,
                "copy",
                &format!("the /home disk's key to /{HOME_KEY}"),
            );
        } else {
            let to = mnt.join(HOME_KEY);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
            std::fs::copy(home_key, &to).context("Couldn't copy the /home disk's key")?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o400))?;
        }
    }
    // A plain folder, so it isn't made a nested subvolume on first boot
    // (those aren't part of undo).
    if !runner.dry() {
        std::fs::create_dir_all(mnt.join("srv"))?;
    }
    // Admin rights for the wheel group: the same file bunny's [security] writes.
    let security = crate::security::Security {
        privilege: crate::security::Privilege::Sudo,
        persist: true,
        admins: vec![account.username.clone()],
    };
    put(
        runner,
        &mnt.join("etc/sudoers.d/bunny"),
        &crate::security::render(&security),
        Some(0o440),
    )?;

    // The Wi-Fi networks joined on the USB (with iwctl or `bunny wifi`),
    // so the new system is online at its first start.
    if let Some(n) =
        copy_wifi_networks(Path::new("/var/lib/iwd"), &mnt.join("var/lib/iwd"), runner)?
    {
        say(
            runner,
            "Keeping your Wi-Fi",
            &format!(
                "Copied {n} Wi-Fi network{} you joined on the USB, so the new system\n\
                 connects at its first start.",
                if n == 1 { "" } else { "s" }
            ),
        );
    }
    let mut services = super::starter::services(account);
    services.extend(super::starter::host_services(super::starter::has_wifi(
        Path::new("/"),
    )));
    let mut boot_services = Vec::new();
    if uuids.home_luks.is_some() {
        boot_services.push("dmcrypt");
    }
    if choices.zram {
        put(runner, &mnt.join("etc/init.d/zram"), ZRAM, Some(0o755))?;
        boot_services.push("zram");
    }
    if choices.swap_mib > 0 {
        put(
            runner,
            &mnt.join("etc/init.d/swapfiles"),
            SWAPFILES,
            Some(0o755),
        )?;
        boot_services.push("swapfiles");
    }
    let boot_services = boot_services.join(" ");
    put(
        runner,
        &mnt.join("root/bunny-install.env"),
        &format!(
            "HOST={}\nUSERNAME={}\nTIMEZONE={}\nKEYMAP={}\nCMDLINE=\"{cmdline}\"\nPACKAGES=\"{}\"\nSERVICES=\"{}\"\nBOOT_SERVICES=\"{boot_services}\"\n",
            account.hostname,
            account.username,
            account.timezone,
            account.keymap,
            packages(choices).join(" "),
            services.join(" "),
        ),
        Some(0o600),
    )?;
    put(
        runner,
        &mnt.join("root/bunny-inside.sh"),
        INSIDE,
        Some(0o700),
    )?;

    // bunny itself, so it's there on the first start.
    if runner.dry() {
        super::would(runner, "copy", "bunny to /usr/local/bin/bunny");
    } else {
        let exe = std::env::current_exe().context("Couldn't find bunny's own file to copy")?;
        let to = mnt.join("usr/local/bin/bunny");
        std::fs::create_dir_all(to.parent().unwrap_or(mnt))?;
        std::fs::copy(&exe, &to).context("Couldn't copy bunny into the new system")?;
    }
    Ok(())
}

/// Runs the rest inside the new system, then sets the password.
pub fn install_inside(
    choices: &Choices,
    account: &Account,
    password: &str,
    runner: &mut dyn Runner,
) -> Result<()> {
    let mnt = choices.mnt.to_string_lossy().into_owned();
    let at = |p: &str| choices.mnt.join(p).to_string_lossy().into_owned();
    let _ = runner.run(&s(&[
        "cp",
        "--dereference",
        "/etc/resolv.conf",
        &at("etc/"),
    ]))?;
    for cmd in [
        s(&["mount", "--types", "proc", "/proc", &at("proc")]),
        s(&["mount", "--rbind", "/sys", &at("sys")]),
        s(&["mount", "--make-rslave", &at("sys")]),
        s(&["mount", "--rbind", "/dev", &at("dev")]),
        s(&["mount", "--make-rslave", &at("dev")]),
        s(&["mount", "--bind", "/run", &at("run")]),
        s(&["mount", "--make-slave", &at("run")]),
    ] {
        if !runner.run(&cmd)? {
            bail!(
                "Couldn't prepare the new system ({} failed).",
                cmd.join(" ")
            );
        }
    }
    say(
        runner,
        "Installing inside the new system (prebuilt packages; this takes a while)",
        "Inside the new system: installs the kernel, GRUB (named Bunny Linux),\n\
         networking and the other base packages, prebuilt from Gentoo; sets\n\
         the time zone and keyboard; adds your account and sets its password.",
    );
    if !runner.run(&s(&["chroot", &mnt, "/bin/bash", "/root/bunny-inside.sh"]))? {
        bail!(
            "Installing inside the new system failed (the message is above).\n  \
             The new system isn't finished, so it won't start yet. Fix the problem\n  \
             (often the network), then run bunny install again."
        );
    }
    if !runner.run_with_input(
        &s(&["chroot", &mnt, "chpasswd"]),
        &format!("{}:{password}\n", account.username),
        true,
    )? {
        bail!("Couldn't set the password for {}.", account.username);
    }
    if !runner.dry() {
        let _ = std::fs::remove_file(choices.mnt.join("root/bunny-install.env"));
        let _ = std::fs::remove_file(choices.mnt.join("root/bunny-inside.sh"));
        let _ = std::fs::remove_file(choices.mnt.join("root/bunny-dmcrypt.conf"));
    }
    Ok(())
}

/// Tidies the new system's boot menu (Gentoo's theme, "Bunny Linux
/// GNU/Linux", the "(EFI BootNext)" entries) and rebuilds it. Done here, not
/// only in the first switch, so it happens whatever your config says. The
/// old files are kept in `/var/lib/bunny/backups/as-installed/`. Problems are
/// explained and never stop the install.
pub fn tidy_boot_menu(choices: &Choices, runner: &mut dyn Runner) -> Result<()> {
    use crate::boot::tidy;
    say(
        runner,
        "Tidying the boot menu",
        "Turns off Gentoo's menu theme, names the main entry \"Bunny Linux\" (not\n\
         \"Bunny Linux GNU/Linux\") with \"Advanced options\" under it, and removes\n\
         the \"(EFI BootNext)\" entries. \"UEFI Firmware Settings\" stays.",
    );
    let mnt = &choices.mnt;
    if runner.dry() {
        for file in [
            tidy::DEFAULT_GRUB,
            tidy::OWN_SCRIPT,
            tidy::LINUX_SCRIPT,
            tidy::BOOTNEXT_SCRIPT,
        ] {
            super::would(runner, "change", &mnt.join(file).display().to_string());
        }
    } else {
        let todo = tidy::needed(mnt);
        if todo.is_empty() {
            return Ok(());
        }
        let backup = mnt.join("var/lib/bunny/backups/as-installed");
        if let Err(e) = tidy::apply(mnt, &todo, &backup) {
            println!("  ⚠ Couldn't tidy the boot menu ({e:#}). It still works as it is.");
            return Ok(());
        }
    }
    if !runner.run(&s(&[
        "chroot",
        &mnt.to_string_lossy(),
        "grub-mkconfig",
        "-o",
        "/boot/grub/grub.cfg",
    ]))? {
        println!(
            "  ⚠ Couldn't rebuild the boot menu (the message is above). After restarting, run:\n    \
             sudo grub-mkconfig -o /boot/grub/grub.cfg"
        );
    }
    Ok(())
}

/// The swapfile's offset, for resuming from hibernation.
pub fn resume_offset(choices: &Choices, runner: &mut dyn Runner) -> Result<Option<String>> {
    if !choices.hibernate || choices.swap_mib == 0 {
        return Ok(None);
    }
    let swapfile = choices.mnt.join("swap/swapfile");
    let out = runner.output(&s(&[
        "btrfs",
        "inspect-internal",
        "map-swapfile",
        "-r",
        &swapfile.to_string_lossy(),
    ]))?;
    match out
        .map(|o| o.trim().to_string())
        .filter(|o| o.parse::<u64>().is_ok())
    {
        Some(offset) => Ok(Some(offset)),
        None if runner.dry() => Ok(Some("<offset>".into())),
        None => bail!("Couldn't find where the swapfile starts, which hibernation needs."),
    }
}

/// Unmounts everything and locks the disks again. Best effort: the system
/// is already complete, and a restart unmounts anything left.
pub fn finish(choices: &Choices, runner: &mut dyn Runner) -> Result<()> {
    let mnt = choices.mnt.to_string_lossy().into_owned();
    if choices.mounted.is_some() {
        // Your own partitions stay mounted; only what bunny mounted goes.
        say(
            runner,
            "Unmounting what bunny mounted (your partitions stay mounted)",
            "Unmounts /proc, /sys, /dev and /run from the new system.",
        );
        for dir in ["run", "dev", "sys", "proc"] {
            let _ = runner.run(&s(&[
                "umount",
                "--recursive",
                "--lazy",
                &choices.mnt.join(dir).to_string_lossy(),
            ]));
        }
        return Ok(());
    }
    say(
        runner,
        "Unmounting",
        if choices.encrypt {
            "Unmounts the new system and locks the encrypted disk again."
        } else {
            "Unmounts the new system."
        },
    );
    let _ = runner.run(&s(&["umount", "--recursive", "--lazy", &mnt]));
    if choices.encrypt {
        if choices.home_disk.is_some() {
            let _ = runner.run(&s(&["cryptsetup", "close", super::layout::HOME_MAPPER]));
        }
        let _ = runner.run(&s(&["cryptsetup", "close", super::layout::ROOT_MAPPER]));
    }
    Ok(())
}

/// Where the /home disk's key is made, outside the new system until it's
/// copied in (only readable by root).
pub fn home_key_path() -> PathBuf {
    PathBuf::from("/run/bunny-install-home.key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_networks_go_into_the_new_system_root_only() {
        use std::os::unix::fs::PermissionsExt;
        struct Real;
        impl Runner for Real {
            fn run(&mut self, _: &[String]) -> Result<bool> {
                Ok(true)
            }
        }
        let live = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        std::fs::write(live.path().join("Home.psk"), "[Security]\nPassphrase=x\n").unwrap();
        std::fs::write(live.path().join("Cafe.open"), "").unwrap();
        std::fs::write(live.path().join("notes.txt"), "not a network").unwrap();
        let to = new.path().join("var/lib/iwd");
        assert_eq!(
            copy_wifi_networks(live.path(), &to, &Real).unwrap(),
            Some(2)
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&to), 0o700);
        assert_eq!(mode(&to.join("Home.psk")), 0o600);
        assert!(to.join("Cafe.open").exists());
        assert!(!to.join("notes.txt").exists());
        // No networks (or no iwd folder): nothing to say.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            copy_wifi_networks(&empty.path().join("none"), &to, &Real).unwrap(),
            None
        );
    }

    #[test]
    fn finds_the_newest_stage3() {
        let listing = "# Latest as of Sun, 15 Nov 2026 16:40:03 +0000\n\
                       # ts=1795279203\n\
                       -----BEGIN PGP SIGNED MESSAGE-----\n\
                       20261115T164003Z/stage3-amd64-openrc-20261115T164003Z.tar.xz 285913212\n";
        assert_eq!(
            latest_stage3(listing).as_deref(),
            Some("20261115T164003Z/stage3-amd64-openrc-20261115T164003Z.tar.xz")
        );
        assert_eq!(latest_stage3("# nothing\n"), None);
    }

    #[test]
    fn reads_the_sha512_line() {
        let hash = "a".repeat(128);
        let digests = format!(
            "# BLAKE2B HASH\n{b} stage3.tar.xz\n# SHA512 HASH\n{hash}  stage3.tar.xz\n# SHA512 HASH\n{hash}  stage3.tar.xz.CONTENTS.gz\n",
            b = "b".repeat(128)
        );
        assert_eq!(sha512_for(&digests, "stage3.tar.xz"), Some(hash.clone()));
        assert_eq!(
            sha512_for(&digests, "stage3.tar.xz.CONTENTS.gz"),
            Some(hash)
        );
        assert_eq!(sha512_for(&digests, "other.tar.xz"), None);
    }

    #[test]
    fn writes_the_settings_into_the_new_system() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("etc/portage")).unwrap();
        std::fs::write(
            dir.path().join("etc/portage/make.conf"),
            "COMMON_FLAGS=\"-O2\"\n",
        )
        .unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: dir.path().to_path_buf(),
            swap_mib: 2048,
            ..Default::default()
        };
        let account = Account {
            username: "you".into(),
            hostname: "laptop".into(),
            ssh: true,
            timezone: "America/Toronto".into(),
            keymap: "us".into(),
        };
        let uuids = Uuids {
            efi: "ABCD-1234".into(),
            root_fs: "fs".into(),
            root_luks: Some("luks".into()),
            ..Default::default()
        };
        struct Quiet;
        impl Runner for Quiet {
            fn run(&mut self, _: &[String]) -> Result<bool> {
                Ok(true)
            }
        }
        configure(
            &choices,
            &account,
            &uuids,
            "rd.luks.uuid=luks",
            Path::new("/nope"),
            &mut Quiet,
        )
        .unwrap();
        let read = |p: &str| std::fs::read_to_string(dir.path().join(p)).unwrap();
        let make = read("etc/portage/make.conf");
        assert!(make.starts_with("COMMON_FLAGS"), "{make}");
        assert!(make.contains("FEATURES=\"getbinpkg binpkg-request-signature\""));
        assert!(read("etc/dracut.conf.d/bunny.conf").contains("btrfs crypt"));
        assert!(read("etc/fstab").contains("UUID=ABCD-1234  /boot  vfat"));
        assert!(read("etc/sudoers.d/bunny").contains("%wheel ALL=(ALL:ALL) ALL"));
        let env = read("root/bunny-install.env");
        assert!(env.contains("HOST=laptop\nUSERNAME=you\n"), "{env}");
        assert!(
            env.contains("TIMEZONE=America/Toronto\nKEYMAP=us\n"),
            "{env}"
        );
        assert!(env.contains("CMDLINE=\"rd.luks.uuid=luks\""), "{env}");
        assert!(env.contains("sys-fs/cryptsetup"), "{env}");
        // Bunny Linux's own packages come with the first switch, once its
        // repository is set up: the install script's emerge can't see them.
        assert!(!env.contains("bunny-release"), "{env}");
        assert!(env.contains("SERVICES=\"dhcpcd sysklogd sshd\""), "{env}");
        assert!(env.contains("BOOT_SERVICES=\"zram swapfiles\""), "{env}");
        assert!(read("etc/init.d/zram").contains("swapon --priority 100"));
        assert!(read("etc/init.d/swapfiles").contains("need localmount"));
        let inside = read("root/bunny-inside.sh");
        assert!(inside.contains("grub-install"));
        // The time zone is set with the link current Gentoo reads.
        assert!(inside.contains("ln -sf \"../usr/share/zoneinfo/$TIMEZONE\" /etc/localtime"));
        assert!(inside.contains("eselect news read --quiet all"));
        assert!(dir.path().join("srv").is_dir());
        assert!(dir.path().join("usr/local/bin/bunny").is_file());
        // No passwords are ever written to the new system's files.
        assert!(!env.to_lowercase().contains("password"));
    }

    /// The console font that can draw ✓ ✗ ⚠ is set up and started at boot.
    #[test]
    fn inside_script_sets_a_console_font_with_bunnys_marks() {
        assert!(INSIDE.contains("/usr/share/consolefonts/eurlatgr.psfu.gz"));
        // ⚠ is added to the "!" glyph (position 0x21).
        assert!(INSIDE.contains("printf '0x021\\tU+26A0\\n'"));
        assert!(INSIDE.contains("/etc/conf.d/consolefont"));
        assert!(INSIDE.contains("rc-update add consolefont boot"));
        // It only needs kbd, which the stage3 already has.
        let font = INSIDE.find("consolefont boot").unwrap();
        let packages = INSIDE.find("emerge -q --noreplace $PACKAGES").unwrap();
        assert!(font < packages);
    }

    /// The stage3's own packages are brought in line with the profile
    /// during the install, so the first `bunny switch` rebuilds nothing.
    #[test]
    fn inside_script_rebuilds_the_stage3_packages_whose_flags_differ() {
        let packages = INSIDE.find("emerge -q --noreplace $PACKAGES").unwrap();
        let world = INSIDE
            .find("emerge -q --update --deep --newuse @world")
            .expect("a world update with bunny switch's options");
        let kernel = INSIDE
            .find("--config sys-kernel/gentoo-kernel-bin")
            .unwrap();
        assert!(packages < world && world < kernel);
    }

    #[test]
    fn the_boot_menu_is_tidied_and_rebuilt() {
        let mnt = crate::boot::tidy::tests::gentoo_root();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        #[derive(Default)]
        struct Recorded(Vec<String>);
        impl Runner for Recorded {
            fn run(&mut self, cmd: &[String]) -> Result<bool> {
                self.0.push(cmd.join(" "));
                Ok(true)
            }
        }
        let mut runner = Recorded::default();
        tidy_boot_menu(&choices, &mut runner).unwrap();
        assert!(crate::boot::tidy::needed(mnt.path()).is_empty());
        assert_eq!(
            runner.0,
            vec![format!(
                "chroot {} grub-mkconfig -o /boot/grub/grub.cfg",
                mnt.path().display()
            )]
        );
        assert!(mnt
            .path()
            .join("var/lib/bunny/backups/as-installed/etc/default/grub")
            .is_file());
        // Already tidy: nothing runs.
        let mut again = Recorded::default();
        tidy_boot_menu(&choices, &mut again).unwrap();
        assert!(again.0.is_empty());
    }
}
