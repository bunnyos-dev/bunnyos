//! `bunny.toml`: what the user wants their system to be.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// The whole config file.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub system: System,
    /// Packages to install, as `category/name` (the world set).
    #[serde(default)]
    pub packages: Vec<String>,
    /// Services to enable at boot, by plain name.
    #[serde(default)]
    pub services: Vec<String>,
    /// Per-package settings, keyed by `category/name`.
    #[serde(default)]
    pub pkg: BTreeMap<String, PkgSettings>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct System {
    #[serde(default)]
    pub init: Init,
    #[serde(default = "default_true")]
    pub multilib: bool,
}

impl Default for System {
    fn default() -> Self {
        Self {
            init: Init::default(),
            multilib: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Init {
    #[default]
    Openrc,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PkgSettings {
    /// USE flags, e.g. `["pgo", "-telemetry"]`.
    #[serde(default, rename = "use")]
    pub use_flags: Vec<String>,
}

/// Something wrong with the config, explained for humans.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub what: String,
    pub fix: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n    Fix: {}", self.what, self.fix)
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Couldn't read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("{} has a mistake in it", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|err| {
            let message = err.to_string();
            match misplaced_top_level_key(&message) {
                Some(key) => anyhow::anyhow!(
                    "{message}\n\
                     `{key}` ended up inside a [section] because it comes after one.\n\
                     In TOML, everything below a [section] header belongs to that section.\n\
                     Fix: move `{key}` above the first [section] in the file."
                ),
                None => anyhow::Error::new(err),
            }
        })
    }

    /// Checks that go beyond "is this valid TOML".
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();

        for atom in &self.packages {
            if !is_package_name(atom) {
                problems.push(Problem {
                    what: format!("\"{atom}\" in `packages` isn't a full package name."),
                    fix: "Use category/name, for example \"www-client/firefox\".".into(),
                });
            }
        }
        check_duplicates(&self.packages, "packages", &mut problems);

        for name in &self.services {
            if name.is_empty() || name.contains('/') || name.contains(char::is_whitespace) {
                problems.push(Problem {
                    what: format!("\"{name}\" in `services` isn't a valid service name."),
                    fix: "Use the plain service name, for example \"sshd\".".into(),
                });
            }
        }
        check_duplicates(&self.services, "services", &mut problems);

        for (atom, settings) in &self.pkg {
            if !is_package_name(atom) {
                problems.push(Problem {
                    what: format!("[pkg.\"{atom}\"] isn't a full package name."),
                    fix: "Use category/name, for example [pkg.\"www-client/firefox\"].".into(),
                });
            }
            for flag in &settings.use_flags {
                if !is_use_flag(flag) {
                    problems.push(Problem {
                        what: format!("\"{flag}\" in the USE flags for {atom} isn't a valid flag."),
                        fix: "Flags are words like \"pgo\", or \"-telemetry\" to turn one off."
                            .into(),
                    });
                }
            }
            check_use_conflicts(atom, &settings.use_flags, &mut problems);
        }

        problems
    }
}

/// Top-level keys that are easy to accidentally put below a `[section]`.
const TOP_LEVEL_KEYS: [&str; 2] = ["packages", "services"];

/// If a TOML error is a top-level key that landed inside a section, returns it.
fn misplaced_top_level_key(message: &str) -> Option<&'static str> {
    // At the top level the error lists `system` as an expected field;
    // inside a section it doesn't.
    if message.contains("`system`") {
        return None;
    }
    TOP_LEVEL_KEYS
        .into_iter()
        .find(|key| message.contains(&format!("unknown field `{key}`")))
}

/// `category/name`, using the characters Gentoo allows.
fn is_package_name(s: &str) -> bool {
    let Some((cat, name)) = s.split_once('/') else {
        return false;
    };
    let ok = |part: &str| {
        !part.is_empty()
            && !part.starts_with(['-', '.', '+'])
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+_.-".contains(c))
    };
    ok(cat) && ok(name) && !name.contains('/')
}

/// A USE flag, optionally prefixed with `-` to disable it.
fn is_use_flag(s: &str) -> bool {
    let flag = s.strip_prefix('-').unwrap_or(s);
    let mut chars = flag.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+_@-".contains(c))
}

fn check_duplicates(items: &[String], field: &str, problems: &mut Vec<Problem>) {
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        if !seen.insert(item) {
            problems.push(Problem {
                what: format!("\"{item}\" is listed twice in `{field}`."),
                fix: "Remove one of them.".into(),
            });
        }
    }
}

fn check_use_conflicts(atom: &str, flags: &[String], problems: &mut Vec<Problem>) {
    for flag in flags.iter().filter(|f| !f.starts_with('-')) {
        if flags.iter().any(|f| f.strip_prefix('-') == Some(flag)) {
            problems.push(Problem {
                what: format!("{atom} has the USE flag \"{flag}\" both on and off."),
                fix: format!("Keep either \"{flag}\" or \"-{flag}\"."),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.system.init, Init::Openrc);
        assert!(config.system.multilib);
        assert!(config.problems().is_empty());
    }

    #[test]
    fn parses_a_full_config() {
        let config = Config::parse(
            r#"
            packages = ["www-client/firefox", "app-editors/neovim"]
            services = ["sshd"]

            [system]
            init = "openrc"
            multilib = false

            [pkg."www-client/firefox"]
            use = ["pgo", "-telemetry"]
            "#,
        )
        .unwrap();
        assert_eq!(config.packages.len(), 2);
        assert!(!config.system.multilib);
        assert_eq!(
            config.pkg["www-client/firefox"].use_flags,
            vec!["pgo", "-telemetry"]
        );
        assert!(config.problems().is_empty());
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = Config::parse("pakages = []").unwrap_err().to_string();
        assert!(err.contains("pakages"), "{err}");
    }

    #[test]
    fn explains_keys_placed_after_a_section() {
        let err = Config::parse("[system]\nmultilib = true\npackages = []")
            .unwrap_err()
            .to_string();
        assert!(err.contains("move `packages` above"), "{err}");
    }

    #[test]
    fn plain_typos_get_no_section_hint() {
        let err = Config::parse("pakages = []").unwrap_err().to_string();
        assert!(!err.contains("above the first"), "{err}");
    }

    #[test]
    fn rejects_systemd_for_now() {
        assert!(Config::parse("[system]\ninit = \"systemd\"").is_err());
    }

    #[test]
    fn finds_problems() {
        let config = Config::parse(
            r#"
            packages = ["firefox", "app-editors/neovim", "app-editors/neovim"]
            services = ["bad name"]
            [pkg."www-client/firefox"]
            use = ["pgo", "-pgo", "bad!flag"]
            "#,
        )
        .unwrap();
        let problems = config.problems();
        let text: Vec<_> = problems.iter().map(|p| p.what.as_str()).collect();
        assert_eq!(problems.len(), 5, "{text:#?}");
    }

    #[test]
    fn package_names() {
        assert!(is_package_name("www-client/firefox"));
        assert!(is_package_name("dev-libs/libsigc++"));
        assert!(!is_package_name("firefox"));
        assert!(!is_package_name("a/b/c"));
        assert!(!is_package_name("/firefox"));
    }

    #[test]
    fn use_flags() {
        assert!(is_use_flag("pgo"));
        assert!(is_use_flag("-telemetry"));
        assert!(is_use_flag("python_targets_python3_12"));
        assert!(!is_use_flag("-"));
        assert!(!is_use_flag("bad flag"));
    }
}
