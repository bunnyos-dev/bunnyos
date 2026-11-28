//! `[system] profile`: which Gentoo profile the system uses.
//!
//! A Gentoo profile is a set of defaults, most importantly which USE flags
//! are on. Bunny Linux installs the plain one ("minimal": almost nothing a
//! desktop needs is on) and offers Gentoo's desktop profile ("desktop":
//! Wayland, X, sound, D-Bus and so on are on). Gentoo builds its prebuilt
//! packages for both, so either one downloads; the desktop one just needs
//! far fewer settings for a desktop. bunny's resolver makes a desktop work
//! from either.
//!
//! bunny only switches between those two, keeping everything else about the
//! profile (architecture, release). A system on another profile (hardened,
//! plasma, systemd, …) is left as it is, and bunny says so.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// The profiles bunny switches between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Minimal,
    Desktop,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::Minimal => "minimal",
            Profile::Desktop => "desktop",
        }
    }
}

/// Portage's link to the profile, under the system root.
pub const LINK: &str = "etc/portage/make.profile";

/// Where the old link target is kept in a switch's backup folder.
pub const BACKUP: &str = "make.profile";

/// A change of profile: the link's target before and after.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub from: Profile,
    pub to: Profile,
    /// The link's new target, written like the old one (relative or not).
    pub target: String,
}

/// What `profile` in the config means for this system.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Wanted {
    /// Nothing to do: not in the config, or already that profile.
    #[default]
    Nothing,
    /// Switch profiles.
    Change(Change),
    /// The system's profile isn't one bunny switches from; why, in words.
    Unsupported(String),
}

/// The part of a profile link's target after `profiles/`, e.g.
/// `default/linux/amd64/23.0/desktop`.
fn profile_part(target: &str) -> Option<&str> {
    target
        .split_once("/profiles/")
        .map(|(_, p)| p.trim_end_matches('/'))
}

/// Which of bunny's profiles a profile path is, and its plain base
/// (`default/linux/amd64/23.0`). `None` for any other profile.
pub fn classify(part: &str) -> Option<(Profile, &str)> {
    if let Some(base) = part.strip_suffix("/desktop") {
        if is_release(base) {
            return Some((Profile::Desktop, base));
        }
        return None;
    }
    is_release(part).then_some((Profile::Minimal, part))
}

