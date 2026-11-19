//! `bunny install --mounted <folder>`: install onto partitions you made and
//! mounted yourself. bunny doesn't partition, format or erase anything: it
//! reads what's mounted, explains anything that limits it, and writes fstab
//! and the kernel settings from it.
//!
//! What's checked:
//! - something is mounted at the folder, and it's empty (apart from the
//!   folders other partitions are mounted on, and `lost+found`)
//! - the EFI partition is mounted at `<folder>/boot`
//! - the system's filesystem: btrfs with the system in a subvolume named `@`
//!   and an `@snapshots` subvolume at `/.snapshots` gets undo; anything else
//!   works without it, and bunny says so. A btrfs filesystem mounted
//!   without subvolumes, and still empty, can get the standard ones first.
//! - encrypted partitions (LUKS) are unlocked at start-up, each one asking
//!   for its password.

use std::path::Path;

use anyhow::{bail, Result};
use serde_json::Value;

use super::layout::{Uuids, BTRFS_OPTS};
use crate::apply::Runner;

/// The `findmnt` command that lists what's mounted under a folder.
pub fn findmnt_cmd(folder: &Path) -> Vec<String> {
    [
        "findmnt",
        "--json",
        "--submounts",
        "--output",
        "TARGET,SOURCE,FSTYPE,UUID",
        "--mountpoint",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([folder.to_string_lossy().into_owned()])
    .collect()
}

/// One mounted filesystem, as the new system will see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Mount {
    /// Where it's mounted inside the new system: `/`, `/home`, `/boot`.
    pub target: String,
    /// The device, like `/dev/nvme0n1p2` or `/dev/mapper/root`.
    pub device: String,
    pub fstype: String,
    pub uuid: String,
    /// For btrfs: the subvolume, like `@home`; empty for the top level.
    pub subvol: Option<String>,
}

impl Mount {
    pub fn describe(&self) -> String {
        let what = match self.subvol.as_deref() {
            Some("") => format!("{} (no subvolume)", self.fstype),
            Some(sv) => format!("{} subvolume {sv}", self.fstype),
            None => self.fstype.clone(),
        };
        format!("{:<17} {:<20} {what}", self.target, self.device)
    }
}

/// What's mounted, and the encrypted partitions under it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mounted {
    /// Sorted so a mount comes after the one it's inside.
    pub mounts: Vec<Mount>,
    /// LUKS UUIDs to unlock at start-up, the system's first.
    pub luks: Vec<String>,
}

impl Mounted {
    pub fn get(&self, target: &str) -> Option<&Mount> {
        self.mounts.iter().find(|m| m.target == target)
    }

    /// `/etc/fstab` for the new system.
    pub fn fstab(&self) -> String {
        let mut out = String::from(
            "# Written by bunny install --mounted, from what was mounted.\n\
             # <fs>  <mount>  <type>  <options>  <dump>  <pass>\n",
        );
        for m in &self.mounts {
            let (opts, pass) = match (m.fstype.as_str(), m.subvol.as_deref()) {
                ("btrfs", Some(sv)) if !sv.is_empty() => (format!("{BTRFS_OPTS},subvol={sv}"), 0),
                ("btrfs", _) => (BTRFS_OPTS.to_string(), 0),
                ("vfat", _) => ("umask=0077".to_string(), 2),
                (_, _) if m.target == "/" => ("defaults".to_string(), 1),
                _ => ("defaults".to_string(), 2),
            };
            out.push_str(&format!(
                "UUID={}  {}  {}  {opts}  0 {pass}\n",
                m.uuid, m.target, m.fstype
            ));
        }
        out
    }

    /// Kernel command line additions.
    pub fn cmdline(&self) -> String {
        let mut words: Vec<String> = self
            .luks
            .iter()
            .map(|u| format!("rd.luks.uuid={u}"))
            .collect();
        if let Some(root) = self.get("/") {
            if let Some(sv) = root.subvol.as_deref().filter(|sv| !sv.is_empty()) {
                words.push(format!("rootflags=subvol={sv}"));
            }
        }
        words.join(" ")
    }

    /// The UUIDs the rest of the install uses.
    pub fn uuids(&self) -> Uuids {
        Uuids {
            efi: self
                .get("/boot")
                .map(|m| m.uuid.clone())
                .unwrap_or_default(),
            root_fs: self.get("/").map(|m| m.uuid.clone()).unwrap_or_default(),
            root_luks: self.luks.first().cloned(),
            home_fs: None,
            home_luks: None,
        }
    }
}

