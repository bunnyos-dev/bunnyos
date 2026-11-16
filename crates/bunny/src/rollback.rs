//! Generations and rollback.
//!
//! Each `bunny switch` leaves a read-only snapshot in `/.snapshots/bunny-<time>`.
//! Rolling back replaces the root subvolume `@` with a writable copy of one of
//! them. The current system is snapshotted first, so a rollback can itself be
//! undone. The swap happens on the btrfs top level and takes effect on reboot.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::apply::Runner;

/// The root subvolume name from the BunnyOS disk layout.
const ROOT_SUBVOL: &str = "@";
/// The subvolume mounted at `/.snapshots`.
const SNAPSHOTS_SUBVOL: &str = "@snapshots";

#[derive(Debug, PartialEq)]
pub struct Generation {
    pub name: String,
}

impl Generation {
    /// `bunny-20261116-204336` → `2026-11-16 20:43:36 UTC`
    pub fn when(&self) -> String {
        let stamp = self
            .name
            .trim_start_matches("bunny-")
            .split('-')
            .take(2)
            .collect::<Vec<_>>();
        match stamp.as_slice() {
            [d, t] if d.len() == 8 && t.len() == 6 => format!(
                "{}-{}-{} {}:{}:{} UTC",
                &d[..4],
                &d[4..6],
                &d[6..],
                &t[..2],
                &t[2..4],
                &t[4..]
            ),
            _ => self.name.clone(),
        }
    }

    pub fn is_pre_rollback(&self) -> bool {
        self.name.ends_with("-before-rollback")
    }
}

/// Snapshots bunny made, oldest first.
pub fn list(root: &Path) -> Result<Vec<Generation>> {
    let dir = root.join(".snapshots");
    let mut generations: Vec<Generation> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("bunny-"))
            .map(|name| Generation { name })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).context("Couldn't read /.snapshots"),
    };
    generations.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(generations)
}

/// Turns a number from `bunny generations` (1 = oldest) into a name.
/// Anything that isn't a valid number is returned as `None`.
pub fn resolve(generations: &[Generation], input: &str) -> Option<String> {
    let n: usize = input.parse().ok()?;
    generations.get(n.checked_sub(1)?).map(|g| g.name.clone())
}

/// Finds the device and subvolume `/` is mounted from, using /proc/self/mountinfo.
pub fn root_mount(mountinfo: &str) -> Result<(String, String)> {
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // 4: root within the filesystem, 5: mount point. After "-": fstype, source.
        if fields.get(4) != Some(&"/") {
            continue;
        }
        let Some(dash) = fields.iter().position(|f| *f == "-") else {
            continue;
        };
        let fstype = fields.get(dash + 1).copied().unwrap_or("");
        let source = fields.get(dash + 2).copied().unwrap_or("");
        if fstype != "btrfs" {
            bail!(
                "Rollback needs / on btrfs, but it's on {fstype}.\n  \
                 bunny's generations only work with the btrfs disk layout."
            );
        }
        let subvol = fields[3].trim_start_matches('/').to_string();
        return Ok((source.to_string(), subvol));
    }
    bail!("Couldn't work out which disk / is mounted from.")
}

#[derive(Debug)]
pub struct Outcome {
    /// Snapshot of the system as it was before rolling back.
    pub before: String,
    /// Where the replaced root subvolume was moved.
    pub replaced: String,
}

