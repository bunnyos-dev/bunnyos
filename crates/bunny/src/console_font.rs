//! A console font that can draw ✓ ✗ ⚠, for systems that didn't get it
//! from `bunny install`.
//!
//! The Linux console's usual font has no ✓ or ✗ (they show as `v` and `x`).
//! kbd's `eurlatgr` (on every Gentoo system) has ✓ ✗ · but no ⚠, so bunny
//! copies it to `/usr/local/share/consolefonts/bunny.psfu` with ⚠ drawn by
//! its `!`, sets it in `/etc/conf.d/consolefont` and adds the `consolefont`
//! service to the boot runlevel. The same font file and settings as
//! `bunny install` and the live USB (`install/inside.sh`,
//! `tools/iso/inside.sh`).
//!
//! A font you chose yourself in `/etc/conf.d/consolefont` is left alone.
//! The settings file is backed up first; the font file and the runlevel
//! link are in `/`, so undoing the switch takes them away again.

use std::path::Path;

use anyhow::{Context, Result};

use crate::apply::services::ServiceCommands;
use crate::apply::Runner;

/// kbd's font, compressed (Gentoo) or not.
const SOURCES: [&str; 2] = [
    "usr/share/consolefonts/eurlatgr.psfu.gz",
    "usr/share/consolefonts/eurlatgr.psfu",
];
/// bunny's copy, with ⚠ added.
pub const FONT: &str = "usr/local/share/consolefonts/bunny.psfu";
pub const SETTINGS: &str = "etc/conf.d/consolefont";
const SERVICE: &str = "etc/init.d/consolefont";
const IN_BOOT: &str = "etc/runlevels/boot/consolefont";
/// The fonts Gentoo sets out of the box: changing these isn't overriding a
/// choice you made.
const STOCK: [&str; 2] = ["", "default8x16"];

/// One part of setting the font up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Make bunny's copy of the font.
    Font,
    /// Point `/etc/conf.d/consolefont` at it.
    Setting,
    /// Add `consolefont` to the boot runlevel.
    AtBoot,
}

impl Step {
    pub fn describe(&self) -> String {
        match self {
            Step::Font => format!("make a copy of kbd's eurlatgr font that also draws ⚠ (/{FONT})"),
            Step::Setting => format!("use it on the console (/{SETTINGS})"),
            Step::AtBoot => "load it at every start (consolefont in the boot runlevel)".into(),
        }
    }
}

/// The `consolefont=` setting in `/etc/conf.d/consolefont`, if one is set
/// (the last uncommented line wins, as in the shell).
fn current_setting(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("consolefont="))
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .next_back()
}

