//! Generations and rollback.
//!
//! Each `bunny switch` leaves a read-only snapshot in `/.snapshots/bunny-<time>`.
//! Rolling back replaces the root subvolume `@` with a writable copy of one of
//! them. The current system is snapshotted first, so a rollback can itself be
//! undone. The swap happens on the btrfs top level and takes effect on reboot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::apply::Runner;
use crate::notes::{self, Kind, Note, State};

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
            // Snapshots are folders; notes next to them are files.
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
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
    /// Nested subvolumes (like /srv) carried over to the restored system.
    pub carried: Vec<String>,
    /// Nested subvolumes that couldn't be carried over (left in `replaced`).
    pub left_behind: Vec<String>,
    /// The config put back as it was at that generation, if any.
    pub config_restored: Option<PathBuf>,
    /// The restored system's old "undo the last change" entry was removed.
    pub undo_entry_dropped: bool,
}

/// Parses `btrfs subvolume list` output into paths relative to the top level.
pub fn parse_subvolume_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_once(" path ").map(|(_, p)| p.trim()))
        .map(|p| p.strip_prefix("<FS_TREE>/").unwrap_or(p).to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Subvolumes nested anywhere inside `subvol` (a path relative to the top
/// level, mounted at `top`), as top-level-relative paths, deepest first.
pub fn nested_subvolumes(runner: &mut dyn Runner, top: &Path, subvol: &str) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let mut queue = vec![subvol.to_string()];
    while let Some(parent) = queue.pop() {
        let cmd: Vec<String> = ["btrfs", "subvolume", "list", "-o"]
            .iter()
            .map(|s| s.to_string())
            .chain([top.join(&parent).to_string_lossy().into_owned()])
            .collect();
        let children = runner
            .output(&cmd)?
            .map(|out| parse_subvolume_list(&out))
            .unwrap_or_default();
        for child in children {
            if child.starts_with(&format!("{parent}/")) {
                queue.push(child.clone());
                found.push(child);
            }
        }
    }
    found.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));
    Ok(found)
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
    // btrfs's own "Create snapshot of …" lines aren't shown; on failure its
    // words are, so the error still makes sense.
    let run = |runner: &mut dyn Runner, parts: &[&str]| -> Result<bool> {
        let cmd: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
        let (ok, said) = runner.run_quiet(&cmd, &mut |_| {})?;
        if !ok && !said.trim().is_empty() {
            eprintln!("  {} said:\n{}", parts[0], crate::emerge::tail(&said, 6));
        }
        Ok(ok)
    };

    let top_str = top.to_string_lossy().into_owned();
    if !run(runner, &["mount", "-o", "subvolid=5", &device, &top_str])? {
        bail!("Couldn't open the btrfs volume, so nothing was changed.");
    }

    let before = format!("bunny-{stamp}-before-rollback");
    let replaced = format!("{ROOT_SUBVOL}replaced-{stamp}");
    // Subvolumes inside @ (like /srv) aren't part of snapshots. Carry them
    // over to the restored system afterwards, like /home, so nothing is lost.
    let nested = nested_subvolumes(runner, &top, ROOT_SUBVOL)?;
    let mut carried = Vec::new();
    let mut left_behind = Vec::new();
    let mut config_restored = None;
    let mut undo_entry_dropped = false;
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
        // Shallowest first, so parents are in place before their children.
        for path in nested.iter().rev() {
            let inside = path
                .strip_prefix(&format!("{ROOT_SUBVOL}/"))
                .unwrap_or(path);
            let from = t(&format!("{replaced}/{inside}"));
            let to = t(path);
            // The snapshot has an empty folder where the subvolume was.
            let _ = run(runner, &["rmdir", &to])?;
            if run(runner, &["mv", &from, &to])? {
                carried.push(format!("/{inside}"));
            } else {
                left_behind.push(format!("/{inside}"));
            }
        }
        // The config that matched this generation, back in its place. A
        // failure here doesn't undo the rollback; the system is what matters.
        match crate::applied::restore_into(&top.join(ROOT_SUBVOL)) {
            Ok(path) => config_restored = path,
            Err(e) => eprintln!("  Note: couldn't put the config back as it was ({e:#})."),
        }
        match crate::boot::drop_entry_in(&top.join(ROOT_SUBVOL)) {
            Ok(dropped) => undo_entry_dropped = dropped,
            Err(e) => eprintln!("  Note: couldn't tidy the boot menu ({e:#})."),
        }
        Ok(())
    })();

    run(runner, &["umount", &top_str])?;
    result?;

    notes::write_note(
        root,
        &before,
        &Note {
            kind: Kind::Rollback,
            summary: String::new(),
            to: Some(generation.to_string()),
        },
    )?;
    notes::write_state(
        root,
        &State {
            last: Kind::Rollback,
            at: stamp.to_string(),
            to: Some(generation.to_string()),
        },
    )?;
    Ok(Outcome {
        before,
        replaced,
        carried,
        left_behind,
        config_restored,
        undo_entry_dropped,
    })
}