/// Reads `findmnt --json --submounts` for `folder`.
pub fn parse_findmnt(json: &str, folder: &Path) -> Result<Vec<Mount>> {
    let v: Value = serde_json::from_str(json)?;
    let mut flat = Vec::new();
    fn walk(v: &Value, out: &mut Vec<Value>) {
        if let Some(list) = v.as_array() {
            for item in list {
                out.push(item.clone());
                if let Some(children) = item.get("children") {
                    walk(children, out);
                }
            }
        }
    }
    walk(v.get("filesystems").unwrap_or(&Value::Null), &mut flat);
    let base = folder.to_string_lossy().trim_end_matches('/').to_string();
    let mut mounts = Vec::new();
    for fs in flat {
        let text = |k: &str| fs.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let target = text("target");
        let inside = if target == base || (base.is_empty() && target == "/") {
            "/".to_string()
        } else if let Some(rest) = target.strip_prefix(&format!("{base}/")) {
            format!("/{rest}")
        } else {
            continue;
        };
        let fstype = text("fstype");
        let source = text("source");
        let (device, bracket) = match source.split_once('[') {
            Some((dev, rest)) => (
                dev.to_string(),
                Some(rest.trim_end_matches(']').to_string()),
            ),
            None => (source.clone(), None),
        };
        let subvol = (fstype == "btrfs").then(|| {
            bracket
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string()
        });
        mounts.push(Mount {
            target: inside,
            device,
            fstype,
            uuid: text("uuid"),
            subvol,
        });
    }
    mounts.sort_by_key(|m| {
        if m.target == "/" {
            0
        } else {
            m.target.matches('/').count()
        }
    });
    Ok(mounts)
}

/// The LUKS UUID under a device, from `lsblk --json --inverse` (which lists
/// a device's parents as its children).
pub fn parse_luks(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    fn find(v: &Value) -> Option<String> {
        for item in v.as_array()? {
            if item.get("fstype").and_then(|f| f.as_str()) == Some("crypto_LUKS") {
                return item.get("uuid").and_then(|u| u.as_str()).map(String::from);
            }
            if let Some(found) = item.get("children").and_then(find) {
                return Some(found);
            }
        }
        None
    }
    find(v.get("blockdevices")?)
}

