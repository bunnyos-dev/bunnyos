//! The Bunny Linux disk layout, and the commands that make it.
//!
//! ```text
//! GPT
//! ├─ 1  EFI   2 GB   FAT32   mounted at /boot (boot loader and kernels)
//! └─ 2  rest         LUKS2 (unless --no-encrypt)
//!    └─ btrfs: @ (/), @home, @log, @cache, @build (/var/tmp/portage),
//!       @snapshots, and @swap (/swap) with a swapfile if asked for
//! ```
//!
//! `/boot` is the EFI partition so the boot menu can start the kernel
//! without unlocking the disk; bunny keeps a copy of each generation's kernel
//! there (see `kernels.rs`), so undo still puts the kernel back.
//!
//! With `--home-disk`, `/home` gets a whole second disk instead (encrypted
//! too, unlocked at boot with a key file kept on the encrypted system).

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::disks::partition;
use super::say;
use crate::apply::Runner;

/// btrfs mount options for every subvolume.
pub const BTRFS_OPTS: &str = "compress=zstd:1,noatime";
/// The names the encrypted partitions are opened as.
pub const ROOT_MAPPER: &str = "bunny-root";
pub const HOME_MAPPER: &str = "bunny-home";
/// Where the key file for a second /home disk lives, in the new system.
pub const HOME_KEY: &str = "etc/bunny/keys/home.key";

/// What the person chose.
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    /// The disk to erase, e.g. `/dev/nvme0n1`.
    pub disk: String,
    pub encrypt: bool,
    /// Size of the EFI partition, in MiB.
    pub efi_mib: u64,
    /// Size of the swapfile in MiB; 0 for none.
    pub swap_mib: u64,
    /// Set up resuming from the swapfile (hibernation).
    pub hibernate: bool,
    /// A second disk, all of it for /home.
    pub home_disk: Option<String>,
    /// Where the new system is mounted while it's installed.
    pub mnt: PathBuf,
    /// `--mounted`: partitions you made and mounted yourself. bunny
    /// doesn't touch the disks, and fstab and the kernel settings come from
    /// what's mounted.
    pub mounted: Option<super::mounted::Mounted>,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            disk: String::new(),
            encrypt: true,
            efi_mib: 2048,
            swap_mib: 0,
            hibernate: false,
            home_disk: None,
            mnt: PathBuf::from("/mnt/bunny"),
            mounted: None,
        }
    }
}

/// The devices the layout ends up with.
#[derive(Debug, PartialEq)]
pub struct Devices {
    pub efi: String,
    /// The second partition, encrypted or not.
    pub root_part: String,
    /// Where the btrfs filesystem is: the opened LUKS device, or `root_part`.
    pub root_fs: String,
    pub home_part: Option<String>,
    pub home_fs: Option<String>,
}

/// Filesystem and LUKS UUIDs, read back after formatting.
#[derive(Debug, Default, PartialEq)]
pub struct Uuids {
    pub efi: String,
    pub root_fs: String,
    pub root_luks: Option<String>,
    pub home_fs: Option<String>,
    pub home_luks: Option<String>,
}

impl Choices {
    pub fn devices(&self) -> Devices {
        let root_part = partition(&self.disk, 2);
        let home_part = self.home_disk.as_deref().map(|d| partition(d, 1));
        Devices {
            efi: partition(&self.disk, 1),
            root_fs: if self.encrypt {
                format!("/dev/mapper/{ROOT_MAPPER}")
            } else {
                root_part.clone()
            },
            root_part,
            home_fs: home_part.as_ref().map(|p| {
                if self.encrypt {
                    format!("/dev/mapper/{HOME_MAPPER}")
                } else {
                    p.clone()
                }
            }),
            home_part,
        }
    }

