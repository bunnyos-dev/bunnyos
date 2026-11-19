//! Which disks `bunny install` can use, from `lsblk --json`.
//!
//! A disk is offered only if nothing on it is in use: nothing mounted
//! (that includes the USB stick the installer runs from), no active swap,
//! and nothing opened on top of it (an unlocked encrypted partition, LVM,
//! RAID). Everything on it is listed, so the preview can say exactly what
//! will be erased.

use anyhow::{bail, Context, Result};
use serde_json::Value;

/// The `lsblk` command that [`parse`] reads.
pub const LSBLK: &[&str] = &[
    "lsblk",
    "--json",
    "--bytes",
    "--output",
    "NAME,PATH,SIZE,TYPE,MODEL,TRAN,RM,RO,MOUNTPOINTS,FSTYPE,LABEL",
];

/// Smallest disk worth installing on.
pub const MIN_SIZE: u64 = 20_000_000_000;

/// Something on a disk: a partition, or what's opened on top of one.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub path: String,
    pub size: u64,
    pub kind: String,
    pub fstype: Option<String>,
    pub label: Option<String>,
    pub mounted: Vec<String>,
}

/// A whole disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Disk {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub model: Option<String>,
    /// How it's connected: `nvme`, `sata`, `usb`, …
    pub transport: Option<String>,
    pub removable: bool,
    pub read_only: bool,
    /// Its partitions, and anything opened on them, depth first.
    pub parts: Vec<Part>,
    /// Mount points directly on the disk (no partition table).
    pub mounted: Vec<String>,
    pub fstype: Option<String>,
}

impl Disk {
    /// Why this disk can't be used, or `None` if it can.
    pub fn unusable(&self) -> Option<String> {
        if self.read_only {
            return Some("it's read-only".into());
        }
        if self.size < MIN_SIZE {
            return Some(format!(
                "it's too small ({}; at least 20 GB)",
                size(self.size)
            ));
        }
        let mounted: Vec<&String> = self
            .mounted
            .iter()
            .chain(self.parts.iter().flat_map(|p| &p.mounted))
            .collect();
        if let Some(m) = mounted.first() {
            return Some(if m.as_str() == "[SWAP]" {
                "it's in use as swap".into()
            } else {
                format!("it's in use (mounted at {m})")
            });
        }
        if let Some(p) = self
            .parts
            .iter()
            .find(|p| p.kind != "part" && p.kind != "disk")
        {
            return Some(format!("it's in use ({} is open on it)", p.path));
        }
        None
    }

    /// One line for the list of disks: `nvme0n1  512 GB  Samsung SSD 980 (NVMe)`.
    pub fn describe(&self) -> String {
        let mut out = format!("{}  {}", self.name, size(self.size));
        if let Some(model) = &self.model {
            out.push_str(&format!("  {model}"));
        }
        let how = match self.transport.as_deref() {
            Some("nvme") => Some("NVMe"),
            Some("sata") | Some("ata") => Some("SATA"),
            Some("usb") => Some("USB"),
            Some("virtio") | None if self.name.starts_with("vd") => Some("virtual disk"),
            _ => None,
        };
        if let Some(how) = how {
            out.push_str(&format!(" ({how})"));
        }
        out
    }

    /// What's on the disk now, one line each, for "this will be erased".
    pub fn contents(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(fs) = &self.fstype {
            out.push(format!("the whole disk is {fs}"));
        }
        for p in self.parts.iter().filter(|p| p.kind == "part") {
            let mut line = format!("{}  {}", p.path, size(p.size));
            match (&p.fstype, &p.label) {
                (Some(fs), Some(label)) => line.push_str(&format!("  {} \"{label}\"", fs_name(fs))),
                (Some(fs), None) => line.push_str(&format!("  {}", fs_name(fs))),
                (None, _) => line.push_str("  (no filesystem)"),
            }
            out.push(line);
        }
        if out.is_empty() {
            out.push("nothing (it's empty)".into());
        }
        out
    }
}

