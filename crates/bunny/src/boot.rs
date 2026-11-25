//! `[boot] undo_entry`: an "Undo" entry in the GRUB menu, for when a change
//! leaves the system unable to start.
//!
//! Two parts:
//! - A small dracut module (`boot/bunny-rollback.sh`) in the startup image.
//!   It does nothing unless the kernel command line has
//!   `bunny.rollback=<generation>`; then, before the system is mounted, it
//!   does the same swap as `bunny rollback`.
//! - After each switch, GRUB entries in `/boot/grub/bunny.cfg` (read when
//!   the menu is shown, so no `grub-mkconfig` is needed), placed right after
//!   the system's own entries by `/etc/grub.d/11_bunny`:
//!   - "Undo the last change", which boots the kernel and startup image
//!     saved *in that generation* with `bunny.rollback=<generation>` added
//!     (its own kernel, so its modules match after the swap);
//!   - "Earlier versions", a submenu of the newest generations, each with
//!     what came after it and "Go back to this version".
//!
//! Older versions of bunny wrote the undo entry to `custom.cfg` (at the
//! bottom of the menu); it's moved on the next switch.

use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub mod tidy;

/// The `[boot]` section.
#[derive(Debug, Default, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Boot {
    /// Add an "Undo" entry to the boot menu after each change.
    #[serde(default)]
    pub undo_entry: bool,
}

/// Where the dracut module lives, relative to the root.
pub const MODULE_DIR: &str = "usr/lib/dracut/modules.d/90bunny-rollback";
pub const MODULE_SETUP: &str = include_str!("boot/module-setup.sh");
pub const HOOK: &str = include_str!("boot/bunny-rollback.sh");

/// Where older versions of bunny put the undo entry (read through
/// `/etc/grub.d/41_custom`, at the bottom of the menu).
pub const CUSTOM_CFG: &str = "boot/grub/custom.cfg";
/// bunny's own entries, read through `/etc/grub.d/11_bunny`.
pub const BUNNY_CFG: &str = "boot/grub/bunny.cfg";
/// The grub-mkconfig script that reads `bunny.cfg` right after the
/// system's own entries.
pub const GRUBD_SCRIPT: &str = "etc/grub.d/11_bunny";
pub const GRUBD_TEXT: &str = include_str!("boot/11_bunny");
const BEGIN: &str = "### BEGIN bunny: undo entry (written by bunny; edit bunny.toml instead)";
const END: &str = "### END bunny";
/// The Earlier versions submenu has its own markers, so the startup hook
/// (which removes the undo entry after an undo) leaves it alone.
const EARLIER_BEGIN: &str = "### BEGIN bunny: earlier versions (written by bunny)";
const EARLIER_END: &str = "### END bunny: earlier versions";
/// How many generations the Earlier versions submenu lists.
pub const EARLIER_SHOWN: usize = 10;

/// What a switch would change for `[boot]`.
#[derive(Debug, Default, PartialEq)]
pub struct Changes {
    /// The module isn't installed (or is out of date): install it and
    /// rebuild the startup images for these kernel versions.
    pub install_module: Option<Vec<String>>,
    /// Turned off: remove the entry from custom.cfg.
    pub remove_entry: bool,
    /// Why it can't be set up on this system (shown, but doesn't stop the
    /// rest of the switch).
    pub problems: Vec<String>,
    /// Gentoo's leftovers in the menu to tidy (theme, "GNU/Linux",
    /// "(EFI BootNext)").
    pub tidy: Vec<tidy::Tidy>,
    /// `/boot` can only be read as root (an EFI partition usually is), and
    /// this isn't root: nothing about the boot menu could be checked.
    pub needs_root: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.install_module.is_none() && !self.remove_entry && self.tidy.is_empty()
    }
}

fn module_installed(root: &Path) -> bool {
    let init = crate::init::for_config(crate::config::Init::default(), root);
    if !init.has_boot_service(
        crate::finish_undo::SERVICE,
        crate::finish_undo::SERVICE_SCRIPT,
    ) {
        return false;
    }
    let dir = root.join(MODULE_DIR);
    std::fs::read_to_string(dir.join("module-setup.sh"))
        .ok()
        .as_deref()
        == Some(MODULE_SETUP)
        && std::fs::read_to_string(dir.join("bunny-rollback.sh"))
            .ok()
            .as_deref()
            == Some(HOOK)
}

