//! A kernel per generation, for systems where `/boot` isn't part of `@`.
//!
//! On the Bunny Linux layout `/boot` lives inside `@`, so every snapshot
//! carries its own kernel. On an encrypted disk `/boot` is usually the EFI
//! partition instead (GRUB can't read the encrypted btrfs), so snapshots have
//! an empty `/boot` and rolling back would keep the newest kernel, whose
//! modules the restored system may not have.
//!
//! So when `/boot` is a separate filesystem, bunny keeps a copy of its kernel
//! files for each generation on `/boot` itself:
//!
//! ```text
//! /boot/bunny/files/vmlinuz-6.12.58-gentoo-dist      one copy per distinct file
//! /boot/bunny/files/initramfs-6.12.58-gentoo-dist.img~2   (a rebuilt one)
//! /boot/bunny/generations/bunny-20261118-120000       "<name> <stored file>" lines
//! ```
//!
//! Identical files are stored once, so generations that share a kernel cost
//! nothing extra. Rolling back puts `/boot`'s kernel files back exactly as
//! they were for that generation; `bunny clean` removes copies nothing uses.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Where the copies live, relative to the root.
pub const STORE: &str = "boot/bunny";
/// Space left free on `/boot` after saving, so a kernel update still fits.
const RESERVE: u64 = 64 * 1024 * 1024;

/// The list of mounted filesystems for `root` (a test root brings its own).
pub fn mountinfo(root: &Path) -> String {
    std::fs::read_to_string(root.join("proc/self/mountinfo")).unwrap_or_default()
}

/// The device `/boot` is mounted from, if `/boot` is its own filesystem.
pub fn boot_device(mountinfo: &str) -> Option<String> {
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.get(4) != Some(&"/boot") {
            continue;
        }
        let dash = fields.iter().position(|f| *f == "-")?;
        return fields.get(dash + 2).map(|s| s.to_string());
    }
    None
}

/// Whether `/boot` is a separate filesystem (so snapshots don't include it).
pub fn separate_boot(mountinfo: &str) -> bool {
    boot_device(mountinfo).is_some()
}

/// The filesystem UUID of `device`, from `/dev/disk/by-uuid`.
pub fn device_uuid(root: &Path, device: &str) -> Option<String> {
    let wanted = std::fs::canonicalize(root.join(device.trim_start_matches('/'))).ok()?;
    std::fs::read_dir(root.join("dev/disk/by-uuid"))
        .ok()?
        .flatten()
        .find(|e| std::fs::canonicalize(e.path()).ok().as_ref() == Some(&wanted))
        .map(|e| e.file_name().to_string_lossy().into_owned())
}

/// Files in `/boot` that belong to a kernel: the kernel, its startup image,
/// its config and symbol map, and CPU microcode images.
pub fn is_kernel_file(name: &str) -> bool {
    if name.ends_with(".bunny-new") || name.ends_with(".bunny-tmp") {
        return false;
    }
    [
        "vmlinuz-",
        "initramfs-",
        "System.map-",
        "config-",
        "kernel-",
    ]
    .iter()
    .any(|p| name.starts_with(p))
        || name.ends_with("-uc.img")
}

/// Kernel files in `/boot` right now, sorted.
pub fn current_files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("boot"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_kernel_file(n))
        .collect();
    names.sort();
    names
}

/// Kernel versions among file names (from `vmlinuz-<version>`), sorted.
pub fn versions<'a>(names: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut found: Vec<String> = names
        .into_iter()
        .filter_map(|n| n.strip_prefix("vmlinuz-"))
        .filter(|v| !v.ends_with(".old"))
        .map(String::from)
        .collect();
    found.sort();
    found
}

fn manifest_path(root: &Path, generation: &str) -> PathBuf {
    root.join(STORE).join("generations").join(generation)
}

fn files_dir(root: &Path) -> PathBuf {
    root.join(STORE).join("files")
}

/// The kernel files saved for a generation: name in `/boot` → stored file.
pub fn load(root: &Path, generation: &str) -> Option<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(manifest_path(root, generation)).ok()?;
    Some(
        text.lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
    )
}

/// The stored copy of `name` for a generation, as a path GRUB can use on the
/// `/boot` filesystem (e.g. `/bunny/files/vmlinuz-6.12.58`).
pub fn grub_path(stored: &str) -> String {
    format!("/bunny/files/{stored}")
}

fn same_contents(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut fa), Ok(mut fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        let na = match fa.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na == 0 {
            return true;
        }
        if fb.read_exact(&mut bb[..na]).is_err() || ba[..na] != bb[..na] {
            return false;
        }
    }
}

