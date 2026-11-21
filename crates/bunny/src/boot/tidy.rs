//! The boot menu's leftover clutter from Gentoo's setup, tidied:
//!
//! - Gentoo's GRUB theme (`GRUB_THEME=…` in `/etc/default/grub`): the line
//!   is commented out, for a plain dark menu until Bunny Linux has its own.
//! - "Bunny Linux GNU/Linux": GRUB's `10_linux` always adds " GNU/Linux" to
//!   `GRUB_DISTRIBUTOR`. bunny's `/etc/grub.d/10_bunny` runs `10_linux` and
//!   changes only the titles ("Bunny Linux", "Advanced options"), and
//!   `10_linux` is made non-executable so grub-mkconfig skips it.
//! - The "(EFI BootNext)" entries from `/etc/grub.d/31_efi_bootnext`, made
//!   non-executable. "UEFI Firmware Settings" (`30_uefi-firmware`) and other
//!   systems (`30_os-prober`) stay.
//!
//! Every file is backed up before it's changed, and it all lives in `/etc`,
//! so undoing the change (a snapshot of `/`) puts it back.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

pub const DEFAULT_GRUB: &str = "etc/default/grub";
pub const OWN_SCRIPT: &str = "etc/grub.d/10_bunny";
pub const OWN_TEXT: &str = include_str!("10_bunny");
pub const LINUX_SCRIPT: &str = "etc/grub.d/10_linux";
pub const BOOTNEXT_SCRIPT: &str = "etc/grub.d/31_efi_bootnext";
/// Written above a theme line bunny turned off.
const THEME_NOTE: &str =
    "# Turned off by bunny: a plain menu until Bunny Linux has its own look. Remove the # to bring it back.";

/// One thing to tidy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tidy {
    /// Comment out `GRUB_THEME=` in `/etc/default/grub`.
    Theme,
    /// Install `10_bunny` and make `10_linux` non-executable.
    Titles,
    /// Make `31_efi_bootnext` non-executable.
    BootNext,
}

impl Tidy {
    /// One line for the plan.
    pub fn describe(&self) -> &'static str {
        match self {
            Tidy::Theme => "turn off Gentoo's menu theme (GRUB_THEME in /etc/default/grub)",
            Tidy::Titles => {
                "call the main entry \"Bunny Linux\", not \"Bunny Linux GNU/Linux\", and its \
                 submenu \"Advanced options\" (/etc/grub.d/10_bunny instead of 10_linux)"
            }
            Tidy::BootNext => {
                "remove the \"(EFI BootNext)\" entries (/etc/grub.d/31_efi_bootnext; \
                 \"UEFI Firmware Settings\" stays)"
            }
        }
    }
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether a line of `/etc/default/grub` sets a theme.
fn sets_theme(line: &str) -> bool {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    line.starts_with("GRUB_THEME=")
}

/// `/etc/default/grub` with every theme line commented out.
pub fn without_theme(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if sets_theme(line) {
            out.push_str(THEME_NOTE);
            out.push('\n');
            out.push('#');
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// What's left to tidy on the system at `root`.
pub fn needed(root: &Path) -> Vec<Tidy> {
    let mut out = Vec::new();
    if std::fs::read_to_string(root.join(DEFAULT_GRUB))
        .is_ok_and(|text| text.lines().any(sets_theme))
    {
        out.push(Tidy::Theme);
    }
    let linux = root.join(LINUX_SCRIPT);
    if linux.is_file()
        && (executable(&linux)
            || std::fs::read_to_string(root.join(OWN_SCRIPT))
                .ok()
                .as_deref()
                != Some(OWN_TEXT)
            || !executable(&root.join(OWN_SCRIPT)))
    {
        out.push(Tidy::Titles);
    }
    if executable(&root.join(BOOTNEXT_SCRIPT)) {
        out.push(Tidy::BootNext);
    }
    out
}

/// Copies `rel` (under `root`) into `backup`, at the same relative path,
/// keeping its permissions.
fn back_up(root: &Path, rel: &str, backup: &Path) -> Result<()> {
    let from = root.join(rel);
    if !from.exists() {
        return Ok(());
    }
    let to = backup.join(rel);
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Couldn't create {}", dir.display()))?;
    }
    std::fs::copy(&from, &to).with_context(|| format!("Couldn't back up {}", from.display()))?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("Couldn't change {}", path.display()))
}