    /// Subvolumes on the main disk and where they're mounted, `@` first.
    pub fn subvolumes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![("@", "/")];
        if self.home_disk.is_none() {
            out.push(("@home", "/home"));
        }
        out.extend([
            ("@log", "/var/log"),
            ("@cache", "/var/cache"),
            ("@build", "/var/tmp/portage"),
            ("@snapshots", "/.snapshots"),
        ]);
        if self.swap_mib > 0 {
            out.push(("@swap", "/swap"));
        }
        out
    }

    /// "What you'll get", one line each.
    pub fn summary(&self) -> Vec<String> {
        let d = self.devices();
        let mut out = vec![format!(
            "{}  {}  EFI partition, mounted at /boot (boot menu and kernels)",
            d.efi,
            mib(self.efi_mib)
        )];
        out.push(format!(
            "{}  the rest  {}",
            d.root_part,
            if self.encrypt {
                "encrypted (LUKS2); you type the password when the computer starts"
            } else {
                "not encrypted"
            }
        ));
        let names: Vec<&str> = self.subvolumes().iter().map(|(sv, _)| *sv).collect();
        out.push(format!(
            "   btrfs, compressed, with subvolumes {}",
            names.join(" ")
        ));
        out.push(
            "   Your files (/home), logs and caches are kept apart from the system, so undo never touches them."
                .into(),
        );
        if self.swap_mib > 0 {
            out.push(format!(
                "   A {} swapfile in /swap{}",
                mib(self.swap_mib),
                if self.hibernate {
                    ", set up for hibernation"
                } else {
                    ""
                }
            ));
        }
        if let (Some(disk), Some(part)) = (&self.home_disk, &d.home_part) {
            out.push(format!(
                "{disk}: one partition, {part}, all of it for /home{}",
                if self.encrypt {
                    " (encrypted, unlocked with the system)"
                } else {
                    ""
                }
            ));
        }
        out
    }

    /// The partition table for the main disk, in sfdisk's format.
    pub fn sfdisk_script(&self) -> String {
        format!(
            "label: gpt\nsize={}MiB, type=U, name=EFI\ntype=L, name=\"Bunny Linux\"\n",
            self.efi_mib
        )
    }

    fn mnt(&self, path: &str) -> String {
        let p = path.trim_start_matches('/');
        if p.is_empty() {
            self.mnt.to_string_lossy().into_owned()
        } else {
            self.mnt.join(p).to_string_lossy().into_owned()
        }
    }

    /// `/etc/fstab` for the new system.
    pub fn fstab(&self, u: &Uuids) -> String {
        if let Some(m) = &self.mounted {
            return m.fstab();
        }
        let mut out = String::from(
            "# Written by bunny install. The layout is described in the Guide:\n\
             # \"The disk layout\".\n\
             # <fs>  <mount>  <type>  <options>  <dump>  <pass>\n",
        );
        for (sv, at) in self.subvolumes() {
            out.push_str(&format!(
                "UUID={}  {at}  btrfs  {BTRFS_OPTS},subvol={sv}  0 0\n",
                u.root_fs
            ));
        }
        if let Some(home) = &u.home_fs {
            out.push_str(&format!(
                "UUID={home}  /home  btrfs  {BTRFS_OPTS},subvol=@home  0 0\n"
            ));
        }
        out.push_str(&format!("UUID={}  /boot  vfat  umask=0077  0 2\n", u.efi));
        if self.swap_mib > 0 {
            out.push_str("/swap/swapfile  none  swap  defaults  0 0\n");
        }
        out
    }

    /// Kernel command line additions (GRUB adds `root=` itself).
    pub fn cmdline(&self, u: &Uuids, resume_offset: Option<&str>) -> String {
        if let Some(m) = &self.mounted {
            return m.cmdline();
        }
        let mut words = Vec::new();
        if let Some(luks) = &u.root_luks {
            words.push(format!("rd.luks.uuid={luks}"));
        }
        words.push("rootflags=subvol=@".to_string());
        if self.hibernate {
            if let Some(offset) = resume_offset {
                words.push(format!("resume=UUID={}", u.root_fs));
                words.push(format!("resume_offset={offset}"));
            }
        }
        words.join(" ")
    }

    /// OpenRC's dmcrypt setting that unlocks a second /home disk at boot.
    pub fn dmcrypt_conf(&self, u: &Uuids) -> Option<String> {
        let luks = u.home_luks.as_ref()?;
        Some(format!(
            "\n# Added by bunny install: unlock the /home disk with its key file.\n\
             target={HOME_MAPPER}\n\
             source=UUID={luks}\n\
             key=/{HOME_KEY}\n"
        ))
    }
}

