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

#[cfg(test)]
mod tests {
    use super::*;

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