/// An already stored copy with the same contents as `source`, if any.
fn find_stored(root: &Path, name: &str, source: &Path) -> Option<String> {
    let mut candidates: Vec<String> = std::fs::read_dir(files_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n == name || n.starts_with(&format!("{name}~")))
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .find(|c| same_contents(&files_dir(root).join(c), source))
}

/// A stored-file name for `name` that isn't taken yet.
fn free_name(root: &Path, name: &str) -> String {
    let dir = files_dir(root);
    if !dir.join(name).exists() {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name}~{n}"))
        .find(|c| !dir.join(c).exists())
        .unwrap_or_else(|| name.to_string())
}

/// Copies a file, keeping its modification time (bunny compares it with the
/// dracut module's), through a temporary name so a half-written copy is never
/// mistaken for a whole one.
fn copy_keeping_time(from: &Path, to: &Path) -> Result<()> {
    let mut tmp = to.as_os_str().to_owned();
    tmp.push(".bunny-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::copy(from, &tmp)
        .with_context(|| format!("Couldn't copy {} to {}", from.display(), tmp.display()))?;
    if let Ok(modified) = std::fs::metadata(from).and_then(|m| m.modified()) {
        if let Ok(file) = std::fs::File::options().write(true).open(&tmp) {
            let _ = file.set_modified(modified);
            let _ = file.sync_all();
        }
    }
    std::fs::rename(&tmp, to).with_context(|| format!("Couldn't put {} in place", to.display()))
}

/// Free bytes on the filesystem holding `path`.
fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `stat` is a valid out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// What [`save`] did.
#[derive(Debug, PartialEq)]
pub enum Saved {
    /// Saved; this many bytes of new copies were needed.
    Done { new_bytes: u64 },
    /// No kernel files in `/boot`, so nothing to keep.
    NothingToSave,
    /// Not enough room on `/boot`.
    NoRoom { needed: u64, free: u64 },
}

/// Keeps a copy of `/boot`'s kernel files for `generation`.
pub fn save(root: &Path, generation: &str) -> Result<Saved> {
    let names = current_files(root);
    if names.is_empty() {
        return Ok(Saved::NothingToSave);
    }
    let dir = files_dir(root);
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    std::fs::create_dir_all(root.join(STORE).join("generations"))?;

    // Work out what's new first, so nothing is copied unless it all fits.
    let mut plan: Vec<(String, Option<String>)> = Vec::new();
    let mut needed = 0;
    for name in &names {
        let source = root.join("boot").join(name);
        let existing = find_stored(root, name, &source);
        if existing.is_none() {
            needed += std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0);
        }
        plan.push((name.clone(), existing));
    }
    if needed > 0 {
        if let Some(free) = free_space(&dir) {
            if needed + RESERVE > free {
                return Ok(Saved::NoRoom { needed, free });
            }
        }
    }

    let mut manifest = String::new();
    for (name, existing) in plan {
        let stored = match existing {
            Some(s) => s,
            None => {
                let s = free_name(root, &name);
                copy_keeping_time(&root.join("boot").join(&name), &dir.join(&s))?;
                s
            }
        };
        manifest.push_str(&format!("{name} {stored}\n"));
    }
    let path = manifest_path(root, generation);
    std::fs::write(&path, manifest)
        .with_context(|| format!("Couldn't write {}", path.display()))?;
    Ok(Saved::Done { new_bytes: needed })
}

/// How `/boot` would change to match a generation.
#[derive(Debug, Default, PartialEq)]
pub struct Restore {
    /// Kernel versions in `/boot` now, and after.
    pub versions_now: Vec<String>,
    pub versions_then: Vec<String>,
    /// Files put back (missing or different now).
    pub put_back: Vec<String>,
    /// Files removed (not there at that generation).
    pub remove: Vec<String>,
}

impl Restore {
    pub fn is_empty(&self) -> bool {
        self.put_back.is_empty() && self.remove.is_empty()
    }
}

/// What restoring `generation`'s kernel files would do, or `None` if none
/// were saved for it.
pub fn restore_plan(root: &Path, generation: &str) -> Option<Restore> {
    let saved = load(root, generation)?;
    let now = current_files(root);
    let put_back = saved
        .iter()
        .filter(|(name, stored)| {
            !same_contents(&root.join("boot").join(name), &files_dir(root).join(stored))
        })
        .map(|(name, _)| name.clone())
        .collect();
    let remove = now
        .iter()
        .filter(|n| !saved.contains_key(*n))
        .cloned()
        .collect();
    Some(Restore {
        versions_now: versions(&now),
        versions_then: versions(saved.keys()),
        put_back,
        remove,
    })
}