/// `2 GB`, `512 MB`, `1.5 GB`, from MiB, matching what was typed (`2G`).
pub fn mib(m: u64) -> String {
    if m < 1024 {
        format!("{m} MB")
    } else if m.is_multiple_of(1024) {
        format!("{} GB", m / 1024)
    } else {
        format!("{:.1} GB", m as f64 / 1024.0)
    }
}

/// Parses sizes like `2G`, `512M`, `4GiB`, `8g` into MiB.
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim().to_ascii_lowercase();
    let t = t
        .trim_end_matches("ib")
        .trim_end_matches('b')
        .trim_end_matches("ib");
    let (num, unit) = t.split_at(t.find(|c: char| !c.is_ascii_digit() && c != '.')?);
    let n: f64 = num.parse().ok()?;
    let mib = match unit {
        "g" => n * 1024.0,
        "m" => n,
        "t" => n * 1024.0 * 1024.0,
        _ => return None,
    };
    (mib >= 1.0).then_some(mib.round() as u64)
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// Runs one step; on failure, stops with a plain explanation.
fn step(runner: &mut dyn Runner, what: &str, cmd: &[&str]) -> Result<()> {
    if !runner.run(&s(cmd))? {
        bail!("{}", failed(what, cmd[0]));
    }
    Ok(())
}

fn step_with_input(
    runner: &mut dyn Runner,
    what: &str,
    cmd: &[&str],
    input: &str,
    secret: bool,
) -> Result<()> {
    if !runner.run_with_input(&s(cmd), input, secret)? {
        bail!("{}", failed(what, cmd[0]));
    }
    Ok(())
}

fn failed(what: &str, tool: &str) -> String {
    format!(
        "Couldn't {what} ({tool} failed; its message is above).\n  \
         The disk is only partly set up, so nothing on it is usable yet.\n  \
         Fix the problem and run bunny install again: it starts over from the beginning."
    )
}