/// Kernel versions that have a startup image bunny can rebuild:
/// `/boot/initramfs-<ver>.img` with `/lib/modules/<ver>`.
pub fn kernels(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root.join("boot"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let ver = name
                .strip_prefix("initramfs-")?
                .strip_suffix(".img")?
                .to_string();
            root.join("lib/modules").join(&ver).is_dir().then_some(ver)
        })
        .collect();
    found.sort();
    found
}

/// Whether `/boot` can't be read because of its permissions (only root can
/// read an EFI partition mounted with `fmask=0077`). Root is never refused.
fn boot_needs_root(root: &Path) -> bool {
    let denied = |r: std::io::Result<std::fs::Metadata>| {
        r.is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    };
    std::fs::read_dir(root.join("boot"))
        .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
        || denied(std::fs::metadata(root.join("boot/grub/grub.cfg")))
}

pub fn changes(boot: &Boot, root: &Path) -> Changes {
    changes_reading(boot, root, boot_needs_root(root))
}

/// [`changes`], told whether `/boot` can be read (for tests, which run as
/// root and so can't be refused).
fn changes_reading(boot: &Boot, root: &Path, needs_root: bool) -> Changes {
    let mut out = Changes::default();
    if boot.undo_entry && needs_root {
        // Every check below reads /boot, and would wrongly say there's no
        // GRUB or startup image.
        out.needs_root = true;
        return out;
    }
    if !boot.undo_entry {
        out.remove_entry =
            std::fs::read_to_string(root.join(CUSTOM_CFG)).is_ok_and(|text| text.contains(BEGIN));
        return out;
    }
    if !root.join("boot/grub/grub.cfg").exists() {
        out.problems
            .push("the undo entry needs GRUB, and there's no /boot/grub/grub.cfg".into());
    } else {
        out.tidy = tidy::needed(root);
    }
    if !root.join("usr/bin/dracut").exists() {
        out.problems
            .push("the undo entry needs dracut to build the startup image".into());
    }
    if !root.join(".snapshots").is_dir() {
        out.problems
            .push("the undo entry needs generations (/.snapshots on btrfs)".into());
    }
    let kernels = kernels(root);
    if kernels.is_empty() {
        out.problems.push(
            "couldn't find a startup image to add the undo step to (/boot/initramfs-<version>.img)"
                .into(),
        );
    }
    if out.problems.is_empty() && !module_installed(root) {
        out.install_module = Some(kernels);
    }
    out
}

/// The kernel command line for the undo entry: the one this system booted
/// with, minus what GRUB adds itself and any earlier `bunny.rollback=`.
pub fn undo_cmdline(proc_cmdline: &str, generation: &str) -> String {
    let mut words: Vec<&str> = proc_cmdline
        .split_whitespace()
        .filter(|w| {
            !w.starts_with("BOOT_IMAGE=")
                && !w.starts_with("initrd=")
                && !w.starts_with("bunny.rollback=")
        })
        .collect();
    // The same setting twice (GRUB's settings and the install's both add
    // rootflags=subvol=@) is said once.
    let mut seen = std::collections::HashSet::new();
    words.retain(|w| seen.insert(*w));
    let arg = format!("bunny.rollback={generation}");
    words.push(&arg);
    words.join(" ")
}

/// The filesystem UUID GRUB should search for, from `root=UUID=…`.
pub fn root_uuid(proc_cmdline: &str) -> Option<&str> {
    proc_cmdline
        .split_whitespace()
        .find_map(|w| w.strip_prefix("root=UUID="))
}

/// The kernel and startup image saved in a generation: the one matching the
/// running kernel if it's there, otherwise the newest.
pub fn generation_kernel(generation_dir: &Path, running: &str) -> Option<String> {
    let has = |ver: &str| {
        generation_dir.join(format!("boot/vmlinuz-{ver}")).is_file()
            && generation_dir
                .join(format!("boot/initramfs-{ver}.img"))
                .is_file()
    };
    if has(running) {
        return Some(running.to_string());
    }
    let mut versions: Vec<String> = std::fs::read_dir(generation_dir.join("boot"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("vmlinuz-").map(String::from)
        })
        .filter(|v| has(v))
        .collect();
    versions.sort();
    versions.pop()
}

