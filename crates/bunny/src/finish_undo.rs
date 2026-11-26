//! Finishing an undo from the boot menu, on the first start afterwards.
//!
//! The boot menu's undo runs in the startup image, before the system is
//! mounted. When `/boot` is its own partition (the EFI partition on an
//! encrypted disk), the startup image can't reach it, so `/boot` keeps the
//! newer kernel while the system went back. The undo entry itself boots the
//! older kernel from its copy in `/boot/bunny/files/`, so that first start
//! works, but the next normal start would use the newer kernel, whose
//! drivers the restored system doesn't have.
//!
//! So the startup image leaves a note in the restored system
//! ([`MARKER`], holding the generation's name), and an early OpenRC service
//! ([`SERVICE`]) runs `bunny finish-undo`. That puts `/boot`'s kernel files
//! back as they were for that generation (from the copies bunny keeps for
//! each one), rebuilds the boot menu, removes the note, and writes
//! [`RECORD`] so `bunny status` can say what happened.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::apply::Runner;

/// Left by the boot menu's undo, relative to the root.
pub const MARKER: &str = "var/lib/bunny/boot-undo";
/// What finishing it did, for `bunny status`.
pub const RECORD: &str = "var/lib/bunny/boot-undo-done.toml";
/// The OpenRC service that runs `bunny finish-undo` at start-up.
pub const SERVICE: &str = "bunny-finish-undo";
pub const SERVICE_SCRIPT: &str = include_str!("boot/bunny-finish-undo.initd");

/// What finishing an undo did, kept for `bunny status`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The generation the boot menu went back to.
    pub to: String,
    /// When it was finished, like `20261119-080000`.
    pub at: String,
    /// Kernel versions in `/boot` before and after.
    #[serde(default)]
    pub kernels_before: Vec<String>,
    #[serde(default)]
    pub kernels_after: Vec<String>,
    /// What went wrong, if anything. The note stays, so the next start tries
    /// again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// What [`finish`] did.
#[derive(Debug, PartialEq)]
pub enum Finished {
    /// No undo from the boot menu is waiting.
    NothingWaiting,
    /// `/boot` is part of the system, so it already went back with it.
    NotNeeded,
    /// No kernel copies were saved for that generation (made by an older
    /// bunny), so `/boot` is left as it is.
    NoCopies(String),
    /// `/boot` already had that generation's kernel.
    AlreadyRight(String),
    /// `/boot`'s kernel was put back (the record may still note a problem
    /// with rebuilding the boot menu).
    Restored(Record),
    /// Putting it back failed; the note stays so the next start tries again.
    Failed(Record),
}

/// The generation an undo from the boot menu went back to, if one is
/// waiting to be finished.
pub fn waiting(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(MARKER)).ok()?;
    let name = text.trim();
    valid_name(name).then(|| name.to_string())
}

fn valid_name(name: &str) -> bool {
    name.starts_with("bunny-") && !name.contains('/') && !name.contains("..")
}

/// Reads what the last finished undo did.
pub fn read_record(root: &Path) -> Option<Record> {
    toml::from_str(&std::fs::read_to_string(root.join(RECORD)).ok()?).ok()
}

fn write_record(root: &Path, record: &Record) -> Result<()> {
    let path = root.join(RECORD);
    std::fs::write(&path, toml::to_string(record)?)
        .with_context(|| format!("Couldn't write {}", path.display()))
}

fn clear_marker(root: &Path) -> Result<()> {
    let path = root.join(MARKER);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("Couldn't remove {}", path.display())),
    }
}

