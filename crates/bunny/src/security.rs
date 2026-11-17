//! `[security]`: which tool gives admin rights (sudo or doas), and who gets them.
//!
//! bunny writes one small file (`/etc/sudoers.d/bunny` or `/etc/doas.conf`)
//! that gives the `wheel` group admin rights, checks it with the tool's own
//! checker before putting it in place, and adds the listed admins to `wheel`.
//! It never removes anyone from `wheel`, so a config mistake can't lock you out.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// The `[security]` section.
#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Security {
    #[serde(default)]
    pub privilege: Privilege,
    /// Remember the password for a few minutes after typing it.
    #[serde(default = "default_true")]
    pub persist: bool,
    /// Users who get admin rights (they're added to the `wheel` group).
    #[serde(default)]
    pub admins: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Privilege {
    /// The default: what most guides and scripts expect.
    #[default]
    Sudo,
    Doas,
}

impl Privilege {
    pub fn command(self) -> &'static str {
        match self {
            Privilege::Sudo => "sudo",
            Privilege::Doas => "doas",
        }
    }

    pub fn package(self) -> &'static str {
        match self {
            Privilege::Sudo => "app-admin/sudo",
            Privilege::Doas => "app-admin/doas",
        }
    }

    /// USE flags BunnyOS sets for the tool unless the config has its own
    /// `[pkg]` entry for it. sudo's `sendmail` flag pulls in a mail server
    /// (nullmailer) just to email root about failed attempts; desktops don't
    /// need that.
    pub fn default_use(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Privilege::Sudo => &[("app-admin/sudo", &["-sendmail"])],
            Privilege::Doas => &[],
        }
    }

    /// The file bunny manages, relative to the root.
    pub fn file(self) -> &'static str {
        match self {
            Privilege::Sudo => "etc/sudoers.d/bunny",
            Privilege::Doas => "etc/doas.conf",
        }
    }

    /// Permissions the file must have (sudo refuses group/world-writable files).
    pub fn mode(self) -> u32 {
        match self {
            Privilege::Sudo => 0o440,
            Privilege::Doas => 0o600,
        }
    }

    /// The command that checks a file before it's put in place.
    pub fn check_command(self, file: &Path) -> Vec<String> {
        let file = file.to_string_lossy().into_owned();
        match self {
            Privilege::Sudo => vec!["visudo".into(), "-cf".into(), file],
            Privilege::Doas => vec!["doas".into(), "-C".into(), file],
        }
    }
}

/// What the managed file should contain.
pub fn render(sec: &Security) -> String {
    let header = "# Managed by bunny from the [security] section of bunny.toml.\n\
                  # Edit bunny.toml instead of this file.\n";
    match sec.privilege {
        Privilege::Sudo => {
            let mut out =
                format!("{header}%wheel ALL=(ALL:ALL) ALL\nDefaults:%wheel lecture=never\n");
            if !sec.persist {
                out.push_str("Defaults:%wheel timestamp_timeout=0\n");
            }
            out
        }
        Privilege::Doas => format!(
            "{header}permit {}:wheel\n",
            if sec.persist { "persist " } else { "" }
        ),
    }
}

/// What a switch would change for `[security]`.
#[derive(Debug, Default, PartialEq)]
pub struct Changes {
    /// The file to write and its new contents, if it differs from what's there.
    pub file: Option<(PathBuf, String)>,
    pub privilege: Option<Privilege>,
    /// Admins to add to the `wheel` group.
    pub add_to_wheel: Vec<String>,
    /// Reasons the switch must not go ahead (it could lock everyone out).
    pub problems: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.file.is_none() && self.add_to_wheel.is_empty()
    }
}

pub fn changes(sec: &Security, root: &Path) -> Changes {
    let mut out = Changes {
        privilege: Some(sec.privilege),
        ..Changes::default()
    };
    let path = root.join(sec.privilege.file());
    let wanted = render(sec);
    match std::fs::read_to_string(&path) {
        Ok(current) if current == wanted => {}
        // sudo's file is readable by root only, so read-only commands run
        // as a normal user can't tell. Don't report a change that may not
        // exist; `bunny switch` runs as root and sees the real file.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
        _ => out.file = Some((path, wanted)),
    }

    let users = users(root);
    let wheel = group_members(root, "wheel");
    for admin in &sec.admins {
        if !users.contains(admin) {
            out.problems.push(format!(
                "\"{admin}\" in [security] admins isn't a user on this system. \
                 Check the spelling, or create the user first."
            ));
        } else if !wheel.contains(admin) {
            out.add_to_wheel.push(admin.clone());
        }
    }
    // Only wheel gets admin rights, so an empty wheel means nobody could
    // ever use sudo/doas again. Only root logins would be left.
    if wheel.is_empty() && out.add_to_wheel.is_empty() && out.problems.is_empty() {
        out.problems.push(format!(
            "Nobody is in the wheel group, so after this nobody could use {}. \
             Add yourself to [security] admins, for example: admins = [\"you\"]",
            sec.privilege.command()
        ));
    }
    out
}