/// Whether the generation's startup image has the undo step: the module was
/// in that system, and the image was built after it.
pub fn generation_has_module(generation_dir: &Path, version: &str) -> bool {
    image_has_module(
        generation_dir,
        &generation_dir.join(format!("boot/initramfs-{version}.img")),
    )
}

/// Whether the startup image `image` has the undo step: it was built after
/// the generation's copy of the module.
pub fn image_has_module(generation_dir: &Path, image: &Path) -> bool {
    let module = generation_dir.join(MODULE_DIR).join("bunny-rollback.sh");
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (modified(&module), modified(image)) {
        (Some(m), Some(i)) => i >= m,
        _ => false,
    }
}

/// How to start one generation from the boot menu: its kernel, startup
/// images and command line (with `bunny.rollback=<generation>`).
#[derive(Debug, Clone, PartialEq)]
pub struct Bootable {
    /// GRUB's module for the filesystem the kernel is on ("btrfs" or "fat").
    pub fs_module: &'static str,
    pub uuid: String,
    pub kernel: String,
    /// Microcode images first, then the startup image, as GRUB expects.
    pub initrds: Vec<String>,
    pub cmdline: String,
}

impl Bootable {
    /// The generation's own kernel inside its snapshot (when `/boot` is
    /// part of the system).
    pub fn in_snapshot(generation: &str, version: &str, uuid: &str, cmdline: &str) -> Bootable {
        let dir = format!("/@snapshots/{generation}/boot");
        Bootable {
            fs_module: "btrfs",
            uuid: uuid.to_string(),
            kernel: format!("{dir}/vmlinuz-{version}"),
            initrds: vec![format!("{dir}/initramfs-{version}.img")],
            cmdline: cmdline.to_string(),
        }
    }

    /// The copies bunny keeps on a separate `/boot` (usually the EFI
    /// partition, with the system encrypted).
    pub fn on_boot(boot_uuid: &str, kernel: &str, initrds: &[String], cmdline: &str) -> Bootable {
        Bootable {
            fs_module: "fat",
            uuid: boot_uuid.to_string(),
            kernel: kernel.to_string(),
            initrds: initrds.to_vec(),
            cmdline: cmdline.to_string(),
        }
    }
}

/// One generation in the boot menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuVersion {
    /// Local time, like `Nov 18, 7:03 PM`.
    pub when: String,
    /// What came right after it, like `+ mangowm, + dejavu` (empty if not
    /// known, or for a rollback).
    pub summary: String,
    pub bootable: Bootable,
}

/// GRUB titles are in single quotes; a quote in one would end it.
fn title(text: &str) -> String {
    text.replace('\'', "’")
}

/// `text`, cut to `max` characters with "…".
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end().trim_end_matches(','))
    }
}

fn menuentry(out: &mut String, indent: &str, label: &str, message: &str, b: &Bootable) {
    let _ = writeln!(out, "{indent}menuentry '{}' --class bunny {{", title(label));
    let _ = writeln!(out, "{indent}    insmod {}", b.fs_module);
    let _ = writeln!(
        out,
        "{indent}    search --no-floppy --fs-uuid --set=root {}",
        b.uuid
    );
    let _ = writeln!(out, "{indent}    echo '{}'", title(message));
    let _ = writeln!(out, "{indent}    linux {} {}", b.kernel, b.cmdline);
    let _ = writeln!(out, "{indent}    initrd {}", b.initrds.join(" "));
    let _ = writeln!(out, "{indent}}}");
}

/// The "Undo the last change" entry, for the newest generation.
pub fn render_undo(newest: &MenuVersion) -> String {
    let what = if newest.summary.is_empty() {
        newest.when.clone()
    } else {
        format!("{}, {}", shorten(&newest.summary, 40), newest.when)
    };
    let mut out = String::new();
    let _ = writeln!(out, "{BEGIN}");
    menuentry(
        &mut out,
        "",
        &format!("Undo the last change ({what})"),
        "Undoing the last change…",
        &newest.bootable,
    );
    let _ = writeln!(out, "{END}");
    out
}