pub fn lsblk_inverse_cmd(device: &str) -> Vec<String> {
    [
        "lsblk",
        "--json",
        "--inverse",
        "--output",
        "PATH,FSTYPE,UUID",
        device,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Filesystems a Bunny Linux system can live on.
const SYSTEM_FILESYSTEMS: [&str; 4] = ["btrfs", "ext4", "xfs", "f2fs"];

/// What bunny found, explained.
#[derive(Debug, PartialEq, Default)]
pub struct Review {
    /// Plain-English notes, each a limit or a suggestion.
    pub notes: Vec<String>,
    /// bunny can save generations and undo.
    pub undo: bool,
    /// btrfs mounted without subvolumes and empty: bunny can make the
    /// standard ones first.
    pub offer_subvolumes: bool,
}

/// Checks what's mounted. `leftovers` are files already in the folder (not
/// counting folders other partitions are mounted on, or `lost+found`).
pub fn review(mounted: &Mounted, folder: &Path, leftovers: &[String]) -> Result<Review> {
    let shown = folder.display();
    let Some(root) = mounted.get("/") else {
        bail!(
            "Nothing is mounted at {shown}.\n  \
             Mount the partition for the system there, and the EFI partition at\n  \
             {shown}/boot, then run bunny install --mounted {shown} again."
        );
    };
    if !leftovers.is_empty() {
        bail!(
            "{shown} isn't empty (it has {}).\n  \
             bunny install puts a new system there, so it needs an empty filesystem.",
            leftovers.join(", ")
        );
    }
    if !SYSTEM_FILESYSTEMS.contains(&root.fstype.as_str()) {
        bail!(
            "The system's partition is {}, which can't hold a Linux system.\n  \
             Format it as btrfs (recommended: it's what makes undo possible) or ext4.",
            root.fstype
        );
    }
    match mounted.get("/boot") {
        None => bail!(
            "There's no EFI partition mounted at {shown}/boot.\n  \
             Bunny Linux keeps the boot menu and kernels on it. Mount it there\n  \
             (mount /dev/<EFI partition> {shown}/boot), then run this again."
        ),
        Some(boot) if boot.fstype != "vfat" => bail!(
            "{shown}/boot is {}, but it needs to be the EFI partition (FAT32).\n  \
             Mount the EFI partition at {shown}/boot instead.",
            boot.fstype
        ),
        Some(_) => {}
    }

    let mut out = Review::default();
    match (root.fstype.as_str(), root.subvol.as_deref()) {
        ("btrfs", Some("@")) => {
            out.undo = mounted
                .get("/.snapshots")
                .is_some_and(|m| m.fstype == "btrfs" && m.device == root.device);
            if !out.undo {
                out.notes.push(
                    "No @snapshots subvolume mounted at /.snapshots, so there's no undo.\n    \
                     Create one on the same filesystem and mount it there to get undo."
                        .into(),
                );
            }
            if mounted.get("/home").is_none() {
                out.notes.push(
                    "/home isn't its own subvolume or partition, so undo would also put\n    \
                     your own files back to how they were. An @home subvolume avoids that."
                        .into(),
                );
            }
        }
        ("btrfs", Some("")) => {
            out.offer_subvolumes = true;
            out.notes.push(
                "The btrfs filesystem is mounted without subvolumes, so there's no undo.\n    \
                 bunny can make the standard ones on it first (@, @home, @log, @cache,\n    \
                 @build, @snapshots), since it's empty."
                    .into(),
            );
        }
        ("btrfs", Some(other)) => out.notes.push(format!(
            "The system is in a subvolume named {other}. Undo needs it to be named @,\n    \
             so there's no undo. Everything else works."
        )),
        (fstype, _) => out.notes.push(format!(
            "The system is on {fstype}, so there's no undo (it needs btrfs). Everything\n    \
             else works: bunny keeps backups of the files it changes."
        )),
    }
    if mounted.luks.len() > 1 {
        out.notes.push(format!(
            "{} encrypted partitions: you'll type a password for each one when the\n    \
             computer starts.",
            mounted.luks.len()
        ));
    }
    Ok(out)
}

/// Files in `folder` that would be in the way: anything but the folders
/// other partitions are mounted on, and `lost+found`.
pub fn leftovers(folder: &Path, mounted: &Mounted) -> Vec<String> {
    let mount_dirs: Vec<String> = mounted
        .mounts
        .iter()
        .filter_map(|m| m.target.trim_start_matches('/').split('/').next())
        .filter(|first| !first.is_empty())
        .map(String::from)
        .collect();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "lost+found" && !mount_dirs.contains(name))
        .collect();
    out.sort();
    out
}

/// The standard subvolumes, and where each is mounted.
const SUBVOLUMES: [(&str, &str); 6] = [
    ("@", "/"),
    ("@home", "/home"),
    ("@log", "/var/log"),
    ("@cache", "/var/cache"),
    ("@build", "/var/tmp/portage"),
    ("@snapshots", "/.snapshots"),
];

/// Makes the standard subvolumes on an empty btrfs filesystem mounted
/// without them, and mounts everything again the Bunny Linux way. Other
/// partitions (the EFI one, a separate /home) are mounted back where they
/// were. Returns what's mounted afterwards.
pub fn add_subvolumes(
    mounted: &Mounted,
    folder: &Path,
    runner: &mut dyn Runner,
) -> Result<Mounted> {
    let root = mounted.get("/").expect("checked by review").clone();
    let others: Vec<Mount> = mounted
        .mounts
        .iter()
        .filter(|m| m.target != "/")
        .cloned()
        .collect();
    let at = |target: &str| {
        if target == "/" {
            folder.to_string_lossy().into_owned()
        } else {
            format!("{}{target}", folder.to_string_lossy().trim_end_matches('/'))
        }
    };
    let mut step = |what: &str, cmd: Vec<String>| -> Result<()> {
        if !runner.run(&cmd)? {
            bail!(
                "Couldn't {what} ({} failed). Nothing was installed.",
                cmd.join(" ")
            );
        }
        Ok(())
    };
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    println!("\n==> Making the subvolumes");
    step("unmount it", v(&["umount", "--recursive", &at("/")]))?;
    step(
        "mount it",
        v(&["mount", "-o", "subvolid=5", &root.device, &at("/")]),
    )?;
    let has_home = others.iter().any(|m| m.target == "/home");
    let wanted: Vec<_> = SUBVOLUMES
        .iter()
        .filter(|(sv, _)| !(has_home && *sv == "@home"))
        .collect();
    for (sv, _) in &wanted {
        step(
            &format!("create the {sv} subvolume"),
            v(&["btrfs", "subvolume", "create", &format!("{}/{sv}", at("/"))]),
        )?;
    }
    step("unmount it", v(&["umount", &at("/")]))?;

    let mut after = Mounted {
        mounts: Vec::new(),
        luks: mounted.luks.clone(),
    };
    for (sv, target) in &wanted {
        if *target != "/" {
            step(
                &format!("create {target}"),
                v(&["mkdir", "-p", &at(target)]),
            )?;
        }
        let opts = format!("{BTRFS_OPTS},subvol={sv}");
        step(
            &format!("mount {target}"),
            v(&["mount", "-o", &opts, &root.device, &at(target)]),
        )?;
        after.mounts.push(Mount {
            target: target.to_string(),
            subvol: Some(sv.to_string()),
            ..root.clone()
        });
    }
    for m in others {
        step(
            &format!("create {}", m.target),
            v(&["mkdir", "-p", &at(&m.target)]),
        )?;
        let mut cmd = v(&["mount"]);
        if let Some(sv) = m.subvol.as_deref().filter(|sv| !sv.is_empty()) {
            cmd.extend(v(&["-o", &format!("subvol={sv}")]));
        }
        cmd.extend([m.device.clone(), at(&m.target)]);
        step(&format!("mount {} again", m.target), cmd)?;
        after.mounts.push(m);
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system on LUKS + btrfs with the standard subvolumes, an EFI
    /// partition, and /home on an ext4 partition of its own.
    const FINDMNT: &str = r#"{
   "filesystems": [
      {"target":"/mnt", "source":"/dev/mapper/root[/@]", "fstype":"btrfs", "uuid":"fs-uuid",
         "children": [
            {"target":"/mnt/boot", "source":"/dev/nvme0n1p1", "fstype":"vfat", "uuid":"ABCD-1234"},
            {"target":"/mnt/.snapshots", "source":"/dev/mapper/root[/@snapshots]", "fstype":"btrfs", "uuid":"fs-uuid"},
            {"target":"/mnt/home", "source":"/dev/sdb1", "fstype":"ext4", "uuid":"home-uuid"}
         ]
      }
   ]
}"#;

    fn mounted(json: &str) -> Mounted {
        Mounted {
            mounts: parse_findmnt(json, Path::new("/mnt")).unwrap(),
            luks: vec!["luks-uuid".into()],
        }
    }

    #[test]
    fn reads_what_is_mounted() {
        let m = mounted(FINDMNT);
        let targets: Vec<_> = m.mounts.iter().map(|m| m.target.as_str()).collect();
        assert_eq!(targets, ["/", "/boot", "/.snapshots", "/home"]);
        assert_eq!(m.get("/").unwrap().device, "/dev/mapper/root");
        assert_eq!(m.get("/").unwrap().subvol.as_deref(), Some("@"));
        assert_eq!(m.get("/home").unwrap().subvol, None);
        assert!(m.get("/").unwrap().describe().contains("btrfs subvolume @"));
    }

    #[test]
    fn writes_fstab_and_kernel_settings_from_it() {
        let m = mounted(FINDMNT);
        let fstab = m.fstab();
        assert!(
            fstab.contains("UUID=fs-uuid  /  btrfs  compress=zstd:1,noatime,subvol=@  0 0"),
            "{fstab}"
        );
        assert!(
            fstab.contains("UUID=ABCD-1234  /boot  vfat  umask=0077  0 2"),
            "{fstab}"
        );
        assert!(
            fstab.contains("UUID=home-uuid  /home  ext4  defaults  0 2"),
            "{fstab}"
        );
        assert_eq!(m.cmdline(), "rd.luks.uuid=luks-uuid rootflags=subvol=@");
        let u = m.uuids();
        assert_eq!(
            (u.efi.as_str(), u.root_fs.as_str()),
            ("ABCD-1234", "fs-uuid")
        );
        assert_eq!(u.root_luks.as_deref(), Some("luks-uuid"));
    }

    #[test]
    fn the_standard_layout_gets_undo() {
        let r = review(&mounted(FINDMNT), Path::new("/mnt"), &[]).unwrap();
        assert!(r.undo && !r.offer_subvolumes);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }

    #[test]
    fn explains_what_limits_other_layouts() {
        let ext4 = r#"{"filesystems":[{"target":"/mnt","source":"/dev/sda2","fstype":"ext4","uuid":"u",
            "children":[{"target":"/mnt/boot","source":"/dev/sda1","fstype":"vfat","uuid":"e"}]}]}"#;
        let r = review(&mounted(ext4), Path::new("/mnt"), &[]).unwrap();
        assert!(!r.undo);
        assert!(
            r.notes[0].contains("on ext4, so there's no undo"),
            "{:?}",
            r.notes
        );
        assert_eq!(
            mounted(ext4).fstab().lines().nth(2).unwrap(),
            "UUID=u  /  ext4  defaults  0 1"
        );
        assert_eq!(mounted(ext4).cmdline(), "rd.luks.uuid=luks-uuid");

        let flat = ext4.replace("\"ext4\"", "\"btrfs\"");
        let r = review(&mounted(&flat), Path::new("/mnt"), &[]).unwrap();
        assert!(r.offer_subvolumes && !r.undo);
        assert!(r.notes[0].contains("without subvolumes"));

        let named = flat.replace("/dev/sda2", "/dev/sda2[/root]");
        let r = review(&mounted(&named), Path::new("/mnt"), &[]).unwrap();
        assert!(!r.offer_subvolumes && !r.undo);
        assert!(r.notes[0].contains("subvolume named root"), "{:?}", r.notes);
    }

    #[test]
    fn refuses_what_cant_work() {
        let none = r#"{"filesystems":[]}"#;
        let err = review(&mounted(none), Path::new("/mnt"), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("Nothing is mounted at /mnt"), "{err}");

        let no_boot = r#"{"filesystems":[{"target":"/mnt","source":"/dev/sda2","fstype":"ext4","uuid":"u"}]}"#;
        let err = review(&mounted(no_boot), Path::new("/mnt"), &[])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no EFI partition mounted at /mnt/boot"),
            "{err}"
        );

        let wrong_boot = r#"{"filesystems":[{"target":"/mnt","source":"/dev/sda2","fstype":"ext4","uuid":"u",
            "children":[{"target":"/mnt/boot","source":"/dev/sda1","fstype":"ext4","uuid":"e"}]}]}"#;
        let err = review(&mounted(wrong_boot), Path::new("/mnt"), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("/mnt/boot is ext4"), "{err}");

        let ntfs = no_boot.replace("ext4", "ntfs");
        let err = review(&mounted(&ntfs), Path::new("/mnt"), &[])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("ntfs, which can't hold a Linux system"),
            "{err}"
        );

        let err = review(&mounted(FINDMNT), Path::new("/mnt"), &["etc".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("/mnt isn't empty (it has etc)"), "{err}");
    }

    #[test]
    fn finds_leftover_files() {
        let dir = tempfile::tempdir().unwrap();
        for d in ["boot", "home", "lost+found", "etc"] {
            std::fs::create_dir(dir.path().join(d)).unwrap();
        }
        let m = mounted(FINDMNT);
        assert_eq!(leftovers(dir.path(), &m), ["etc"]);
    }

    #[test]
    fn finds_the_encrypted_partition_under_a_device() {
        let json = r#"{"blockdevices":[{"path":"/dev/mapper/root","fstype":"btrfs","uuid":"fs",
            "children":[{"path":"/dev/nvme0n1p2","fstype":"crypto_LUKS","uuid":"luks-uuid",
            "children":[{"path":"/dev/nvme0n1","fstype":null,"uuid":null}]}]}]}"#;
        assert_eq!(parse_luks(json).as_deref(), Some("luks-uuid"));
        assert_eq!(
            parse_luks(r#"{"blockdevices":[{"path":"/dev/sda2","fstype":"ext4","uuid":"x"}]}"#),
            None
        );
    }

    #[test]
    fn makes_the_subvolumes_and_mounts_everything_again() {
        #[derive(Default)]
        struct Recorded(Vec<String>);
        impl Runner for Recorded {
            fn run(&mut self, cmd: &[String]) -> Result<bool> {
                self.0.push(cmd.join(" "));
                Ok(true)
            }
        }
        let flat = r#"{"filesystems":[{"target":"/mnt","source":"/dev/sda2","fstype":"btrfs","uuid":"u",
            "children":[{"target":"/mnt/boot","source":"/dev/sda1","fstype":"vfat","uuid":"e"}]}]}"#;
        let mut r = Recorded::default();
        let after = add_subvolumes(&mounted(flat), Path::new("/mnt"), &mut r).unwrap();
        assert_eq!(r.0[0], "umount --recursive /mnt");
        assert_eq!(r.0[1], "mount -o subvolid=5 /dev/sda2 /mnt");
        assert!(r
            .0
            .contains(&"btrfs subvolume create /mnt/@snapshots".to_string()));
        assert!(r
            .0
            .contains(&"mount -o compress=zstd:1,noatime,subvol=@ /dev/sda2 /mnt".to_string()));
        assert_eq!(r.0.last().unwrap(), "mount /dev/sda1 /mnt/boot");
        let review = review(&after, Path::new("/mnt"), &[]).unwrap();
        assert!(review.undo, "{:?}", review.notes);
        assert!(after.fstab().contains("subvol=@home"));
    }
}