/// Erases the disk(s), makes the layout and mounts it at `choices.mnt`.
/// `home_key` is a file holding a random key for the /home disk (only used
/// with `--home-disk` and encryption).
pub fn prepare(
    choices: &Choices,
    password: &str,
    home_key: &Path,
    runner: &mut dyn Runner,
) -> Result<()> {
    let d = choices.devices();
    let disk = choices.disk.as_str();

    say(
        runner,
        &format!("Partitioning {disk}"),
        &format!(
            "Erases {disk}'s partition table and makes two partitions: EFI ({}),\n\
             which the computer starts from, and the rest for Bunny Linux.",
            mib(choices.efi_mib)
        ),
    );
    step(
        runner,
        "clear the old partition table",
        &["wipefs", "--all", "--force", disk],
    )?;
    step_with_input(
        runner,
        "write the new partition table",
        &[
            "sfdisk",
            "--quiet",
            "--wipe",
            "always",
            "--wipe-partitions",
            "always",
            disk,
        ],
        &choices.sfdisk_script(),
        false,
    )?;
    // Wait for the new partitions to appear.
    let _ = runner.run(&s(&["udevadm", "settle"]));

    say(
        runner,
        "Creating the EFI partition's filesystem",
        "Formats the EFI partition as FAT32, the format computers start from.",
    );
    step(
        runner,
        "format the EFI partition",
        &["mkfs.vfat", "-F", "32", "-n", "EFI", &d.efi],
    )?;

    if choices.encrypt {
        say(
            runner,
            "Encrypting the disk (LUKS2)",
            "Encrypts the Bunny Linux partition with your password, then unlocks\n\
             it so the install can write to it.",
        );
        step_with_input(
            runner,
            "encrypt the disk",
            &[
                "cryptsetup",
                "luksFormat",
                "--type",
                "luks2",
                "--batch-mode",
                "--key-file=-",
                &d.root_part,
            ],
            password,
            true,
        )?;
        step_with_input(
            runner,
            "unlock the new encrypted partition",
            &[
                "cryptsetup",
                "open",
                "--key-file=-",
                &d.root_part,
                ROOT_MAPPER,
            ],
            password,
            true,
        )?;
    }

    let names: Vec<&str> = choices.subvolumes().iter().map(|(sv, _)| *sv).collect();
    say(
        runner,
        "Creating the btrfs filesystem and its subvolumes",
        &format!(
            "Makes a btrfs filesystem with subvolumes, so the system can be saved\n\
             and undone without touching your files:\n{}.",
            names.join(", ")
        ),
    );
    step(
        runner,
        "create the btrfs filesystem",
        &["mkfs.btrfs", "--force", "--label", "bunny", &d.root_fs],
    )?;
    let mnt = choices.mnt("/");
    step(runner, "create the mount folder", &["mkdir", "-p", &mnt])?;
    step(
        runner,
        "mount the new filesystem",
        &["mount", &d.root_fs, &mnt],
    )?;
    for (sv, _) in choices.subvolumes() {
        let path = choices.mnt(sv);
        step(
            runner,
            &format!("create the {sv} subvolume"),
            &["btrfs", "subvolume", "create", &path],
        )?;
    }
    step(runner, "unmount the new filesystem", &["umount", &mnt])?;

    if let (Some(home_disk), Some(home_part), Some(home_fs)) =
        (&choices.home_disk, &d.home_part, &d.home_fs)
    {
        prepare_home_disk(
            choices, home_disk, home_part, home_fs, password, home_key, runner,
        )?;
    }

    say(
        runner,
        &format!("Mounting the new system at {mnt}"),
        "Mounts the subvolumes and the EFI partition (at /boot) where the new\n\
         system is put together.",
    );
    let opts = |sv: &str| format!("{BTRFS_OPTS},subvol={sv}");
    step(
        runner,
        "mount the system subvolume",
        &["mount", "-o", &opts("@"), &d.root_fs, &mnt],
    )?;
    for (sv, at) in choices.subvolumes().into_iter().skip(1) {
        let path = choices.mnt(at);
        step(runner, &format!("create {at}"), &["mkdir", "-p", &path])?;
        step(
            runner,
            &format!("mount {at}"),
            &["mount", "-o", &opts(sv), &d.root_fs, &path],
        )?;
    }
    if let Some(home_fs) = &d.home_fs {
        let path = choices.mnt("/home");
        step(runner, "create /home", &["mkdir", "-p", &path])?;
        step(
            runner,
            "mount /home",
            &["mount", "-o", &opts("@home"), home_fs, &path],
        )?;
    }
    let boot = choices.mnt("/boot");
    step(runner, "create /boot", &["mkdir", "-p", &boot])?;
    step(
        runner,
        "mount the EFI partition",
        &["mount", "-o", "umask=0077", &d.efi, &boot],
    )?;

    if choices.swap_mib > 0 {
        say(
            runner,
            "Making the swapfile",
            &format!("Makes a {} swapfile in /swap.", mib(choices.swap_mib)),
        );
        step(
            runner,
            "make the swapfile",
            &[
                "btrfs",
                "filesystem",
                "mkswapfile",
                "--size",
                &format!("{}m", choices.swap_mib),
                &choices.mnt("/swap/swapfile"),
            ],
        )?;
    }
    Ok(())
}