/// What `bunny clean` would remove.
#[derive(Debug, Default, PartialEq)]
pub struct CleanPlan {
    /// Old generations (folders in /.snapshots).
    pub generations: Vec<String>,
    /// Old root subvolumes left behind by rollbacks.
    pub replaced: Vec<String>,
    /// Subvolumes nested inside each old root, deleted first (deepest first).
    pub nested: BTreeMap<String, Vec<String>>,
}

impl CleanPlan {
    pub fn is_empty(&self) -> bool {
        self.generations.is_empty() && self.replaced.is_empty()
    }
}

/// Keeps the newest `keep` generations. Old root subvolumes are removable,
/// except the one `/` is still running from (before the reboot).
/// `running_copy_of` is the generation the current system was restored from;
/// it's always kept, so it's clear what you're running.
pub fn clean_plan(
    generations: &[Generation],
    keep: usize,
    top_level_names: &[String],
    running_subvol: &str,
    running_copy_of: Option<&str>,
) -> CleanPlan {
    let cut = generations.len().saturating_sub(keep);
    let mut replaced: Vec<String> = top_level_names
        .iter()
        .filter(|n| n.starts_with("@replaced-") && n.as_str() != running_subvol)
        .cloned()
        .collect();
    replaced.sort();
    CleanPlan {
        generations: generations[..cut]
            .iter()
            .filter(|g| Some(g.name.as_str()) != running_copy_of)
            .map(|g| g.name.clone())
            .collect(),
        replaced,
        nested: BTreeMap::new(),
    }
}