/// Puts `/boot`'s kernel files back as they were for `generation`. Copies
/// everything in first and only then removes what wasn't there, so a failure
/// part-way never leaves `/boot` without a kernel.
pub fn restore(root: &Path, generation: &str) -> Result<Restore> {
    let plan = restore_plan(root, generation).unwrap_or_default();
    let saved = load(root, generation).unwrap_or_default();
    for name in &plan.put_back {
        let stored = &saved[name];
        copy_keeping_time(&files_dir(root).join(stored), &root.join("boot").join(name))?;
    }
    for name in &plan.remove {
        let path = root.join("boot").join(name);
        std::fs::remove_file(&path)
            .with_context(|| format!("Couldn't remove {}", path.display()))?;
    }
    Ok(plan)
}

/// Removes saved lists for generations that are gone, and copies no list
/// uses any more. Returns the number of copies removed.
pub fn tidy(root: &Path) -> Result<usize> {
    let gens_dir = root.join(STORE).join("generations");
    let mut used = BTreeSet::new();
    for entry in std::fs::read_dir(&gens_dir).into_iter().flatten().flatten() {
        let generation = entry.file_name().to_string_lossy().into_owned();
        if !root.join(".snapshots").join(&generation).is_dir() {
            std::fs::remove_file(entry.path())
                .with_context(|| format!("Couldn't remove {}", entry.path().display()))?;
            continue;
        }
        used.extend(load(root, &generation).unwrap_or_default().into_values());
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(files_dir(root))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !used.contains(&name) {
            std::fs::remove_file(entry.path())
                .with_context(|| format!("Couldn't remove {}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Kernel versions in `/boot` that the system at `system` has no modules
/// for. After a rollback these would start without most drivers.
pub fn missing_modules(root: &Path, system: &Path) -> Vec<String> {
    versions(&current_files(root))
        .into_iter()
        .filter(|v| !system.join("lib/modules").join(v).is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
22 1 0:21 /@ / rw,noatime shared:1 - btrfs /dev/mapper/root rw,subvol=/@
24 22 259:1 / /boot rw,relatime shared:3 - vfat /dev/vda1 rw
";

    fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn snapshot(root: &Path, generation: &str) {
        std::fs::create_dir_all(root.join(".snapshots").join(generation)).unwrap();
    }

    #[test]
    fn finds_a_separate_boot() {
        assert_eq!(boot_device(MOUNTINFO).as_deref(), Some("/dev/vda1"));
        assert!(separate_boot(MOUNTINFO));
        assert!(!separate_boot(MOUNTINFO.lines().next().unwrap()));
    }

    #[test]
    fn knows_kernel_files() {
        for name in [
            "vmlinuz-6.12.58-gentoo-dist",
            "initramfs-6.12.58-gentoo-dist.img",
            "System.map-6.12.58-gentoo-dist",
            "config-6.12.58-gentoo-dist",
            "intel-uc.img",
            "amd-uc.img",
            "vmlinuz-6.12.58-gentoo-dist.old",
        ] {
            assert!(is_kernel_file(name), "{name}");
        }
        for name in [
            "grub",
            "EFI",
            "initramfs-6.img.bunny-new",
            "bunny",
            "loader",
        ] {
            assert!(!is_kernel_file(name), "{name}");
        }
        let names = vec![
            "vmlinuz-6.2".to_string(),
            "vmlinuz-6.1".into(),
            "vmlinuz-6.1.old".into(),
        ];
        assert_eq!(versions(&names), vec!["6.1", "6.2"]);
    }

    #[test]
    fn saves_once_and_shares_identical_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "boot/vmlinuz-6.1", "kernel 6.1");
        write(root, "boot/initramfs-6.1.img", "image 6.1");
        write(root, "boot/grub/grub.cfg", "menu");

        assert!(matches!(
            save(root, "bunny-1").unwrap(),
            Saved::Done { new_bytes: 19 }
        ));
        // Nothing changed: the second generation reuses both copies.
        assert_eq!(save(root, "bunny-2").unwrap(), Saved::Done { new_bytes: 0 });
        // The startup image was rebuilt: same name, new contents, new copy.
        write(root, "boot/initramfs-6.1.img", "image 6.1 again");
        assert_eq!(
            save(root, "bunny-3").unwrap(),
            Saved::Done { new_bytes: 15 }
        );

        let third = load(root, "bunny-3").unwrap();
        assert_eq!(third["vmlinuz-6.1"], "vmlinuz-6.1");
        assert_eq!(third["initramfs-6.1.img"], "initramfs-6.1.img~2");
        assert!(!third.contains_key("grub.cfg"));
        assert_eq!(grub_path(&third["vmlinuz-6.1"]), "/bunny/files/vmlinuz-6.1");
    }

    #[test]
    fn nothing_to_save_without_a_kernel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("boot")).unwrap();
        assert_eq!(save(dir.path(), "bunny-1").unwrap(), Saved::NothingToSave);
        assert!(load(dir.path(), "bunny-1").is_none());
    }

    #[test]
    fn restores_boot_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "boot/vmlinuz-6.1", "kernel 6.1");
        write(root, "boot/initramfs-6.1.img", "image 6.1");
        save(root, "bunny-1").unwrap();

        // A kernel update: 6.2 arrives, 6.1's image is rebuilt, 6.1 removed.
        write(root, "boot/vmlinuz-6.2", "kernel 6.2");
        write(root, "boot/initramfs-6.2.img", "image 6.2");
        std::fs::remove_file(root.join("boot/vmlinuz-6.1")).unwrap();
        write(root, "boot/initramfs-6.1.img", "image 6.1 rebuilt");

        let plan = restore_plan(root, "bunny-1").unwrap();
        assert_eq!(plan.versions_now, vec!["6.2"]);
        assert_eq!(plan.versions_then, vec!["6.1"]);
        assert_eq!(plan.put_back, vec!["initramfs-6.1.img", "vmlinuz-6.1"]);
        assert_eq!(plan.remove, vec!["initramfs-6.2.img", "vmlinuz-6.2"]);

        restore(root, "bunny-1").unwrap();
        assert_eq!(
            current_files(root),
            vec!["initramfs-6.1.img", "vmlinuz-6.1"]
        );
        assert_eq!(
            std::fs::read_to_string(root.join("boot/initramfs-6.1.img")).unwrap(),
            "image 6.1"
        );
        assert!(restore_plan(root, "bunny-1").unwrap().is_empty());
        assert!(restore_plan(root, "bunny-unknown").is_none());
    }

    #[test]
    fn keeps_the_modification_time() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "boot/vmlinuz-6.1", "kernel");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(root.join("boot/vmlinuz-6.1"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        save(root, "bunny-1").unwrap();
        let copy = std::fs::metadata(root.join("boot/bunny/files/vmlinuz-6.1")).unwrap();
        assert_eq!(copy.modified().unwrap(), old);
    }

    #[test]
    fn tidy_removes_what_nothing_uses() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "boot/vmlinuz-6.1", "kernel 6.1");
        snapshot(root, "bunny-1");
        save(root, "bunny-1").unwrap();
        write(root, "boot/vmlinuz-6.1", "kernel 6.1 rebuilt");
        snapshot(root, "bunny-2");
        save(root, "bunny-2").unwrap();

        // Nothing is gone yet.
        assert_eq!(tidy(root).unwrap(), 0);
        // bunny-1 was cleaned: its list and its copy go.
        std::fs::remove_dir(root.join(".snapshots/bunny-1")).unwrap();
        assert_eq!(tidy(root).unwrap(), 1);
        assert!(load(root, "bunny-1").is_none());
        assert!(root.join("boot/bunny/files/vmlinuz-6.1~2").is_file());
        assert!(!root.join("boot/bunny/files/vmlinuz-6.1").exists());
    }

    #[test]
    fn notices_kernels_without_modules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "boot/vmlinuz-6.1", "k");
        write(root, "boot/vmlinuz-6.2", "k");
        std::fs::create_dir_all(root.join("lib/modules/6.1")).unwrap();
        assert_eq!(missing_modules(root, root), vec!["6.2"]);
    }

    #[test]
    fn finds_the_uuid_of_a_device() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "dev/vda1", "");
        write(root, "dev/vda2", "");
        std::fs::create_dir_all(root.join("dev/disk/by-uuid")).unwrap();
        std::os::unix::fs::symlink("../../vda2", root.join("dev/disk/by-uuid/aaaa-root")).unwrap();
        std::os::unix::fs::symlink("../../vda1", root.join("dev/disk/by-uuid/1234-ABCD")).unwrap();
        assert_eq!(device_uuid(root, "/dev/vda1").as_deref(), Some("1234-ABCD"));
        assert_eq!(device_uuid(root, "/dev/vda9"), None);
    }
}