/// The "Earlier versions" submenu, newest first. `saved` is how many
/// generations there are in all (only the newest few are listed).
pub fn render_earlier(versions: &[MenuVersion], saved: usize) -> String {
    let mut out = String::new();
    if versions.is_empty() {
        return out;
    }
    let _ = writeln!(out, "{EARLIER_BEGIN}");
    let _ = writeln!(
        out,
        "submenu 'Earlier versions ({saved} saved)' --class bunny {{"
    );
    for v in versions {
        let label = if v.summary.is_empty() {
            v.when.clone()
        } else {
            format!(
                "{} · before {}",
                v.when,
                shorten(&crate::notes::before_words(&v.summary), 56)
            )
        };
        let _ = writeln!(out, "    submenu '{}' {{", title(&label));
        menuentry(
            &mut out,
            "        ",
            &format!("Go back to this version ({})", v.when),
            &format!("Going back to {}…", v.when),
            &v.bootable,
        );
        if !v.summary.is_empty() {
            // GRUB can't show plain text: each line of what came after this
            // version is an empty submenu, which does nothing when picked.
            let mut line = |label: &str| {
                let _ = writeln!(out, "        submenu '{}' {{", title(label));
                let _ = writeln!(out, "            true");
                let _ = writeln!(out, "        }}");
            };
            line("What came after it:");
            for change in v.summary.split(", ") {
                line(&format!("    {change}"));
            }
        }
        let _ = writeln!(out, "    }}");
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "{EARLIER_END}");
    out
}

/// The undo entry for `generation`, with no summary (for tests).
#[cfg(test)]
pub fn render_entry(
    generation: &str,
    when: &str,
    version: &str,
    uuid: &str,
    cmdline: &str,
) -> String {
    render_undo(&MenuVersion {
        when: when.to_string(),
        summary: String::new(),
        bootable: Bootable::in_snapshot(generation, version, uuid, cmdline),
    })
}

