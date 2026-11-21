//! `[desktop]`: what a graphical desktop needs from the base system.
//!
//! Bunny Linux installs a minimal system, where logins don't go through
//! elogind. A Wayland desktop (Mango, Sway, Hyprland, …) needs them to: elogind
//! makes `/run/user/<id>` (`XDG_RUNTIME_DIR`) and hands the screen and
//! keyboard to the desktop through libseat. `wayland = true` adds exactly the
//! pieces that takes, as if they were written in the config by hand, so
//! `bunny plan` and `bunny diff` show them like any other setting, marked
//! "from [desktop]". A `[pkg]` entry of your own for one of these packages
//! always wins.
//!
//! Per-package flags rather than a global `USE=elogind` or Gentoo's desktop
//! profile: those rebuild every installed package that has the flag (or, for
//! the profile, turn on dozens of other flags too), and bunny doesn't manage
//! `make.conf` or the profile. See "Desktop" in docs/DESIGN.md.

use serde::Deserialize;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    /// Set up what any Wayland desktop needs: elogind logins and seats.
    #[serde(default)]
    pub wayland: bool,
}

/// Packages a Wayland desktop needs installed (kept in the world set).
const PACKAGES: &[&str] = &["sys-apps/dbus", "sys-auth/elogind"];

/// USE flags a Wayland desktop needs: logins go through elogind (so it makes
/// `XDG_RUNTIME_DIR`), and libseat can ask elogind for the screen.
const USE_FLAGS: &[(&str, &[&str])] = &[
    ("sys-auth/pambase", &["elogind"]),
    ("sys-auth/seatd", &["elogind"]),
];

/// Services a Wayland desktop needs at boot.
const SERVICES: &[&str] = &["dbus", "elogind"];

impl Desktop {
    /// Whether the section asks for anything.
    pub fn is_on(&self) -> bool {
        self.wayland
    }

    pub fn packages(&self) -> &'static [&'static str] {
        if self.is_on() {
            PACKAGES
        } else {
            &[]
        }
    }

    pub fn use_flags(&self) -> &'static [(&'static str, &'static [&'static str])] {
        if self.is_on() {
            USE_FLAGS
        } else {
            &[]
        }
    }

    pub fn services(&self) -> &'static [&'static str] {
        if self.is_on() {
            SERVICES
        } else {
            &[]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_adds_nothing() {
        let d = Desktop::default();
        assert!(d.packages().is_empty());
        assert!(d.use_flags().is_empty());
        assert!(d.services().is_empty());
    }

    #[test]
    fn wayland_adds_elogind_logins_and_seats() {
        let d = Desktop { wayland: true };
        assert_eq!(d.services(), ["dbus", "elogind"]);
        assert_eq!(d.packages(), ["sys-apps/dbus", "sys-auth/elogind"]);
        let atoms: Vec<&str> = d.use_flags().iter().map(|(a, _)| *a).collect();
        assert_eq!(atoms, ["sys-auth/pambase", "sys-auth/seatd"]);
    }
}