/// Finishes a waiting undo from the boot menu: puts `/boot`'s kernel files
/// back for that generation and rebuilds the boot menu. Commands go through
/// `runner`, so a test root only prints them.
pub fn finish(root: &Path, runner: &mut dyn Runner, stamp: &str) -> Result<Finished> {
    let Some(generation) = waiting(root) else {
        // A note with a name that isn't a generation is never acted on.
        if root.join(MARKER).exists() {
            clear_marker(root)?;
        }
        return Ok(Finished::NothingWaiting);
    };
    // Only while that undo is still the last thing that happened. A note
    // carried into a later snapshot (a switch made before it was finished)
    // is stale: acting on it would put the wrong kernel back.
    let current = crate::notes::read_state(root).is_some_and(|s| {
        s.last == crate::notes::Kind::Rollback && s.to.as_deref() == Some(generation.as_str())
    });
    if !current {
        clear_marker(root)?;
        return Ok(Finished::NothingWaiting);
    }
    if !crate::kernels::separate_boot(&crate::kernels::mountinfo(root)) {
        clear_marker(root)?;
        return Ok(Finished::NotNeeded);
    }
    // The undo entry the startup image couldn't remove (it's on /boot).
    if let Err(e) = crate::boot::drop_entry_in(root) {
        eprintln!("  Note: couldn't tidy the boot menu ({e:#}).");
    }
    let plan = match crate::kernels::restore_plan(root, &generation) {
        None => {
            clear_marker(root)?;
            return Ok(Finished::NoCopies(generation));
        }
        Some(plan) if plan.is_empty() => {
            clear_marker(root)?;
            return Ok(Finished::AlreadyRight(generation));
        }
        Some(plan) => plan,
    };
    let mut record = Record {
        to: generation.clone(),
        at: stamp.to_string(),
        kernels_before: plan.versions_now.clone(),
        kernels_after: plan.versions_then.clone(),
        problem: None,
    };
    if let Err(e) = crate::kernels::restore(root, &generation) {
        record.problem = Some(format!("couldn't copy the kernel files back ({e:#})"));
        write_record(root, &record)?;
        return Ok(Finished::Failed(record));
    }
    let cfg = root.join("boot/grub/grub.cfg");
    if cfg.is_file() {
        let command = ["grub-mkconfig", "-o", &cfg.to_string_lossy()].map(String::from);
        let ok = runner.run(&command)?;
        if !ok {
            record.problem = Some(
                "the kernel files are back, but the boot menu couldn't be rebuilt".to_string(),
            );
        }
    }
    write_record(root, &record)?;
    if record.problem.is_some() {
        return Ok(Finished::Failed(record));
    }
    clear_marker(root)?;
    Ok(Finished::Restored(record))
}