/// What's left to set up at `root`. Nothing when kbd's font or OpenRC's
/// `consolefont` service is missing, or when you chose another font.
pub fn needed(root: &Path) -> Vec<Step> {
    let ours = format!("/{FONT}");
    if !root.join(SERVICE).is_file() {
        return Vec::new();
    }
    let have_font = root.join(FONT).is_file();
    if !have_font && source(root).is_none() {
        return Vec::new();
    }
    let setting = std::fs::read_to_string(root.join(SETTINGS))
        .ok()
        .and_then(|t| current_setting(&t))
        .unwrap_or_default();
    if setting != ours && !STOCK.contains(&setting.as_str()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if !have_font {
        out.push(Step::Font);
    }
    if setting != ours {
        out.push(Step::Setting);
    }
    if std::fs::symlink_metadata(root.join(IN_BOOT)).is_err() {
        out.push(Step::AtBoot);
    }
    out
}

fn source(root: &Path) -> Option<std::path::PathBuf> {
    SOURCES.iter().map(|s| root.join(s)).find(|p| p.is_file())
}

/// `/etc/conf.d/consolefont` pointing at `font`: the `consolefont=` line
/// (commented out or not) replaced, or one added.
pub fn with_setting(text: &str, font: &str) -> String {
    let line = format!("consolefont=\"{font}\"");
    let mut out = String::new();
    let mut done = false;
    for l in text.lines() {
        let bare = l.trim_start().trim_start_matches('#');
        if bare.starts_with("consolefont=") {
            if !done {
                out.push_str(&line);
                out.push('\n');
                done = true;
            }
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    if !done {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The command that makes bunny's copy of the font from kbd's.
fn make_font_command(src: &Path, font: &Path) -> Vec<String> {
    const SCRIPT: &str = "set -e; mkdir -p \"${2%/*}\"; t=$(mktemp -d); \
        zcat -f \"$1\" > \"$t/f.psfu\"; psfgettable \"$t/f.psfu\" \"$t/f.txt\"; \
        printf '0x021\\tU+26A0\\n' >> \"$t/f.txt\"; \
        psfaddtable \"$t/f.psfu\" \"$t/f.txt\" \"$2\"; rm -rf \"$t\"";
    vec![
        "sh".into(),
        "-c".into(),
        SCRIPT.into(),
        "sh".into(),
        src.to_string_lossy().into_owned(),
        font.to_string_lossy().into_owned(),
    ]
}

/// Sets the font up as `steps` says, backing up the settings file into
/// `backup` first. Returns notes for anything that didn't work: the font
/// is a nicety, never a reason to stop a switch.
pub fn apply(
    root: &Path,
    steps: &[Step],
    backup: &Path,
    runner: &mut dyn Runner,
    services: &mut ServiceCommands,
) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    if steps.is_empty() {
        return Ok(notes);
    }
    if steps.contains(&Step::Font) {
        let made = match source(root) {
            Some(src) => runner.run(&make_font_command(&src, &root.join(FONT)))?,
            None => false,
        };
        if !made {
            notes.push(
                "Couldn't make the console font, so the console keeps its usual one \
                 (✓ and ✗ show as v and x there)."
                    .into(),
            );
            return Ok(notes);
        }
    }
    if steps.contains(&Step::Setting) {
        let path = root.join(SETTINGS);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if path.exists() {
            let to = backup.join(SETTINGS);
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("Couldn't create {}", dir.display()))?;
            }
            std::fs::copy(&path, &to)
                .with_context(|| format!("Couldn't back up {}", path.display()))?;
        } else if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Couldn't create {}", dir.display()))?;
        }
        std::fs::write(&path, with_setting(&text, &format!("/{FONT}")))
            .with_context(|| format!("Couldn't write {}", path.display()))?;
    }
    if steps.contains(&Step::AtBoot)
        && !services.run(
            runner,
            &["rc-update", "add", "consolefont", "boot"].map(String::from),
        )?
    {
        notes.push(
            "Couldn't add consolefont to the boot runlevel, so the console font isn't \
             loaded at start."
                .into(),
        );
    }
    // Load it now, so the console has it before the next restart.
    if !services.run(
        runner,
        &["rc-service", "consolefont", "restart"].map(String::from),
    )? {
        notes.push("The console font is set up, and is used from the next restart.".into());
    } else if notes.is_empty() && !services.verbose && !runner.dry() {
        println!("  ✓ The console now has a font that draws ✓ ✗ ⚠, at every start too.");
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Gentoo system as it comes: kbd's fonts, OpenRC's consolefont
    /// service and its stock setting.
    pub(crate) fn gentoo_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        for d in [
            "usr/share/consolefonts",
            "etc/init.d",
            "etc/conf.d",
            "etc/runlevels/boot",
        ] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        std::fs::write(r.join(SOURCES[0]), "font").unwrap();
        std::fs::write(r.join(SERVICE), "#!/sbin/openrc-run\n").unwrap();
        std::fs::write(
            r.join(SETTINGS),
            "# The consolefont to load.\nconsolefont=\"default8x16\"\n\n# The unicode map.\nunicodemap=\"\"\n",
        )
        .unwrap();
        dir
    }

    #[derive(Default)]
    struct Recorder(Vec<String>);
    impl Runner for Recorder {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.0.push(cmd.join(" "));
            Ok(true)
        }
    }

    #[test]
    fn an_existing_system_needs_all_of_it() {
        let root = gentoo_root();
        assert_eq!(
            needed(root.path()),
            [Step::Font, Step::Setting, Step::AtBoot]
        );
    }

    #[test]
    fn sets_it_up_backs_up_and_is_done_once() {
        let root = gentoo_root();
        let r = root.path();
        let backup = r.join("var/lib/bunny/backups/1");
        let mut runner = Recorder::default();
        let notes = apply(
            r,
            &needed(r),
            &backup,
            &mut runner,
            &mut ServiceCommands::new(false),
        )
        .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        // The font, made with kbd's tools from eurlatgr, ⚠ added.
        let make = &runner.0[0];
        assert!(make.starts_with("sh -c set -e;"), "{make}");
        assert!(make.contains("psfaddtable"), "{make}");
        assert!(make.contains("U+26A0"), "{make}");
        assert!(make.ends_with(&format!(
            "{} {}",
            r.join(SOURCES[0]).display(),
            r.join(FONT).display()
        )));
        assert_eq!(runner.0[1], "rc-update add consolefont boot");
        assert_eq!(runner.0[2], "rc-service consolefont restart");
        // The setting points at it; the rest of the file stays.
        let settings = std::fs::read_to_string(r.join(SETTINGS)).unwrap();
        assert!(
            settings.contains(&format!("consolefont=\"/{FONT}\"\n")),
            "{settings}"
        );
        assert!(!settings.contains("default8x16"), "{settings}");
        assert!(settings.contains("unicodemap=\"\""));
        // The original is in the backup.
        assert!(std::fs::read_to_string(backup.join(SETTINGS))
            .unwrap()
            .contains("consolefont=\"default8x16\""));

        // Once the font exists and the service is in boot, nothing's left.
        std::fs::create_dir_all(r.join(FONT).parent().unwrap()).unwrap();
        std::fs::write(r.join(FONT), "font").unwrap();
        std::os::unix::fs::symlink("/etc/init.d/consolefont", r.join(IN_BOOT)).unwrap();
        assert!(needed(r).is_empty());
    }

    #[test]
    fn a_font_you_chose_is_left_alone() {
        let root = gentoo_root();
        std::fs::write(root.path().join(SETTINGS), "consolefont=\"ter-v16n\"\n").unwrap();
        assert!(needed(root.path()).is_empty());
    }

    #[test]
    fn nothing_without_kbds_font_or_the_service() {
        let root = gentoo_root();
        std::fs::remove_file(root.path().join(SOURCES[0])).unwrap();
        assert!(needed(root.path()).is_empty());
        let root = gentoo_root();
        std::fs::remove_file(root.path().join(SERVICE)).unwrap();
        assert!(needed(root.path()).is_empty());
    }

    #[test]
    fn writes_the_setting_once() {
        assert_eq!(
            with_setting("a=1\n#consolefont=\"x\"\nconsolefont=\"y\"\n", "/f"),
            "a=1\nconsolefont=\"/f\"\n"
        );
        assert_eq!(with_setting("a=1\n", "/f"), "a=1\nconsolefont=\"/f\"\n");
        assert_eq!(
            current_setting("consolefont=\"/f\"\n").as_deref(),
            Some("/f")
        );
        assert_eq!(current_setting("#consolefont=\"/f\"\n"), None);
    }

    #[test]
    fn a_font_that_cant_be_made_is_a_note_not_a_failure() {
        struct Fails;
        impl Runner for Fails {
            fn run(&mut self, _: &[String]) -> Result<bool> {
                Ok(false)
            }
        }
        let root = gentoo_root();
        let r = root.path();
        let notes = apply(
            r,
            &needed(r),
            &r.join("b"),
            &mut Fails,
            &mut ServiceCommands::new(false),
        )
        .unwrap();
        assert!(
            notes[0].contains("Couldn't make the console font"),
            "{notes:?}"
        );
        // The setting isn't pointed at a font that doesn't exist.
        assert!(std::fs::read_to_string(r.join(SETTINGS))
            .unwrap()
            .contains("default8x16"));
    }
}