/// Makes the changes in `todo`, backing up each file into `backup` first
/// (`<backup>/etc/default/grub`, `<backup>/etc/grub.d/…`). Doesn't run
/// grub-mkconfig: the caller does, once.
pub fn apply(root: &Path, todo: &[Tidy], backup: &Path) -> Result<()> {
    for item in todo {
        match item {
            Tidy::Theme => {
                back_up(root, DEFAULT_GRUB, backup)?;
                let path = root.join(DEFAULT_GRUB);
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("Couldn't read {}", path.display()))?;
                std::fs::write(&path, without_theme(&text))
                    .with_context(|| format!("Couldn't write {}", path.display()))?;
            }
            Tidy::Titles => {
                back_up(root, LINUX_SCRIPT, backup)?;
                back_up(root, OWN_SCRIPT, backup)?;
                // bunny's script first, so there's never a moment with
                // neither: it prints nothing while 10_linux still runs.
                let own = root.join(OWN_SCRIPT);
                std::fs::write(&own, OWN_TEXT)
                    .with_context(|| format!("Couldn't write {}", own.display()))?;
                set_mode(&own, 0o755)?;
                set_mode(&root.join(LINUX_SCRIPT), 0o644)?;
            }
            Tidy::BootNext => {
                back_up(root, BOOTNEXT_SCRIPT, backup)?;
                set_mode(&root.join(BOOTNEXT_SCRIPT), 0o644)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const GENTOO_DEFAULT: &str = "GRUB_DISTRIBUTOR=\"Gentoo\"\n\
        #GRUB_THEME=\"/boot/grub/themes/starfield/theme.txt\"\n\
        GRUB_THEME=\"/usr/share/grub/themes/gentoo_glass/theme.txt\"\n\
        \n# Added by bunny install\nGRUB_DISTRIBUTOR=\"Bunny Linux\"\n";

    /// A system as Gentoo's GRUB leaves it.
    pub(crate) fn gentoo_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("etc/grub.d")).unwrap();
        std::fs::create_dir_all(r.join("etc/default")).unwrap();
        std::fs::write(r.join(DEFAULT_GRUB), GENTOO_DEFAULT).unwrap();
        for (name, text) in [
            ("10_linux", "#! /bin/sh\necho linux\n"),
            ("30_os-prober", "#! /bin/sh\n"),
            ("30_uefi-firmware", "#! /bin/sh\n"),
            ("31_efi_bootnext", "#! /bin/sh\n"),
        ] {
            let p = r.join("etc/grub.d").join(name);
            std::fs::write(&p, text).unwrap();
            set_mode(&p, 0o755).unwrap();
        }
        dir
    }

    #[test]
    fn finds_all_three_on_a_gentoo_system() {
        let root = gentoo_root();
        assert_eq!(
            needed(root.path()),
            vec![Tidy::Theme, Tidy::Titles, Tidy::BootNext]
        );
    }

    #[test]
    fn tidies_backs_up_and_is_done_once() {
        let root = gentoo_root();
        let r = root.path();
        let backup = r.join("var/lib/bunny/backups/1");
        apply(r, &needed(r), &backup).unwrap();

        let grub = std::fs::read_to_string(r.join(DEFAULT_GRUB)).unwrap();
        assert!(grub.contains("#GRUB_THEME=\"/usr/share/grub/themes/gentoo_glass/theme.txt\""));
        assert!(!grub.lines().any(sets_theme), "{grub}");
        assert!(grub.contains("GRUB_DISTRIBUTOR=\"Bunny Linux\""));
        assert!(grub.contains("Turned off by bunny"));
        assert_eq!(
            std::fs::read_to_string(r.join(OWN_SCRIPT)).unwrap(),
            OWN_TEXT
        );
        assert!(executable(&r.join(OWN_SCRIPT)));
        assert!(!executable(&r.join(LINUX_SCRIPT)));
        assert!(!executable(&r.join(BOOTNEXT_SCRIPT)));
        // Firmware settings and other systems stay.
        assert!(executable(&r.join("etc/grub.d/30_uefi-firmware")));
        assert!(executable(&r.join("etc/grub.d/30_os-prober")));

        // The originals are in the backup, as they were.
        assert_eq!(
            std::fs::read_to_string(backup.join(DEFAULT_GRUB)).unwrap(),
            GENTOO_DEFAULT
        );
        assert!(executable(&backup.join(LINUX_SCRIPT)));
        assert!(executable(&backup.join(BOOTNEXT_SCRIPT)));

        assert!(needed(r).is_empty());
        // Commenting out again changes nothing.
        assert_eq!(without_theme(&grub), grub);
    }

    #[test]
    fn a_grub_update_that_turns_10_linux_back_on_is_noticed() {
        let root = gentoo_root();
        let r = root.path();
        apply(r, &needed(r), &r.join("b")).unwrap();
        set_mode(&r.join(LINUX_SCRIPT), 0o755).unwrap();
        assert_eq!(needed(r), vec![Tidy::Titles]);
    }

    #[test]
    fn nothing_to_do_without_gentoo_s_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(needed(dir.path()).is_empty());
    }

    /// Runs bunny's script next to a fake 10_linux, as grub-mkconfig would.
    fn run_own_script(linux_mode: u32) -> String {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let linux = d.join("10_linux");
        std::fs::write(
            &linux,
            "#! /bin/sh\ncat <<'X'\n\
             menuentry 'Bunny Linux GNU/Linux' --class bunny $menuentry_id_option 'gnulinux-simple-u' {\n\
             \tlinux /vmlinuz-6.12 root=UUID=u\n}\n\
             submenu 'Advanced options for Bunny Linux GNU/Linux' $menuentry_id_option 'gnulinux-advanced-u' {\n\
             \tmenuentry 'Bunny Linux GNU/Linux, with Linux 6.12' --class bunny {\n\
             \t\techo 'Loading Linux 6.12 ...'\n\t}\n}\nX\n",
        )
        .unwrap();
        set_mode(&linux, linux_mode).unwrap();
        let own = d.join("10_bunny");
        std::fs::write(&own, OWN_TEXT).unwrap();
        set_mode(&own, 0o755).unwrap();
        let out = std::process::Command::new(&own)
            .env("GRUB_DISTRIBUTOR", "Bunny Linux")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn the_script_tidies_only_the_titles() {
        let out = run_own_script(0o644);
        assert!(
            out.contains("menuentry 'Bunny Linux' --class bunny"),
            "{out}"
        );
        assert!(
            out.contains("submenu 'Advanced options' $menuentry_id_option"),
            "{out}"
        );
        assert!(
            out.contains("menuentry 'Bunny Linux, with Linux 6.12'"),
            "{out}"
        );
        assert!(!out.contains("GNU/Linux"), "{out}");
        assert!(out.contains("linux /vmlinuz-6.12 root=UUID=u"));
    }

    #[test]
    fn the_script_stays_quiet_while_10_linux_runs_itself() {
        assert_eq!(run_own_script(0o755).trim(), "");
    }
}