/// User names from `/etc/passwd`.
fn users(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("etc/passwd"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split(':').next())
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect()
}

/// Members of a group from `/etc/group`.
pub fn group_members(root: &Path, group: &str) -> Vec<String> {
    std::fs::read_to_string(root.join("etc/group"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let mut parts = l.split(':');
            (parts.next() == Some(group)).then(|| parts.nth(2).unwrap_or("").to_string())
        })
        .map(|m| {
            m.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Finds a program in `$PATH`.
fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The admin command to use on this machine ("sudo" or "doas"), for running
/// bunny as root and for the commands bunny suggests. Prefers the one chosen
/// in `/etc/bunny/bunny.toml` if it's installed, then whichever is installed.
pub fn admin() -> &'static str {
    static ADMIN: OnceLock<&'static str> = OnceLock::new();
    ADMIN.get_or_init(|| {
        let chosen = crate::config::Config::load(Path::new("/etc/bunny/bunny.toml"))
            .ok()
            .and_then(|c| c.security)
            .map(|s| s.privilege.command());
        chosen
            .into_iter()
            .chain(["sudo", "doas"])
            .find(|p| in_path(p))
            .unwrap_or("sudo")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sec(privilege: Privilege, persist: bool, admins: &[&str]) -> Security {
        Security {
            privilege,
            persist,
            admins: admins.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn fake_root(wheel: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        std::fs::write(
            dir.path().join("etc/passwd"),
            "root:x:0:0::/root:/bin/bash\nyou:x:1000:1000::/home/you:/bin/bash\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("etc/group"),
            format!("root::0:root\nwheel::10:{wheel}\nusers::100:\n"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn sudo_is_the_default() {
        let s: Security = toml::from_str("").unwrap();
        assert_eq!(s.privilege, Privilege::Sudo);
        assert!(s.persist);
    }

    #[test]
    fn renders_each_tool() {
        assert!(render(&sec(Privilege::Doas, true, &[])).ends_with("permit persist :wheel\n"));
        assert!(render(&sec(Privilege::Doas, false, &[])).ends_with("permit :wheel\n"));
        let sudo = render(&sec(Privilege::Sudo, true, &[]));
        assert!(sudo.contains("%wheel ALL=(ALL:ALL) ALL\n"));
        assert!(sudo.ends_with("Defaults:%wheel lecture=never\n"));
        assert!(render(&sec(Privilege::Sudo, false, &[]))
            .ends_with("Defaults:%wheel timestamp_timeout=0\n"));
    }

    #[test]
    fn reads_group_members() {
        let root = fake_root("you,alex");
        assert_eq!(group_members(root.path(), "wheel"), ["you", "alex"]);
        assert!(group_members(root.path(), "users").is_empty());
        assert!(group_members(root.path(), "nope").is_empty());
    }

    #[test]
    fn writes_the_file_and_adds_admins() {
        let root = fake_root("");
        let c = changes(&sec(Privilege::Sudo, true, &["you"]), root.path());
        assert!(c.problems.is_empty());
        assert_eq!(c.add_to_wheel, ["you"]);
        let (path, text) = c.file.unwrap();
        assert!(path.ends_with("etc/sudoers.d/bunny"));
        assert!(text.contains("%wheel"));
    }

    #[test]
    fn nothing_to_do_when_already_set_up() {
        let root = fake_root("you");
        let s = sec(Privilege::Doas, true, &["you"]);
        std::fs::write(root.path().join("etc/doas.conf"), render(&s)).unwrap();
        assert!(changes(&s, root.path()).is_empty());
    }

    #[test]
    fn refuses_to_lock_everyone_out() {
        let root = fake_root("");
        let c = changes(&sec(Privilege::Sudo, true, &[]), root.path());
        assert!(c.problems[0].contains("Nobody is in the wheel group"));
    }

    #[test]
    fn unknown_admins_are_a_problem() {
        let root = fake_root("you");
        let c = changes(&sec(Privilege::Sudo, true, &["yuo"]), root.path());
        assert!(c.problems[0].contains("\"yuo\""));
        assert!(c.add_to_wheel.is_empty());
    }

    #[test]
    fn check_commands() {
        assert_eq!(
            Privilege::Sudo.check_command(Path::new("/x")),
            ["visudo", "-cf", "/x"]
        );
        assert_eq!(
            Privilege::Doas.check_command(Path::new("/x")),
            ["doas", "-C", "/x"]
        );
    }

    #[test]
    fn a_file_only_root_can_read_isnt_reported_as_changed() {
        use std::os::unix::fs::PermissionsExt;
        let root = fake_root("you");
        let s = sec(Privilege::Sudo, true, &["you"]);
        let path = root.path().join("etc/sudoers.d/bunny");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old contents").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_to_string(&path).is_ok(); // true when tests run as root
        let c = changes(&s, root.path());
        assert_eq!(c.file.is_some(), readable);
    }
}