/// Text with the block between `begin` and `end` replaced by `block` (or
/// removed if it's empty), keeping everything else.
fn merge_block(existing: &str, begin: &str, end: &str, block: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in existing.lines() {
        if line == begin {
            inside = true;
            continue;
        }
        if inside {
            if line == end {
                inside = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !block.is_empty() {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(block);
    }
    out
}

/// bunny.cfg with its Earlier versions submenu replaced by `block`.
pub fn merge_earlier(existing: &str, block: &str) -> String {
    merge_block(existing, EARLIER_BEGIN, EARLIER_END, block)
}

/// custom.cfg with bunny's block replaced by `block` (or removed if empty),
/// keeping everything else in the file.
pub fn merge_custom_cfg(existing: &str, block: &str) -> String {
    merge_block(existing, BEGIN, END, block)
}

/// Removes bunny's undo entry from a restored system (mounted at `system`).
/// The restored system brings back its own, older custom.cfg, whose entry
/// would undo one change further back; picking it again and again would keep
/// going back. After a rollback there's no "last change" to undo until the
/// next switch, so the entry goes. Returns true if one was removed.
pub fn drop_entry_in(system: &Path) -> Result<bool> {
    let mut dropped = false;
    for file in [BUNNY_CFG, CUSTOM_CFG] {
        let path = system.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let without = merge_custom_cfg(&text, "");
        if without != text {
            std::fs::write(&path, without)
                .with_context(|| format!("Couldn't write {}", path.display()))?;
            dropped = true;
        }
    }
    Ok(dropped)
}

/// The path of a kernel's startup image.
pub fn initramfs(root: &Path, version: &str) -> PathBuf {
    root.join(format!("boot/initramfs-{version}.img"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMDLINE: &str =
        "BOOT_IMAGE=/@/boot/vmlinuz-6.12.58-gentoo-dist root=UUID=1234-abcd ro rootflags=subvol=@ quiet";

    #[test]
    fn command_line_keeps_the_system_settings() {
        assert_eq!(
            undo_cmdline(CMDLINE, "bunny-20261117-091500"),
            "root=UUID=1234-abcd ro rootflags=subvol=@ quiet bunny.rollback=bunny-20261117-091500"
        );
        assert_eq!(
            undo_cmdline("root=/dev/vda2 bunny.rollback=bunny-old", "bunny-new"),
            "root=/dev/vda2 bunny.rollback=bunny-new"
        );
        assert_eq!(root_uuid(CMDLINE), Some("1234-abcd"));
        assert_eq!(root_uuid("root=/dev/vda2"), None);
    }

    #[test]
    fn says_a_setting_once() {
        assert_eq!(
            undo_cmdline(
                "BOOT_IMAGE=/vmlinuz root=UUID=1 ro rootflags=subvol=@ rd.luks.uuid=2 rootflags=subvol=@",
                "bunny-1"
            ),
            "root=UUID=1 ro rootflags=subvol=@ rd.luks.uuid=2 bunny.rollback=bunny-1"
        );
    }

    #[test]
    fn earlier_versions_lists_each_with_what_came_after_it() {
        let b = Bootable::in_snapshot("bunny-7", "6.18", "u", "root=UUID=u bunny.rollback=bunny-7");
        let versions = [
            MenuVersion {
                when: "Nov 18, 7:03 PM".into(),
                summary: "+ repository guru, + mangowm, + dejavu".into(),
                bootable: b.clone(),
            },
            MenuVersion {
                when: "Nov 18, 4:46 PM".into(),
                summary: String::new(),
                bootable: b,
            },
        ];
        let menu = render_earlier(&versions, 11);
        assert!(menu.starts_with(EARLIER_BEGIN), "{menu}");
        assert!(menu.contains("submenu 'Earlier versions (11 saved)'"));
        assert!(
            menu.contains(
                "submenu 'Nov 18, 7:03 PM · before adding the repository guru, adding mangowm and dejavu'"
            ),
            "{menu}"
        );
        assert!(menu.contains("menuentry 'Go back to this version (Nov 18, 7:03 PM)'"));
        assert!(menu.contains("        submenu '    + mangowm' {\n            true\n        }"));
        assert!(menu.contains("submenu 'Nov 18, 4:46 PM' {"));
        assert!(menu.trim_end().ends_with(EARLIER_END));
        assert_eq!(render_earlier(&[], 0), "");
        // The hook's marker for the undo entry doesn't match these.
        assert!(!menu.contains(BEGIN) && !menu.lines().any(|l| l == END));

        // Replacing it keeps the undo entry, and the other way round.
        let undo = render_entry("bunny-9", "now", "6.18", "u", "x");
        let both = merge_earlier(&merge_custom_cfg("", &undo), &menu);
        let fewer = merge_earlier(&both, &render_earlier(&versions[..1], 1));
        assert!(fewer.contains("Undo the last change") && fewer.contains("(1 saved)"));
        assert!(!fewer.contains("4:46 PM"));
        let no_undo = merge_custom_cfg(&both, "");
        assert!(!no_undo.contains("Undo the last change") && no_undo.contains("(11 saved)"));
    }

    #[test]
    fn titles_are_safe_in_quotes_and_short() {
        assert_eq!(title("it's"), "it’s");
        assert_eq!(shorten("abcdef", 10), "abcdef");
        assert_eq!(shorten("+ a, + b, + c", 9), "+ a, + b…");
    }

    #[test]
    fn the_entry_boots_the_generations_own_kernel() {
        let entry = render_entry(
            "bunny-1",
            "2026-11-17 09:15 UTC",
            "6.12.58",
            "1234",
            "root=UUID=1234 bunny.rollback=bunny-1",
        );
        assert!(
            entry.contains("menuentry 'Undo the last change (2026-11-17 09:15 UTC)'"),
            "{entry}"
        );
        assert!(entry.contains("search --no-floppy --fs-uuid --set=root 1234"));
        assert!(entry.contains(
            "linux /@snapshots/bunny-1/boot/vmlinuz-6.12.58 root=UUID=1234 bunny.rollback=bunny-1"
        ));
        assert!(entry.contains("initrd /@snapshots/bunny-1/boot/initramfs-6.12.58.img"));
        assert!(entry.starts_with(BEGIN) && entry.trim_end().ends_with(END));
    }

    #[test]
    fn custom_cfg_keeps_the_users_own_entries() {
        let mine = "menuentry 'Windows' {\n    chainloader /x\n}\n";
        let block = render_entry("bunny-1", "now", "6", "u", "c");
        let once = merge_custom_cfg(mine, &block);
        assert!(once.starts_with(mine));
        assert!(once.contains("bunny-1"));
        let block2 = render_entry("bunny-2", "now", "6", "u", "c");
        let twice = merge_custom_cfg(&once, &block2);
        assert!(!twice.contains("bunny-1"), "{twice}");
        assert!(twice.contains("bunny-2"));
        assert!(twice.starts_with(mine));
        assert_eq!(merge_custom_cfg(&twice, "").trim_end(), mine.trim_end());
    }

    fn fake_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        for d in ["boot/grub", "usr/bin", ".snapshots", "lib/modules/6.12.58"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        std::fs::write(r.join("boot/grub/grub.cfg"), "").unwrap();
        std::fs::write(r.join("usr/bin/dracut"), "").unwrap();
        std::fs::write(r.join("boot/initramfs-6.12.58.img"), "").unwrap();
        std::fs::write(r.join("boot/vmlinuz-6.12.58"), "").unwrap();
        dir
    }

    #[test]
    fn plans_the_module_once() {
        let root = fake_root();
        let boot = Boot { undo_entry: true };
        let c = changes(&boot, root.path());
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(c.install_module, Some(vec!["6.12.58".to_string()]));

        let dir = root.path().join(MODULE_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module-setup.sh"), MODULE_SETUP).unwrap();
        std::fs::write(dir.join("bunny-rollback.sh"), HOOK).unwrap();
        // The start-up service that finishes an undo is part of it too.
        assert!(!changes(&boot, root.path()).is_empty());
        crate::init::for_config(crate::config::Init::default(), root.path())
            .add_boot_service(
                crate::finish_undo::SERVICE,
                crate::finish_undo::SERVICE_SCRIPT,
            )
            .unwrap();
        assert!(changes(&boot, root.path()).is_empty());
    }

    #[test]
    fn doesnt_guess_when_boot_can_only_be_read_as_root() {
        // Run as yourself on an encrypted install: /boot is the EFI
        // partition, readable only by root, so nothing there can be seen.
        let root = tempfile::tempdir().unwrap();
        let c = changes_reading(&Boot { undo_entry: true }, root.path(), true);
        assert!(c.needs_root);
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert!(c.install_module.is_none());
        assert!(c.is_empty());
        // Readable: the usual checks.
        let c = changes_reading(&Boot { undo_entry: true }, root.path(), false);
        assert!(!c.needs_root);
        assert!(!c.problems.is_empty());
        // Turned off: there's nothing to check.
        assert!(!changes_reading(&Boot { undo_entry: false }, root.path(), true).needs_root);
        // A /boot that's simply readable isn't mistaken for one that isn't.
        assert!(!boot_needs_root(fake_root().path()));
    }

    #[test]
    fn says_why_it_cant_be_set_up() {
        let dir = tempfile::tempdir().unwrap();
        let c = changes(&Boot { undo_entry: true }, dir.path());
        assert!(c.install_module.is_none());
        assert!(c.problems.iter().any(|p| p.contains("GRUB")));
        assert!(c.problems.iter().any(|p| p.contains("dracut")));
    }

    #[test]
    fn turning_it_off_removes_the_entry() {
        let root = fake_root();
        let off = Boot { undo_entry: false };
        assert!(changes(&off, root.path()).is_empty());
        std::fs::write(
            root.path().join(CUSTOM_CFG),
            render_entry("bunny-1", "now", "6", "u", "c"),
        )
        .unwrap();
        assert!(changes(&off, root.path()).remove_entry);
    }

    #[test]
    fn a_restored_system_loses_its_old_undo_entry() {
        let system = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(system.path().join("boot/grub")).unwrap();
        let mine = "menuentry 'Windows' {\n    chainloader /x\n}\n";
        let block = render_entry("bunny-old", "earlier", "6", "u", "c");
        std::fs::write(
            system.path().join(CUSTOM_CFG),
            merge_custom_cfg(mine, &block),
        )
        .unwrap();
        assert!(drop_entry_in(system.path()).unwrap());
        let left = std::fs::read_to_string(system.path().join(CUSTOM_CFG)).unwrap();
        assert!(!left.contains("undo the last change"));
        assert!(left.contains("Windows"));
        assert!(!drop_entry_in(system.path()).unwrap());
        let empty = tempfile::tempdir().unwrap();
        assert!(!drop_entry_in(empty.path()).unwrap());
    }

    #[test]
    fn finds_the_generations_kernel() {
        let root = fake_root();
        assert_eq!(
            generation_kernel(root.path(), "6.12.58").as_deref(),
            Some("6.12.58")
        );
        assert_eq!(
            generation_kernel(root.path(), "7.0.0").as_deref(),
            Some("6.12.58")
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(generation_kernel(empty.path(), "6.12.58"), None);
        assert!(!generation_has_module(root.path(), "6.12.58"));
    }

    /// Runs the real boot-time script with the system commands replaced by
    /// stand-ins working on a folder, and checks it does the swap.
    fn run_hook(target: &str, top: &Path) -> String {
        let script = format!(
            r#"
            getarg() {{ echo "{target}"; }}
            warn() {{ echo "WARN: $*"; }}
            info() {{ echo "INFO: $*"; }}
            mount() {{ return 0; }}
            umount() {{ return 0; }}
            date() {{ echo 20261117-120000; }}
            btrfs() {{ shift 2; [ "$1" = -r ] && shift; cp -a "$1" "$2"; }}
            mkdir() {{ return 0; }}
            root=block:/dev/fake
            {hook}
            "#,
            hook = HOOK.replace("/run/bunny-top", &top.to_string_lossy())
        );
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }

    fn fake_top() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path();
        std::fs::create_dir_all(t.join("@")).unwrap();
        std::fs::write(t.join("@/broken"), "").unwrap();
        std::fs::create_dir_all(t.join("@snapshots/bunny-20261117-091500")).unwrap();
        std::fs::write(t.join("@snapshots/bunny-20261117-091500/good"), "").unwrap();
        dir
    }

    #[test]
    fn boot_step_does_the_swap() {
        let top = fake_top();
        let t = top.path();
        let log = run_hook("bunny-20261117-091500", t);
        assert!(t.join("@/good").exists(), "{log}");
        assert!(t.join("@replaced-20261117-120000/broken").exists(), "{log}");
        assert!(t
            .join("@snapshots/bunny-20261117-120000-before-rollback/broken")
            .exists());
        let note: crate::notes::Note = toml::from_str(
            &std::fs::read_to_string(
                t.join("@snapshots/bunny-20261117-120000-before-rollback.toml"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(note.kind, crate::notes::Kind::Rollback);
        assert_eq!(note.to.as_deref(), Some("bunny-20261117-091500"));
        let state: crate::notes::State = toml::from_str(
            &std::fs::read_to_string(t.join("@snapshots/bunny-state.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(state.last, crate::notes::Kind::Rollback);
    }

    #[test]
    fn boot_step_removes_the_restored_systems_old_entry() {
        let top = fake_top();
        let t = top.path();
        let grub = t.join("@snapshots/bunny-20261117-091500/boot/grub");
        std::fs::create_dir_all(&grub).unwrap();
        let mine = "menuentry 'Windows' {\n    chainloader /x\n}\n";
        let older = render_entry("bunny-20261116-080000", "earlier", "6", "u", "c");
        std::fs::write(grub.join("custom.cfg"), merge_custom_cfg(mine, &older)).unwrap();
        let log = run_hook("bunny-20261117-091500", t);
        let left = std::fs::read_to_string(t.join("@/boot/grub/custom.cfg")).unwrap();
        assert!(!left.contains("undo the last change"), "{log}\n{left}");
        assert!(left.contains("chainloader /x"), "{left}");
    }

    #[test]
    fn boot_step_does_nothing_when_already_undone() {
        let top = fake_top();
        let t = top.path();
        std::fs::write(
            t.join("@snapshots/bunny-state.toml"),
            "last = \"rollback\"\nat = \"20261117-100000\"\nto = \"bunny-20261117-091500\"\n",
        )
        .unwrap();
        let log = run_hook("bunny-20261117-091500", t);
        assert!(log.contains("already undone"), "{log}");
        assert!(
            t.join("@/broken").exists(),
            "the system was swapped anyway: {log}"
        );
        assert!(!t.join("@replaced-20261117-120000").exists());
    }

    #[test]
    fn boot_step_puts_the_matching_config_back() {
        let top = fake_top();
        let t = top.path();
        let generation = t.join("@snapshots/bunny-20261117-091500");
        let applied = generation.join(crate::applied::DIR);
        std::fs::create_dir_all(&applied).unwrap();
        std::fs::create_dir_all(generation.join("etc/bunny")).unwrap();
        std::fs::write(
            generation.join("etc/bunny/bunny.toml"),
            "edited after the snapshot",
        )
        .unwrap();
        std::fs::write(applied.join("bunny.toml"), "as applied").unwrap();
        std::fs::write(applied.join("bunny.lock"), "lock as applied").unwrap();
        std::fs::write(applied.join("config-path"), "/etc/bunny/bunny.toml\n").unwrap();
        let log = run_hook("bunny-20261117-091500", t);
        assert_eq!(
            std::fs::read_to_string(t.join("@/etc/bunny/bunny.toml")).unwrap(),
            "as applied",
            "{log}"
        );
        assert_eq!(
            std::fs::read_to_string(t.join("@/etc/bunny/bunny.lock")).unwrap(),
            "lock as applied"
        );
    }

    #[test]
    fn boot_step_leaves_a_note_to_finish_the_undo() {
        let top = fake_top();
        let t = top.path();
        std::fs::create_dir_all(t.join("@snapshots/bunny-20261117-091500/var/lib/bunny")).unwrap();
        let log = run_hook("bunny-20261117-091500", t);
        assert_eq!(
            std::fs::read_to_string(t.join("@").join(crate::finish_undo::MARKER)).unwrap(),
            "bunny-20261117-091500\n",
            "{log}"
        );
        assert_eq!(
            crate::finish_undo::waiting(&t.join("@")).as_deref(),
            Some("bunny-20261117-091500")
        );
    }

    #[test]
    fn boot_step_puts_the_host_file_back_too() {
        let top = fake_top();
        let t = top.path();
        let generation = t.join("@snapshots/bunny-20261117-091500");
        let applied = generation.join(crate::applied::DIR);
        std::fs::create_dir_all(&applied).unwrap();
        std::fs::create_dir_all(generation.join("etc/bunny/hosts")).unwrap();
        std::fs::write(generation.join("etc/bunny/bunny.toml"), "edited").unwrap();
        std::fs::write(
            generation.join("etc/bunny/hosts/laptop.toml"),
            "host edited after the snapshot",
        )
        .unwrap();
        std::fs::write(applied.join("bunny.toml"), "as applied").unwrap();
        std::fs::write(applied.join("config-path"), "/etc/bunny/bunny.toml\n").unwrap();
        std::fs::write(applied.join("host.toml"), "host as applied").unwrap();
        std::fs::write(applied.join("host-path"), "/etc/bunny/hosts/laptop.toml\n").unwrap();
        let log = run_hook("bunny-20261117-091500", t);
        assert_eq!(
            std::fs::read_to_string(t.join("@/etc/bunny/hosts/laptop.toml")).unwrap(),
            "host as applied",
            "{log}"
        );

        // A path that leaves /etc is ignored.
        let top = fake_top();
        let t = top.path();
        let generation = t.join("@snapshots/bunny-20261117-091500");
        let applied = generation.join(crate::applied::DIR);
        std::fs::create_dir_all(&applied).unwrap();
        std::fs::write(applied.join("host.toml"), "host as applied").unwrap();
        std::fs::write(applied.join("host-path"), "/etc/../evil.toml\n").unwrap();
        let log = run_hook("bunny-20261117-091500", t);
        assert!(!t.join("@/evil.toml").exists(), "{log}");
    }

    #[test]
    fn boot_step_does_nothing_on_a_normal_boot_or_a_bad_name() {
        for target in ["", "../etc", "bunny-missing", "@home"] {
            let top = fake_top();
            let log = run_hook(target, top.path());
            assert!(top.path().join("@/broken").exists(), "{target}: {log}");
            assert!(!top.path().join("@/good").exists(), "{target}: {log}");
        }
    }
}