/// Deletes what `plan` lists. `top` is where the btrfs top level is mounted.
/// Returns the names that couldn't be deleted.
pub fn clean(
    root: &Path,
    top: &Path,
    plan: &CleanPlan,
    runner: &mut dyn Runner,
) -> Result<Vec<String>> {
    let mut failed = Vec::new();
    let delete = |runner: &mut dyn Runner, path: PathBuf| -> Result<bool> {
        let (ok, said) = runner.run_quiet(
            &[
                "btrfs".to_string(),
                "subvolume".to_string(),
                "delete".to_string(),
                path.to_string_lossy().into_owned(),
            ],
            &mut |_| {},
        )?;
        if !ok && !said.trim().is_empty() {
            eprintln!("  btrfs said:\n{}", crate::emerge::tail(&said, 6));
        }
        Ok(ok)
    };
    for name in &plan.generations {
        if delete(runner, top.join(SNAPSHOTS_SUBVOL).join(name))? {
            let _ = std::fs::remove_file(notes::note_path(root, name));
        } else {
            failed.push(name.clone());
        }
    }
    for name in &plan.replaced {
        let mut nested_ok = true;
        for inner in plan.nested.get(name).into_iter().flatten() {
            if !delete(runner, top.join(inner))? {
                nested_ok = false;
                failed.push(inner.clone());
            }
        }
        if !nested_ok || !delete(runner, top.join(name))? {
            failed.push(name.clone());
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        commands: Vec<String>,
        fail: Option<&'static str>,
        /// Canned output for `btrfs subvolume list -o <path ending>`.
        lists: Vec<(&'static str, &'static str)>,
    }

    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            let line = cmd.join(" ");
            let ok = self.fail.is_none_or(|f| !line.contains(f));
            self.commands.push(line);
            Ok(ok)
        }

        fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
            let path = cmd.last().unwrap();
            Ok(Some(
                self.lists
                    .iter()
                    .find(|(end, _)| path.ends_with(end))
                    .map(|(_, out)| out.to_string())
                    .unwrap_or_default(),
            ))
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
    fn notes_and_files_are_not_generations() {
        let root = system_with(&["bunny-a"]);
        std::fs::write(root.path().join(".snapshots/bunny-a.toml"), "").unwrap();
        std::fs::write(root.path().join(".snapshots/bunny-state.toml"), "").unwrap();
        assert_eq!(list(root.path()).unwrap().len(), 1);
    }

    #[test]
    fn rollback_leaves_notes() {
        let root = system_with(&["bunny-a"]);
        rollback(
            root.path(),
            "bunny-a",
            MOUNTINFO,
            &mut Recorder::default(),
            "X",
        )
        .unwrap();
        let note = notes::read_note(root.path(), "bunny-X-before-rollback").unwrap();
        assert_eq!(note.to.as_deref(), Some("bunny-a"));
        let state = notes::read_state(root.path()).unwrap();
        assert_eq!(state.last, Kind::Rollback);
        assert_eq!(state.to.as_deref(), Some("bunny-a"));
    }

    #[test]
    fn plans_cleanup() {
        let gens: Vec<_> = ["bunny-1", "bunny-2", "bunny-3"]
            .iter()
            .map(|n| Generation {
                name: n.to_string(),
            })
            .collect();
        let top = ["@", "@home", "@replaced-b", "@replaced-a", "@snapshots"].map(String::from);
        let plan = clean_plan(&gens, 2, &top, "@", None);
        assert_eq!(plan.generations, ["bunny-1"]);
        assert_eq!(plan.replaced, ["@replaced-a", "@replaced-b"]);

        // Not rebooted after a rollback yet: / still runs from @replaced-b.
        let plan = clean_plan(&gens, 5, &top, "@replaced-b", None);
        assert!(plan.generations.is_empty());
        assert_eq!(plan.replaced, ["@replaced-a"]);
    }

    #[test]
    fn cleans() {
        let root = system_with(&["bunny-1"]);
        std::fs::write(
            notes::note_path(root.path(), "bunny-1"),
            "kind = \"switch\"",
        )
        .unwrap();
        let plan = CleanPlan {
            generations: vec!["bunny-1".into()],
            replaced: vec!["@replaced-a".into()],
            nested: BTreeMap::from([("@replaced-a".into(), vec!["@replaced-a/srv".into()])]),
        };
        let mut runner = Recorder::default();
        let failed = clean(root.path(), Path::new("/top"), &plan, &mut runner).unwrap();
        assert!(failed.is_empty());
        assert_eq!(
            runner.commands,
            [
                "btrfs subvolume delete /top/@snapshots/bunny-1",
                "btrfs subvolume delete /top/@replaced-a/srv",
                "btrfs subvolume delete /top/@replaced-a"
            ]
        );
        assert!(!notes::note_path(root.path(), "bunny-1").exists());
    }

    #[test]
    fn keeps_the_generation_you_are_running_a_copy_of() {
        let gens: Vec<_> = ["bunny-1", "bunny-2", "bunny-3"]
            .iter()
            .map(|n| Generation {
                name: n.to_string(),
            })
            .collect();
        let plan = clean_plan(&gens, 1, &[], "@", Some("bunny-1"));
        assert_eq!(plan.generations, ["bunny-2"]);
    }

    #[test]
    fn parses_subvolume_lists() {
        let out = "ID 262 gen 85 top level 256 path <FS_TREE>/@replaced-X/srv\n\
                   ID 270 gen 9 top level 256 path @/var/lib/machines\n";
        assert_eq!(
            parse_subvolume_list(out),
            ["@replaced-X/srv", "@/var/lib/machines"]
        );
    }

    #[test]
    fn finds_nested_subvolumes_deepest_first() {
        let mut runner = Recorder {
            lists: vec![
                (
                    "top/@",
                    "ID 1 gen 1 top level 5 path @/srv\nID 2 gen 1 top level 5 path @/opt\n",
                ),
                ("top/@/srv", "ID 3 gen 1 top level 1 path @/srv/data\n"),
            ],
            ..Default::default()
        };
        let nested = nested_subvolumes(&mut runner, Path::new("/top"), "@").unwrap();
        assert_eq!(nested[0], "@/srv/data");
        assert_eq!(nested.len(), 3);
    }

    #[test]
    fn rollback_carries_nested_subvolumes_over() {
        let root = system_with(&["bunny-a"]);
        let mut runner = Recorder {
            lists: vec![("top/@", "ID 262 gen 85 top level 265 path @/srv\n")],
            ..Default::default()
        };
        let out = rollback(root.path(), "bunny-a", MOUNTINFO, &mut runner, "X").unwrap();
        let top = root
            .path()
            .join("run/bunny/top")
            .to_string_lossy()
            .into_owned();
        let restore = runner
            .commands
            .iter()
            .position(|c| c.contains("snapshots/bunny-a"))
            .unwrap();
        assert_eq!(runner.commands[restore + 1], format!("rmdir {top}/@/srv"));
        assert_eq!(
            runner.commands[restore + 2],
            format!("mv {top}/@replaced-X/srv {top}/@/srv")
        );
        assert_eq!(out.carried, ["/srv"]);
        assert!(out.left_behind.is_empty());
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