fn prepare_home_disk(
    choices: &Choices,
    disk: &str,
    part: &str,
    fs: &str,
    password: &str,
    home_key: &Path,
    runner: &mut dyn Runner,
) -> Result<()> {
    say(
        runner,
        &format!("Setting up {disk} for /home"),
        &if choices.encrypt {
            format!(
                "Erases {disk} and makes one encrypted partition for /home. It unlocks\n\
                 with a key file kept in the encrypted system, so one password opens both."
            )
        } else {
            format!("Erases {disk} and makes one partition for /home.")
        },
    );
    step(
        runner,
        "clear the /home disk",
        &["wipefs", "--all", "--force", disk],
    )?;
    step_with_input(
        runner,
        "partition the /home disk",
        &[
            "sfdisk",
            "--quiet",
            "--wipe",
            "always",
            "--wipe-partitions",
            "always",
            disk,
        ],
        "label: gpt\ntype=L, name=\"Bunny Linux home\"\n",
        false,
    )?;
    let _ = runner.run(&s(&["udevadm", "settle"]));
    if choices.encrypt {
        step_with_input(
            runner,
            "encrypt the /home disk",
            &[
                "cryptsetup",
                "luksFormat",
                "--type",
                "luks2",
                "--batch-mode",
                "--key-file=-",
                part,
            ],
            password,
            true,
        )?;
        // A random key so it unlocks at boot without a second password; the
        // password stays as a way in if the key file is ever lost.
        let key = home_key.to_string_lossy();
        step_with_input(
            runner,
            "add the /home disk's key file",
            &[
                "cryptsetup",
                "luksAddKey",
                "--batch-mode",
                "--key-file=-",
                part,
                &key,
            ],
            password,
            true,
        )?;
        step(
            runner,
            "unlock the /home disk",
            &["cryptsetup", "open", "--key-file", &key, part, HOME_MAPPER],
        )?;
    }
    step(
        runner,
        "create the /home filesystem",
        &["mkfs.btrfs", "--force", "--label", "bunny-home", fs],
    )?;
    let tmp = choices.mnt("/");
    step(runner, "mount the /home disk", &["mount", fs, &tmp])?;
    let sv = choices.mnt("@home");
    step(
        runner,
        "create the @home subvolume",
        &["btrfs", "subvolume", "create", &sv],
    )?;
    step(runner, "unmount the /home disk", &["umount", &tmp])?;
    Ok(())
}