/// Filesystem types in words people know.
fn fs_name(fstype: &str) -> String {
    match fstype {
        "vfat" => "FAT (EFI or USB)".into(),
        "ntfs" => "NTFS (Windows)".into(),
        "BitLocker" => "BitLocker (encrypted Windows)".into(),
        "crypto_LUKS" => "encrypted (LUKS)".into(),
        "apfs" | "hfsplus" => "macOS".into(),
        "swap" => "swap".into(),
        other => other.to_string(),
    }
}

/// Sizes the way disk makers count them: `512 GB`, `1.0 TB`, `105 MB`.
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb < 1.0 {
        format!("{:.0} MB", bytes as f64 / 1e6)
    } else if gb >= 1000.0 {
        format!("{:.1} TB", gb / 1000.0)
    } else if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// lsblk gives sizes as numbers, or as strings on older versions.
fn number(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// lsblk gives flags as booleans, or as "0"/"1" on older versions.
fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.trim() == "1",
        Some(Value::Number(n)) => n.as_u64() == Some(1),
        _ => false,
    }
}

fn mountpoints(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = match v.get("mountpoints") {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|m| m.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };
    if let Some(m) = text(v, "mountpoint") {
        out.push(m);
    }
    out
}

fn collect_parts(children: Option<&Value>, out: &mut Vec<Part>) {
    for child in children.and_then(|c| c.as_array()).into_iter().flatten() {
        out.push(Part {
            path: text(child, "path")
                .unwrap_or_else(|| format!("/dev/{}", text(child, "name").unwrap_or_default())),
            size: number(child, "size"),
            kind: text(child, "type").unwrap_or_default(),
            fstype: text(child, "fstype"),
            label: text(child, "label"),
            mounted: mountpoints(child),
        });
        collect_parts(child.get("children"), out);
    }
}

/// The whole disks in `lsblk`'s output (not CD drives, loop devices or
/// compressed RAM).
pub fn parse(json: &str) -> Result<Vec<Disk>> {
    let v: Value = serde_json::from_str(json).context("Couldn't read lsblk's list of disks")?;
    let Some(devices) = v.get("blockdevices").and_then(|d| d.as_array()) else {
        bail!("lsblk's list of disks didn't have the expected shape");
    };
    let mut disks = Vec::new();
    for d in devices {
        let name = text(d, "name").unwrap_or_default();
        if text(d, "type").as_deref() != Some("disk") || name.starts_with("zram") {
            continue;
        }
        let mut parts = Vec::new();
        collect_parts(d.get("children"), &mut parts);
        disks.push(Disk {
            path: text(d, "path").unwrap_or_else(|| format!("/dev/{name}")),
            name,
            size: number(d, "size"),
            model: text(d, "model"),
            transport: text(d, "tran"),
            removable: flag(d, "rm"),
            read_only: flag(d, "ro"),
            parts,
            mounted: mountpoints(d),
            fstype: text(d, "fstype"),
        });
    }
    Ok(disks)
}

