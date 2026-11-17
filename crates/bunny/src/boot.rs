//! `[boot] undo_entry`: an "Undo" entry in the GRUB menu, for when a change
//! leaves the system unable to start.
//!
//! Two parts:
//! - A small dracut module (`boot/bunny-rollback.sh`) in the startup image.
//!   It does nothing unless the kernel command line has
//!   `bunny.rollback=<generation>`; then, before the system is mounted, it
//!   does the same swap as `bunny rollback`.
//! - After each switch, a GRUB entry (in `/boot/grub/custom.cfg`, which
//!   GRUB reads at boot, so no `grub-mkconfig` is needed) that boots the
//!   kernel and startup image saved *in that generation*, with
//!   `bunny.rollback=<generation>` added. Using the generation's own kernel
//!   means its modules match after the swap, even if the change updated the
//!   kernel.

use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

/// GRUB reads this at boot (through `/etc/grub.d/41_custom`).
pub const CUSTOM_CFG: &str = "boot/grub/custom.cfg";
const BEGIN: &str = "### BEGIN bunny: undo entry (written by bunny; edit bunny.toml instead)";
const END: &str = "### END bunny";

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
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.install_module.is_none() && !self.remove_entry
    }
}

fn module_installed(root: &Path) -> bool {
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

pub fn changes(boot: &Boot, root: &Path) -> Changes {
    let mut out = Changes::default();
    if !boot.undo_entry {
        out.remove_entry =
            std::fs::read_to_string(root.join(CUSTOM_CFG)).is_ok_and(|text| text.contains(BEGIN));
        return out;
    }
    if !root.join("boot/grub/grub.cfg").exists() {
        out.problems
            .push("the undo entry needs GRUB, and there's no /boot/grub/grub.cfg".into());
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
    let module = generation_dir.join(MODULE_DIR).join("bunny-rollback.sh");
    let image = generation_dir.join(format!("boot/initramfs-{version}.img"));
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (modified(&module), modified(&image)) {
        (Some(m), Some(i)) => i >= m,
        _ => false,
    }
}

/// The GRUB entry that undoes the change made right after `generation`.
pub fn render_entry(
    generation: &str,
    when: &str,
    version: &str,
    uuid: &str,
    cmdline: &str,
) -> String {
    let dir = format!("/@snapshots/{generation}/boot");
    let mut out = String::new();
    let _ = writeln!(out, "{BEGIN}");
    let _ = writeln!(
        out,
        "menuentry 'BunnyOS: undo the last change (back to {when})' --class bunnyos {{"
    );
    let _ = writeln!(out, "    insmod btrfs");
    let _ = writeln!(out, "    search --no-floppy --fs-uuid --set=root {uuid}");
    let _ = writeln!(out, "    echo 'Undoing the last change…'");
    let _ = writeln!(out, "    linux {dir}/vmlinuz-{version} {cmdline}");
    let _ = writeln!(out, "    initrd {dir}/initramfs-{version}.img");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "{END}");
    out
}

/// custom.cfg with bunny's block replaced by `block` (or removed if empty),
/// keeping everything else in the file.
pub fn merge_custom_cfg(existing: &str, block: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in existing.lines() {
        if line == BEGIN {
            inside = true;
            continue;
        }
        if inside {
            if line == END {
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

/// Removes bunny's undo entry from a restored system (mounted at `system`).
/// The restored system brings back its own, older custom.cfg, whose entry
/// would undo one change further back; picking it again and again would keep
/// going back. After a rollback there's no "last change" to undo until the
/// next switch, so the entry goes. Returns true if one was removed.
pub fn drop_entry_in(system: &Path) -> Result<bool> {
    let path = system.join(CUSTOM_CFG);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let without = merge_custom_cfg(&text, "");
    if without == text {
        return Ok(false);
    }
    std::fs::write(&path, without).with_context(|| format!("Couldn't write {}", path.display()))?;
    Ok(true)
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
    fn the_entry_boots_the_generations_own_kernel() {
        let entry = render_entry(
            "bunny-1",
            "2026-11-17 09:15 UTC",
            "6.12.58",
            "1234",
            "root=UUID=1234 bunny.rollback=bunny-1",
        );
        assert!(entry
            .contains("menuentry 'BunnyOS: undo the last change (back to 2026-11-17 09:15 UTC)'"));
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
        assert!(changes(&boot, root.path()).is_empty());
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
    fn boot_step_does_nothing_on_a_normal_boot_or_a_bad_name() {
        for target in ["", "../etc", "bunny-missing", "@home"] {
            let top = fake_top();
            let log = run_hook(target, top.path());
            assert!(top.path().join("@/broken").exists(), "{target}: {log}");
            assert!(!top.path().join("@/good").exists(), "{target}: {log}");
        }
    }
}