/// Reads the UUIDs back from the new filesystems (for fstab and the boot
/// menu). On a dry run there are none, so placeholders stand in.
pub fn read_uuids(choices: &Choices, runner: &mut dyn Runner) -> Result<Uuids> {
    let d = choices.devices();
    let mut uuid = |dev: &str| -> Result<String> {
        let out = runner.output(&s(&[
            "blkid",
            "--match-tag",
            "UUID",
            "--output",
            "value",
            dev,
        ]))?;
        match out.map(|o| o.trim().to_string()).filter(|o| !o.is_empty()) {
            Some(u) => Ok(u),
            None if runner.dry() => Ok(format!("<UUID of {dev}>")),
            None => bail!("Couldn't read the UUID of {dev} (blkid found none)."),
        }
    };
    Ok(Uuids {
        efi: uuid(&d.efi)?,
        root_fs: uuid(&d.root_fs)?,
        root_luks: if choices.encrypt {
            Some(uuid(&d.root_part)?)
        } else {
            None
        },
        home_fs: match &d.home_fs {
            Some(fs) => Some(uuid(fs)?),
            None => None,
        },
        home_luks: match (&d.home_part, choices.encrypt) {
            (Some(p), true) => Some(uuid(p)?),
            _ => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        commands: Vec<String>,
        inputs: Vec<(String, bool)>,
    }

    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.commands.push(cmd.join(" "));
            Ok(true)
        }
        fn run_with_input(&mut self, cmd: &[String], input: &str, secret: bool) -> Result<bool> {
            self.commands.push(cmd.join(" "));
            self.inputs.push((input.to_string(), secret));
            Ok(true)
        }
        fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
            Ok(Some(format!("uuid-of-{}\n", cmd.last().unwrap())))
        }
    }

    fn nvme() -> Choices {
        Choices {
            disk: "/dev/nvme0n1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn devices_follow_the_disk_name() {
        let d = nvme().devices();
        assert_eq!(d.efi, "/dev/nvme0n1p1");
        assert_eq!(d.root_part, "/dev/nvme0n1p2");
        assert_eq!(d.root_fs, "/dev/mapper/bunny-root");
        let plain = Choices {
            disk: "/dev/sda".into(),
            encrypt: false,
            ..Default::default()
        };
        assert_eq!(plain.devices().root_fs, "/dev/sda2");
    }

    #[test]
    fn makes_the_standard_layout_in_order() {
        let mut r = Recorder::default();
        prepare(&nvme(), "secret pw", Path::new("/run/k"), &mut r).unwrap();
        let c = &r.commands;
        assert_eq!(c[0], "wipefs --all --force /dev/nvme0n1");
        assert_eq!(
            c[1],
            "sfdisk --quiet --wipe always --wipe-partitions always /dev/nvme0n1"
        );
        assert!(c.contains(&"mkfs.vfat -F 32 -n EFI /dev/nvme0n1p1".to_string()));
        let format = c
            .iter()
            .position(|x| x.starts_with("cryptsetup luksFormat"))
            .unwrap();
        let open = c
            .iter()
            .position(|x| x.starts_with("cryptsetup open"))
            .unwrap();
        let mkfs = c.iter().position(|x| x.starts_with("mkfs.btrfs")).unwrap();
        assert!(format < open && open < mkfs, "{c:?}");
        assert!(c.contains(&"mkfs.btrfs --force --label bunny /dev/mapper/bunny-root".to_string()));
        for sv in ["@", "@home", "@log", "@cache", "@build", "@snapshots"] {
            assert!(
                c.contains(&format!("btrfs subvolume create /mnt/bunny/{sv}")),
                "{sv}: {c:?}"
            );
        }
        assert!(!c.iter().any(|x| x.contains("@swap")));
        assert!(c.contains(
            &"mount -o compress=zstd:1,noatime,subvol=@ /dev/mapper/bunny-root /mnt/bunny"
                .to_string()
        ));
        assert!(c.contains(
            &"mount -o compress=zstd:1,noatime,subvol=@build /dev/mapper/bunny-root /mnt/bunny/var/tmp/portage"
                .to_string()
        ));
        assert_eq!(
            c.last().unwrap(),
            "mount -o umask=0077 /dev/nvme0n1p1 /mnt/bunny/boot"
        );

        // The partition table is shown; the password never is.
        assert_eq!(r.inputs[0], (nvme().sfdisk_script(), false));
        assert!(r.inputs[1..]
            .iter()
            .all(|(i, secret)| i == "secret pw" && *secret));
    }

    #[test]
    fn swap_home_disk_and_no_encryption() {
        let choices = Choices {
            disk: "/dev/sda".into(),
            encrypt: false,
            swap_mib: 4096,
            home_disk: Some("/dev/sdb".into()),
            ..Default::default()
        };
        let mut r = Recorder::default();
        prepare(&choices, "pw", Path::new("/run/k"), &mut r).unwrap();
        let c = &r.commands;
        assert!(!c.iter().any(|x| x.starts_with("cryptsetup")), "{c:?}");
        // @home is made on the second disk, after it's formatted.
        let home_mkfs = c
            .iter()
            .position(|x| x == "mkfs.btrfs --force --label bunny-home /dev/sdb1")
            .unwrap();
        let home_sv = c
            .iter()
            .position(|x| x == "btrfs subvolume create /mnt/bunny/@home")
            .unwrap();
        assert!(home_sv > home_mkfs, "{c:?}");
        assert!(c.contains(&"btrfs subvolume create /mnt/bunny/@swap".to_string()));
        assert!(c.contains(
            &"mount -o compress=zstd:1,noatime,subvol=@home /dev/sdb1 /mnt/bunny/home".to_string()
        ));
        assert!(c.contains(
            &"btrfs filesystem mkswapfile --size 4096m /mnt/bunny/swap/swapfile".to_string()
        ));
    }

    #[test]
    fn encrypted_home_disk_unlocks_with_a_key_file() {
        let choices = Choices {
            disk: "/dev/sda".into(),
            home_disk: Some("/dev/sdb".into()),
            ..Default::default()
        };
        let mut r = Recorder::default();
        prepare(&choices, "pw", Path::new("/run/bunny-home.key"), &mut r).unwrap();
        assert!(r.commands.contains(
            &"cryptsetup luksAddKey --batch-mode --key-file=- /dev/sdb1 /run/bunny-home.key"
                .to_string()
        ));
        assert!(r.commands.contains(
            &"cryptsetup open --key-file /run/bunny-home.key /dev/sdb1 bunny-home".to_string()
        ));
        let u = read_uuids(&choices, &mut r).unwrap();
        assert_eq!(u.home_luks.as_deref(), Some("uuid-of-/dev/sdb1"));
        let conf = choices.dmcrypt_conf(&u).unwrap();
        assert!(conf.contains("target=bunny-home"), "{conf}");
        assert!(conf.contains("source=UUID=uuid-of-/dev/sdb1"), "{conf}");
        assert!(conf.contains("key=/etc/bunny/keys/home.key"), "{conf}");
    }

    #[test]
    fn fstab_and_command_line() {
        let choices = Choices {
            swap_mib: 8192,
            hibernate: true,
            ..nvme()
        };
        let u = Uuids {
            efi: "ABCD-1234".into(),
            root_fs: "fs-uuid".into(),
            root_luks: Some("luks-uuid".into()),
            ..Default::default()
        };
        let fstab = choices.fstab(&u);
        assert!(
            fstab.contains("UUID=fs-uuid  /  btrfs  compress=zstd:1,noatime,subvol=@  0 0"),
            "{fstab}"
        );
        assert!(
            fstab.contains("UUID=fs-uuid  /swap  btrfs  compress=zstd:1,noatime,subvol=@swap  0 0")
        );
        assert!(fstab.contains("UUID=ABCD-1234  /boot  vfat  umask=0077  0 2"));
        assert!(fstab.contains("/swap/swapfile  none  swap  defaults  0 0"));
        assert_eq!(
            choices.cmdline(&u, Some("12345")),
            "rd.luks.uuid=luks-uuid rootflags=subvol=@ resume=UUID=fs-uuid resume_offset=12345"
        );
        assert_eq!(
            nvme().cmdline(&u, None),
            "rd.luks.uuid=luks-uuid rootflags=subvol=@"
        );
    }

    #[test]
    fn sizes_people_type() {
        assert_eq!(parse_size("2G"), Some(2048));
        assert_eq!(parse_size("2GiB"), Some(2048));
        assert_eq!(parse_size("512M"), Some(512));
        assert_eq!(parse_size("1.5g"), Some(1536));
        assert_eq!(parse_size("8gb"), Some(8192));
        assert_eq!(parse_size("lots"), None);
        assert_eq!(parse_size("12"), None);
        assert_eq!(mib(2048), "2 GB");
        assert_eq!(mib(1536), "1.5 GB");
        assert_eq!(mib(512), "512 MB");
    }

    #[test]
    fn summary_says_what_you_get() {
        let text = nvme().summary().join("\n");
        assert!(
            text.contains("/dev/nvme0n1p1  2 GB  EFI partition, mounted at /boot"),
            "{text}"
        );
        assert!(text.contains("encrypted (LUKS2)"), "{text}");
        assert!(
            text.contains("@ @home @log @cache @build @snapshots"),
            "{text}"
        );
    }
}