/// The path of partition `n` on `disk`: `/dev/sda1`, `/dev/nvme0n1p1`.
pub fn partition(disk: &str, n: u32) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{disk}p{n}")
    } else {
        format!("{disk}{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A laptop booted from the installer USB: an NVMe disk with Windows,
    /// a second SATA disk that's empty, the USB stick (mounted), and zram.
    const LSBLK_JSON: &str = r#"{
   "blockdevices": [
      {"name":"sda", "path":"/dev/sda", "size":500107862016, "type":"disk", "model":"CT500MX500SSD1", "tran":"sata", "rm":false, "ro":false, "mountpoints":[null], "fstype":null, "label":null},
      {"name":"sdb", "path":"/dev/sdb", "size":15376000000, "type":"disk", "model":"USB Flash Disk", "tran":"usb", "rm":true, "ro":false, "mountpoints":[null], "fstype":"iso9660", "label":"BUNNY",
         "children": [
            {"name":"sdb1", "path":"/dev/sdb1", "size":15376000000, "type":"part", "rm":true, "ro":false, "mountpoints":["/run/initramfs/live"], "fstype":"iso9660", "label":"BUNNY"}
         ]
      },
      {"name":"zram0", "path":"/dev/zram0", "size":8000000000, "type":"disk", "mountpoints":["[SWAP]"]},
      {"name":"nvme0n1", "path":"/dev/nvme0n1", "size":512110190592, "type":"disk", "model":"Samsung SSD 980 PRO 512GB", "tran":"nvme", "rm":false, "ro":false, "mountpoints":[null], "fstype":null, "label":null,
         "children": [
            {"name":"nvme0n1p1", "path":"/dev/nvme0n1p1", "size":104857600, "type":"part", "mountpoints":[null], "fstype":"vfat", "label":"SYSTEM"},
            {"name":"nvme0n1p2", "path":"/dev/nvme0n1p2", "size":16777216, "type":"part", "mountpoints":[null], "fstype":null, "label":null},
            {"name":"nvme0n1p3", "path":"/dev/nvme0n1p3", "size":511000000000, "type":"part", "mountpoints":[null], "fstype":"ntfs", "label":"Windows"}
         ]
      },
      {"name":"sr0", "path":"/dev/sr0", "size":1073741312, "type":"rom", "mountpoints":[null]}
   ]
}"#;

    #[test]
    fn lists_whole_disks_only() {
        let disks = parse(LSBLK_JSON).unwrap();
        let names: Vec<_> = disks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["sda", "sdb", "nvme0n1"]);
    }

    #[test]
    fn says_why_a_disk_cant_be_used() {
        let disks = parse(LSBLK_JSON).unwrap();
        assert_eq!(disks[0].unusable(), None);
        assert_eq!(
            disks[1].unusable().unwrap(),
            "it's too small (15 GB; at least 20 GB)"
        );
        assert_eq!(disks[2].unusable(), None);

        let mut usb = disks[1].clone();
        usb.size = 64_000_000_000;
        assert_eq!(
            usb.unusable().unwrap(),
            "it's in use (mounted at /run/initramfs/live)"
        );

        let mut opened = disks[0].clone();
        opened.parts.push(Part {
            path: "/dev/mapper/root".into(),
            size: 1,
            kind: "crypt".into(),
            fstype: None,
            label: None,
            mounted: vec![],
        });
        assert_eq!(
            opened.unusable().unwrap(),
            "it's in use (/dev/mapper/root is open on it)"
        );
    }

    #[test]
    fn describes_disks_and_what_is_on_them() {
        let disks = parse(LSBLK_JSON).unwrap();
        assert_eq!(
            disks[2].describe(),
            "nvme0n1  512 GB  Samsung SSD 980 PRO 512GB (NVMe)"
        );
        assert_eq!(disks[0].describe(), "sda  500 GB  CT500MX500SSD1 (SATA)");
        assert_eq!(
            disks[2].contents(),
            [
                "/dev/nvme0n1p1  105 MB  FAT (EFI or USB) \"SYSTEM\"",
                "/dev/nvme0n1p2  17 MB  (no filesystem)",
                "/dev/nvme0n1p3  511 GB  NTFS (Windows) \"Windows\"",
            ]
        );
        assert_eq!(disks[0].contents(), ["nothing (it's empty)"]);
    }

    #[test]
    fn old_lsblk_strings_still_work() {
        let json = r#"{"blockdevices":[{"name":"vda","size":"64424509440","type":"disk","rm":"0","ro":"1","mountpoint":null}]}"#;
        let disks = parse(json).unwrap();
        assert_eq!(disks[0].path, "/dev/vda");
        assert_eq!(disks[0].size, 64_424_509_440);
        assert!(disks[0].read_only);
        assert_eq!(disks[0].describe(), "vda  64 GB (virtual disk)");
    }

    #[test]
    fn names_partitions() {
        assert_eq!(partition("/dev/sda", 1), "/dev/sda1");
        assert_eq!(partition("/dev/vda", 2), "/dev/vda2");
        assert_eq!(partition("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
        assert_eq!(partition("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
    }

    #[test]
    fn sizes_read_like_the_box_says() {
        assert_eq!(size(512_110_190_592), "512 GB");
        assert_eq!(size(2_000_398_934_016), "2.0 TB");
        assert_eq!(size(2_147_483_648), "2.1 GB");
    }
}