/// Replaces `@` with a writable copy of `generation`. Takes effect on reboot.
pub fn rollback(
    root: &Path,
    generation: &str,
    mountinfo: &str,
    runner: &mut dyn Runner,
    stamp: &str,
) -> Result<Outcome> {
    if !list(root)?.iter().any(|g| g.name == generation) {
        bail!(
            "There's no generation called \"{generation}\".\n  \
             Run `bunny generations` to see the ones you have."
        );
    }
    let (device, subvol) = root_mount(mountinfo)?;
    if subvol != ROOT_SUBVOL {
        bail!(
            "/ is mounted from the subvolume \"{subvol}\", not \"{ROOT_SUBVOL}\".\n  \
             If you already rolled back, reboot first, then try again."
        );
    }

    let top: PathBuf = root.join("run/bunny/top");
    std::fs::create_dir_all(&top).context("Couldn't create a temporary mount point")?;
    let t = |p: &str| top.join(p).to_string_lossy().into_owned();
    let run = |runner: &mut dyn Runner, parts: &[&str]| -> Result<bool> {
        runner.run(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };

    let top_str = top.to_string_lossy().into_owned();
    if !run(runner, &["mount", "-o", "subvolid=5", &device, &top_str])? {
        bail!("Couldn't open the btrfs volume, so nothing was changed.");
    }

    let before = format!("bunny-{stamp}-before-rollback");
    let replaced = format!("{ROOT_SUBVOL}replaced-{stamp}");
    let result = (|| -> Result<()> {
        let before_path = t(&format!("{SNAPSHOTS_SUBVOL}/{before}"));
        if !run(
            runner,
            &[
                "btrfs",
                "subvolume",
                "snapshot",
                "-r",
                &t(ROOT_SUBVOL),
                &before_path,
            ],
        )? {
            bail!("Couldn't save the current system first, so nothing was changed.");
        }
        if !run(runner, &["mv", &t(ROOT_SUBVOL), &t(&replaced)])? {
            bail!("Couldn't move the current system aside, so nothing was changed.");
        }
        let source = t(&format!("{SNAPSHOTS_SUBVOL}/{generation}"));
        if !run(
            runner,
            &["btrfs", "subvolume", "snapshot", &source, &t(ROOT_SUBVOL)],
        )? {
            // Put the current system back where it was.
            run(runner, &["mv", &t(&replaced), &t(ROOT_SUBVOL)])?;
            bail!("Couldn't restore \"{generation}\". Your current system was put back unchanged.");
        }
        Ok(())
    })();

    run(runner, &["umount", &top_str])?;
    result?;
    Ok(Outcome { before, replaced })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        commands: Vec<String>,
        fail: Option<&'static str>,
    }

    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            let line = cmd.join(" ");
            let ok = self.fail.is_none_or(|f| !line.contains(f));
            self.commands.push(line);
            Ok(ok)
        }
    }

    const MOUNTINFO: &str = "\
22 1 0:21 /@ / rw,noatime shared:1 - btrfs /dev/vda2 rw,compress=zstd:1,subvolid=256,subvol=/@
23 22 0:21 /@home /home rw,noatime shared:2 - btrfs /dev/vda2 rw,subvol=/@home
24 22 259:1 / /efi rw,relatime shared:3 - vfat /dev/vda1 rw
";

    fn system_with(gens: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for g in gens {
            std::fs::create_dir_all(dir.path().join(".snapshots").join(g)).unwrap();
        }
        dir
    }

    #[test]
    fn lists_generations_oldest_first() {
        let root = system_with(&["bunny-20261116-204336", "bunny-20261101-090000", "other"]);
        let gens = list(root.path()).unwrap();
        let names: Vec<_> = gens.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["bunny-20261101-090000", "bunny-20261116-204336"]);
        assert_eq!(gens[1].when(), "2026-11-16 20:43:36 UTC");
    }

    #[test]
    fn resolves_numbers() {
        let gens: Vec<_> = ["bunny-a", "bunny-b"]
            .iter()
            .map(|n| Generation {
                name: n.to_string(),
            })
            .collect();
        assert_eq!(resolve(&gens, "1").as_deref(), Some("bunny-a"));
        assert_eq!(resolve(&gens, "2").as_deref(), Some("bunny-b"));
        assert_eq!(resolve(&gens, "0"), None);
        assert_eq!(resolve(&gens, "3"), None);
        assert_eq!(resolve(&gens, "bunny-a"), None);
    }

    #[test]
    fn finds_the_root_mount() {
        assert_eq!(
            root_mount(MOUNTINFO).unwrap(),
            ("/dev/vda2".to_string(), "@".to_string())
        );
        let ext4 = "22 1 8:2 / / rw - ext4 /dev/sda2 rw\n";
        assert!(root_mount(ext4).unwrap_err().to_string().contains("btrfs"));
    }

    #[test]
    fn rolls_back() {
        let root = system_with(&["bunny-20261116-204336"]);
        let mut runner = Recorder::default();
        let out = rollback(
            root.path(),
            "bunny-20261116-204336",
            MOUNTINFO,
            &mut runner,
            "X",
        )
        .unwrap();
        let top = root
            .path()
            .join("run/bunny/top")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            runner.commands,
            [
                format!("mount -o subvolid=5 /dev/vda2 {top}"),
                format!(
                    "btrfs subvolume snapshot -r {top}/@ {top}/@snapshots/bunny-X-before-rollback"
                ),
                format!("mv {top}/@ {top}/@replaced-X"),
                format!("btrfs subvolume snapshot {top}/@snapshots/bunny-20261116-204336 {top}/@"),
                format!("umount {top}"),
            ]
        );
        assert_eq!(out.before, "bunny-X-before-rollback");
    }

    #[test]
    fn failed_restore_puts_the_system_back() {
        let root = system_with(&["bunny-a"]);
        let mut runner = Recorder {
            fail: Some("bunny-a"),
            ..Default::default()
        };
        let err = rollback(root.path(), "bunny-a", MOUNTINFO, &mut runner, "X").unwrap_err();
        assert!(err.to_string().contains("put back unchanged"));
        let top = root
            .path()
            .join("run/bunny/top")
            .to_string_lossy()
            .into_owned();
        assert!(runner
            .commands
            .contains(&format!("mv {top}/@replaced-X {top}/@")));
        assert_eq!(runner.commands.last().unwrap(), &format!("umount {top}"));
    }

    #[test]
    fn refuses_unknown_generation_and_rollback_twice() {
        let root = system_with(&["bunny-a"]);
        let mut runner = Recorder::default();
        let err = rollback(root.path(), "bunny-nope", MOUNTINFO, &mut runner, "X").unwrap_err();
        assert!(err.to_string().contains("no generation"));

        let rolled = MOUNTINFO.replace("/@ / ", "/@replaced-X / ");
        let err = rollback(root.path(), "bunny-a", &rolled, &mut runner, "X").unwrap_err();
        assert!(err.to_string().contains("reboot first"));
        assert!(runner.commands.is_empty());
    }
}
