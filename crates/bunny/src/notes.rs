//! Small notes kept next to the snapshots in `/.snapshots`.
//!
//! `/.snapshots` is its own subvolume, so these survive rollbacks:
//! - `bunny-<time>.toml` says what happened right after that snapshot was taken
//!   (which switch or rollback it was saved before).
//! - `bunny-state.toml` remembers the last thing bunny did, so the list can
//!   show what you're running now.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "bunny-state.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Switch,
    Rollback,
}

/// What happened right after a snapshot was taken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub kind: Kind,
    /// For a switch: a short summary of the changes, e.g. "+ tmux, start cronie".
    #[serde(default)]
    pub summary: String,
    /// For a rollback: the generation it went back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// The last thing bunny did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub last: Kind,
    /// Timestamp, like `20261116-210840`.
    pub at: String,
    /// For a rollback: the generation now running (as a copy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

fn snapshots(root: &Path) -> PathBuf {
    root.join(".snapshots")
}

pub fn note_path(root: &Path, generation: &str) -> PathBuf {
    snapshots(root).join(format!("{generation}.toml"))
}

/// Saves a note. Does nothing if there's no `/.snapshots` folder.
pub fn write_note(root: &Path, generation: &str, note: &Note) -> Result<()> {
    if !snapshots(root).is_dir() {
        return Ok(());
    }
    std::fs::write(note_path(root, generation), toml::to_string(note)?)
        .context("Couldn't save the note for this generation")
}

pub fn read_note(root: &Path, generation: &str) -> Option<Note> {
    let text = std::fs::read_to_string(note_path(root, generation)).ok()?;
    toml::from_str(&text).ok()
}

/// Saves the state. Does nothing if there's no `/.snapshots` folder.
pub fn write_state(root: &Path, state: &State) -> Result<()> {
    if !snapshots(root).is_dir() {
        return Ok(());
    }
    std::fs::write(snapshots(root).join(STATE_FILE), toml::to_string(state)?)
        .context("Couldn't save bunny's state")
}

pub fn read_state(root: &Path) -> Option<State> {
    let text = std::fs::read_to_string(snapshots(root).join(STATE_FILE)).ok()?;
    toml::from_str(&text).ok()
}

/// A switch's summary (`+ tmux, + cronie, - tree, start cronie`) as what
/// the version before it was "before": `adding tmux and cronie, removing
/// tree, starting cronie`. For the boot menu's "Earlier versions" and
/// `bunny generations`, where "before + tmux" read oddly. Parts it doesn't
/// know are kept as they are, so older notes still read.
pub fn before_words(summary: &str) -> String {
    // (verb, what) for each part; consecutive parts with the same verb are
    // joined: "adding tmux and cronie".
    let mut parts: Vec<(&str, Vec<String>)> = Vec::new();
    let mut tail = String::new();
    for part in summary.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
        // "… and 3 more" on the last part.
        let (part, more) = match part.rsplit_once(" and ") {
            Some((p, m)) if m.ends_with(" more") => (p, Some(m)),
            _ => (part, None),
        };
        let (verb, what) = if let Some(r) = part.strip_prefix("+ repository ") {
            ("adding the repository", r.to_string())
        } else if let Some(r) = part
            .strip_prefix("- repository ")
            .or_else(|| part.strip_prefix("− repository "))
        {
            ("removing the repository", r.to_string())
        } else if let Some(p) = part.strip_prefix("+ ") {
            ("adding", p.to_string())
        } else if let Some(p) = part.strip_prefix("- ").or_else(|| part.strip_prefix("− ")) {
            ("removing", p.to_string())
        } else if let Some(s) = part.strip_prefix("start ") {
            ("starting", s.to_string())
        } else if let Some(p) = part.strip_prefix("USE flags for ") {
            ("changing USE flags for", p.to_string())
        } else if let Some(p) = part.strip_prefix("testing versions for ") {
            ("allowing testing versions of", p.to_string())
        } else if let Some(t) = part.strip_prefix("admin rights: ") {
            ("setting up admin rights with", t.to_string())
        } else if part == "boot menu undo" {
            ("setting up", "undo in the boot menu".to_string())
        } else if part == "tidier boot menu" {
            ("tidying", "the boot menu".to_string())
        } else if part == "console font" {
            ("setting up", "the console font".to_string())
        } else {
            ("", part.to_string())
        };
        match parts.last_mut() {
            Some((v, items)) if !verb.is_empty() && *v == verb => items.push(what),
            _ => parts.push((verb, vec![what])),
        }
        if let Some(m) = more {
            tail = format!(" and {m}");
        }
    }
    let phrases: Vec<String> = parts
        .into_iter()
        .map(|(verb, items)| {
            let list = match items.as_slice() {
                [one] => one.to_string(),
                [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                [] => String::new(),
            };
            if verb.is_empty() {
                list
            } else {
                format!("{verb} {list}")
            }
        })
        .collect();
    format!("{}{tail}", phrases.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_a_version_was_before_in_words() {
        assert_eq!(before_words("- tree"), "removing tree");
        assert_eq!(before_words("− tree"), "removing tree");
        assert_eq!(
            before_words("+ tmux, + cronie, start cronie"),
            "adding tmux and cronie, starting cronie"
        );
        assert_eq!(
            before_words("+ a, + b, + c, - d"),
            "adding a, b and c, removing d"
        );
        assert_eq!(
            before_words("+ repository guru, + mangowm"),
            "adding the repository guru, adding mangowm"
        );
        assert_eq!(
            before_words("USE flags for curl, testing versions for mangowm"),
            "changing USE flags for curl, allowing testing versions of mangowm"
        );
        assert_eq!(
            before_words("admin rights: doas, boot menu undo, tidier boot menu"),
            "setting up admin rights with doas, setting up undo in the boot menu, \
             tidying the boot menu"
        );
        assert_eq!(
            before_words("+ a, + b, + c, + d, + e, + f and 2 more"),
            "adding a, b, c, d, e and f and 2 more"
        );
        // Something it doesn't know stays as it was.
        assert_eq!(before_words("something new"), "something new");
    }

    #[test]
    fn notes_and_state_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".snapshots")).unwrap();
        let note = Note {
            kind: Kind::Switch,
            summary: "+ tmux".into(),
            to: None,
        };
        write_note(dir.path(), "bunny-a", &note).unwrap();
        assert_eq!(read_note(dir.path(), "bunny-a"), Some(note));

        let state = State {
            last: Kind::Rollback,
            at: "x".into(),
            to: Some("bunny-a".into()),
        };
        write_state(dir.path(), &state).unwrap();
        assert_eq!(read_state(dir.path()), Some(state));
    }

    #[test]
    fn skips_quietly_without_snapshots_folder() {
        let dir = tempfile::tempdir().unwrap();
        let note = Note {
            kind: Kind::Switch,
            summary: String::new(),
            to: None,
        };
        write_note(dir.path(), "bunny-a", &note).unwrap();
        assert_eq!(read_note(dir.path(), "bunny-a"), None);
    }
}