/// What [`finish`] did, in plain words (for the start-up log and
/// `bunny finish-undo`).
pub fn describe(finished: &Finished) -> String {
    match finished {
        Finished::NothingWaiting => "There's no undo from the boot menu to finish.".to_string(),
        Finished::NotNeeded => "The undo from the boot menu is complete: /boot is part of the \
                                system, so its kernel went back with it."
            .to_string(),
        Finished::NoCopies(generation) => format!(
            "No copy of the kernel was kept for {generation} (it was saved by an older bunny), \
             so /boot keeps the kernel it has. `bunny status` says if that's a problem."
        ),
        Finished::AlreadyRight(generation) => {
            format!("/boot already has the kernel that {generation} used. Nothing to do.")
        }
        Finished::Restored(record) => {
            let then = record.kernels_after.join(", ");
            if record.kernels_before == record.kernels_after {
                format!("/boot's kernel {then} is back as it was, and the boot menu is up to date.")
            } else {
                format!(
                    "/boot has kernel {then} again (it had {}), and the boot menu is up to date.",
                    record.kernels_before.join(", ")
                )
            }
        }
        Finished::Failed(record) => format!(
            "Couldn't finish the undo from the boot menu: {}. bunny tries again at the next \
             start, or run: {} bunny finish-undo",
            record.problem.as_deref().unwrap_or("something went wrong"),
            crate::security::admin()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
22 1 0:21 /@ / rw,noatime shared:1 - btrfs /dev/mapper/root rw,subvol=/@
24 22 259:1 / /boot rw,relatime shared:3 - vfat /dev/vda1 rw
";

    struct Recorder {
        commands: Vec<String>,
        succeed: bool,
    }

    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.commands.push(cmd.join(" "));
            Ok(self.succeed)
        }
    }

    fn recorder() -> Recorder {
        Recorder {
            commands: vec![],
            succeed: true,
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn rolled_back_to(generation: &str) -> crate::notes::State {
        crate::notes::State {
            last: crate::notes::Kind::Rollback,
            at: "20261119-075900".into(),
            to: Some(generation.into()),
        }
    }

    #[test]
    fn a_stale_note_is_dropped() {
        let dir = undone_system();
        let r = dir.path();
        // A switch happened since (the note came back with a later snapshot).
        crate::notes::write_state(
            r,
            &crate::notes::State {
                last: crate::notes::Kind::Switch,
                at: "20261119-090000".into(),
                to: None,
            },
        )
        .unwrap();
        let mut runner = recorder();
        assert_eq!(
            finish(r, &mut runner, "x").unwrap(),
            Finished::NothingWaiting
        );
        assert!(r.join("boot/vmlinuz-6.12.60").exists());
        assert!(!r.join(MARKER).exists());
        // Another rollback, to a different generation, makes it stale too.
        write(r, MARKER, "bunny-old\n");
        crate::notes::write_state(r, &rolled_back_to("bunny-other")).unwrap();
        assert_eq!(
            finish(r, &mut runner, "x").unwrap(),
            Finished::NothingWaiting
        );
        assert!(r.join("boot/vmlinuz-6.12.60").exists());
    }

    /// A system that was undone from the boot menu to `bunny-old`, whose
    /// kernel was 6.12.58; /boot still has 6.12.60 from the newer version.
    fn undone_system() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "proc/self/mountinfo", MOUNTINFO);
        write(r, MARKER, "bunny-old\n");
        std::fs::create_dir_all(r.join(".snapshots/bunny-old")).unwrap();
        crate::notes::write_state(r, &rolled_back_to("bunny-old")).unwrap();
        write(r, "boot/grub/grub.cfg", "menu");
        write(
            r,
            "boot/grub/custom.cfg",
            "### BEGIN bunny: undo entry (written by bunny; edit bunny.toml instead)\n\
             menuentry 'undo' {}\n### END bunny\n",
        );
        write(r, "boot/bunny/files/vmlinuz-6.12.58", "old kernel");
        write(r, "boot/bunny/files/initramfs-6.12.58.img", "old image");
        write(
            r,
            "boot/bunny/generations/bunny-old",
            "vmlinuz-6.12.58 vmlinuz-6.12.58\ninitramfs-6.12.58.img initramfs-6.12.58.img\n",
        );
        write(r, "boot/vmlinuz-6.12.60", "new kernel");
        write(r, "boot/initramfs-6.12.60.img", "new image");
        std::fs::create_dir_all(r.join("lib/modules/6.12.58")).unwrap();
        dir
    }

    #[test]
    fn puts_the_kernel_back_and_rebuilds_the_menu() {
        let dir = undone_system();
        let r = dir.path();
        let mut runner = recorder();
        let done = finish(r, &mut runner, "20261119-080000").unwrap();

        let Finished::Restored(record) = &done else {
            panic!("{done:?}");
        };
        assert_eq!(record.kernels_before, ["6.12.60"]);
        assert_eq!(record.kernels_after, ["6.12.58"]);
        assert_eq!(
            std::fs::read_to_string(r.join("boot/vmlinuz-6.12.58")).unwrap(),
            "old kernel"
        );
        assert!(!r.join("boot/vmlinuz-6.12.60").exists());
        assert!(!r.join("boot/initramfs-6.12.60.img").exists());
        assert_eq!(
            runner.commands,
            [format!(
                "grub-mkconfig -o {}",
                r.join("boot/grub/grub.cfg").display()
            )]
        );
        // The stale undo entry is gone, the note is cleared, and the record
        // is there for `bunny status`.
        assert!(!std::fs::read_to_string(r.join("boot/grub/custom.cfg"))
            .unwrap()
            .contains("undo"));
        assert!(!r.join(MARKER).exists());
        assert_eq!(read_record(r).as_ref(), Some(record));
        // The kernel warning in `bunny status` is gone too.
        assert!(crate::kernels::missing_modules(r, r).is_empty());

        let text = describe(&done);
        assert!(
            text.contains("/boot has kernel 6.12.58 again (it had 6.12.60)"),
            "{text}"
        );
    }

    #[test]
    fn does_nothing_without_a_note() {
        let dir = undone_system();
        let r = dir.path();
        std::fs::remove_file(r.join(MARKER)).unwrap();
        let mut runner = recorder();
        assert_eq!(
            finish(r, &mut runner, "x").unwrap(),
            Finished::NothingWaiting
        );
        assert!(runner.commands.is_empty());
        assert!(r.join("boot/vmlinuz-6.12.60").exists());
    }

    #[test]
    fn a_bad_name_in_the_note_is_ignored() {
        for name in ["../etc", "bunny-x/../../y", "@home"] {
            let dir = undone_system();
            let r = dir.path();
            write(r, MARKER, name);
            let mut runner = recorder();
            assert_eq!(
                finish(r, &mut runner, "x").unwrap(),
                Finished::NothingWaiting,
                "{name}"
            );
            assert!(r.join("boot/vmlinuz-6.12.60").exists(), "{name}");
            assert!(!r.join(MARKER).exists(), "{name}");
        }
    }

    #[test]
    fn boot_inside_the_system_needs_nothing() {
        let dir = undone_system();
        let r = dir.path();
        write(r, "proc/self/mountinfo", MOUNTINFO.lines().next().unwrap());
        let mut runner = recorder();
        assert_eq!(finish(r, &mut runner, "x").unwrap(), Finished::NotNeeded);
        assert!(!r.join(MARKER).exists());
        assert!(r.join("boot/vmlinuz-6.12.60").exists());
    }

    #[test]
    fn no_copies_leaves_boot_alone() {
        let dir = undone_system();
        let r = dir.path();
        std::fs::remove_file(r.join("boot/bunny/generations/bunny-old")).unwrap();
        let mut runner = recorder();
        assert_eq!(
            finish(r, &mut runner, "x").unwrap(),
            Finished::NoCopies("bunny-old".into())
        );
        assert!(r.join("boot/vmlinuz-6.12.60").exists());
        assert!(!r.join(MARKER).exists());
    }

    #[test]
    fn already_right_changes_nothing() {
        let dir = undone_system();
        let r = dir.path();
        std::fs::remove_file(r.join("boot/vmlinuz-6.12.60")).unwrap();
        std::fs::remove_file(r.join("boot/initramfs-6.12.60.img")).unwrap();
        write(r, "boot/vmlinuz-6.12.58", "old kernel");
        write(r, "boot/initramfs-6.12.58.img", "old image");
        let mut runner = recorder();
        assert_eq!(
            finish(r, &mut runner, "x").unwrap(),
            Finished::AlreadyRight("bunny-old".into())
        );
        assert!(runner.commands.is_empty());
    }

    #[test]
    fn a_failed_menu_rebuild_is_recorded_and_tried_again() {
        let dir = undone_system();
        let r = dir.path();
        let mut runner = Recorder {
            commands: vec![],
            succeed: false,
        };
        let done = finish(r, &mut runner, "x").unwrap();
        let Finished::Failed(record) = &done else {
            panic!("{done:?}");
        };
        assert!(record.problem.as_deref().unwrap().contains("boot menu"));
        // The kernel files are back; the note stays for the next start.
        assert!(r.join("boot/vmlinuz-6.12.58").exists());
        assert!(r.join(MARKER).exists());
        assert!(describe(&done).contains("bunny finish-undo"));
    }
}