/// `default/linux/<arch>/<release>`, where the release is a version like
/// `23.0`.
fn is_release(base: &str) -> bool {
    let parts: Vec<&str> = base.split('/').collect();
    matches!(parts.as_slice(), ["default", "linux", _, release]
        if release.split('.').all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
}

/// The profile link's target as written (relative, usually).
fn read_link(root: &Path) -> Option<String> {
    std::fs::read_link(root.join(LINK))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// The system's profile, if it's one of bunny's.
pub fn current(root: &Path) -> Option<Profile> {
    let target = read_link(root)?;
    classify(profile_part(&target)?).map(|(p, _)| p)
}

/// What to do for `wanted` (from the config) on the system at `root`.
pub fn wanted(root: &Path, wanted: Option<Profile>) -> Wanted {
    let Some(wanted) = wanted else {
        return Wanted::Nothing;
    };
    let Some(target) = read_link(root) else {
        return Wanted::Unsupported(format!(
            "{} isn't a link to a Gentoo profile, so bunny can't tell which one this \
             system uses.",
            Path::new("/").join(LINK).display()
        ));
    };
    let Some(part) = profile_part(&target) else {
        return Wanted::Unsupported(format!(
            "/{LINK} points at {target}, which isn't in a profiles folder."
        ));
    };
    let Some((now, base)) = classify(part) else {
        return Wanted::Unsupported(format!(
            "This system uses the {part} profile. bunny only switches between \
             Gentoo's plain profile (\"minimal\") and its desktop profile (\"desktop\"), \
             so it leaves this one as it is. Remove `profile` from [system] to stop \
             this note, or switch by hand with `eselect profile`."
        ));
    };
    if now == wanted {
        return Wanted::Nothing;
    }
    let new_part = match wanted {
        Profile::Minimal => base.to_string(),
        Profile::Desktop => format!("{base}/desktop"),
    };
    let (head, _) = target
        .split_once("/profiles/")
        .unwrap_or((target.as_str(), ""));
    Wanted::Change(Change {
        from: now,
        to: wanted,
        target: format!("{head}/profiles/{new_part}"),
    })
}

/// Points the profile link at the new profile, keeping the old target in
/// `backup` so a cancelled switch can put it back.
pub fn apply(root: &Path, change: &Change, backup: &Path) -> Result<()> {
    let link = root.join(LINK);
    if let Some(old) = read_link(root) {
        std::fs::write(backup.join(BACKUP), &old)
            .context("Couldn't back up which profile the system used")?;
    }
    let staged: PathBuf = {
        let mut s = link.clone().into_os_string();
        s.push(".bunny-new");
        s.into()
    };
    let _ = std::fs::remove_file(&staged);
    std::os::unix::fs::symlink(&change.target, &staged)
        .with_context(|| format!("Couldn't write {}", staged.display()))?;
    std::fs::rename(&staged, &link).with_context(|| format!("Couldn't switch {}", link.display()))
}

/// Puts the profile link back from a switch's backup, if it changed.
pub fn restore(root: &Path, backup: &Path) -> Result<()> {
    let Ok(old) = std::fs::read_to_string(backup.join(BACKUP)) else {
        return Ok(());
    };
    let link = root.join(LINK);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(old.trim(), &link).context("Couldn't put the profile back")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(target: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc/portage")).unwrap();
        std::os::unix::fs::symlink(target, root.path().join(LINK)).unwrap();
        root
    }

    const PLAIN: &str = "../../var/db/repos/gentoo/profiles/default/linux/amd64/23.0";

    #[test]
    fn tells_the_two_profiles_apart() {
        assert_eq!(
            classify("default/linux/amd64/23.0"),
            Some((Profile::Minimal, "default/linux/amd64/23.0"))
        );
        assert_eq!(
            classify("default/linux/amd64/23.0/desktop"),
            Some((Profile::Desktop, "default/linux/amd64/23.0"))
        );
        for other in [
            "default/linux/amd64/23.0/desktop/plasma",
            "default/linux/amd64/23.0/hardened",
            "default/linux/amd64/23.0/systemd",
            "default/linux/amd64/23.0/no-multilib",
        ] {
            assert_eq!(classify(other), None, "{other}");
        }
    }

    #[test]
    fn switches_to_desktop_keeping_the_link_style() {
        let root = system(PLAIN);
        assert_eq!(current(root.path()), Some(Profile::Minimal));
        let Wanted::Change(change) = wanted(root.path(), Some(Profile::Desktop)) else {
            panic!("expected a change");
        };
        assert_eq!(change.from, Profile::Minimal);
        assert_eq!(change.target, format!("{PLAIN}/desktop"));
        let backup = tempfile::tempdir().unwrap();
        apply(root.path(), &change, backup.path()).unwrap();
        assert_eq!(current(root.path()), Some(Profile::Desktop));
        assert_eq!(wanted(root.path(), Some(Profile::Desktop)), Wanted::Nothing);
        // Cancelled: back to how it was.
        restore(root.path(), backup.path()).unwrap();
        assert_eq!(
            std::fs::read_link(root.path().join(LINK)).unwrap(),
            Path::new(PLAIN)
        );
    }

    #[test]
    fn back_to_minimal() {
        let root = system(&format!("{PLAIN}/desktop"));
        let Wanted::Change(change) = wanted(root.path(), Some(Profile::Minimal)) else {
            panic!("expected a change");
        };
        assert_eq!(change.target, PLAIN);
    }

    #[test]
    fn other_profiles_are_left_alone() {
        let root = system(&format!("{PLAIN}/desktop/plasma"));
        let Wanted::Unsupported(why) = wanted(root.path(), Some(Profile::Desktop)) else {
            panic!("expected unsupported");
        };
        assert!(why.contains("desktop/plasma"), "{why}");
        assert!(why.contains("leaves this one as it is"), "{why}");
        // Not in the config: nothing to say.
        assert_eq!(wanted(root.path(), None), Wanted::Nothing);
    }
}
