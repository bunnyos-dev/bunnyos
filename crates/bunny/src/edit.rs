//! Changing `bunny.toml` for the user (`bunny add`, `bunny remove`), keeping
//! their comments, order and layout.

use anyhow::{bail, Context, Result};
use toml_edit::{Array, DocumentMut, Item, Value};

#[derive(Debug)]
pub struct Edited {
    pub text: String,
    /// Names that changed.
    pub changed: Vec<String>,
    /// Names that were already there (add) or weren't there (remove).
    pub unchanged: Vec<String>,
}

fn parse(text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>().context(
        "bunny.toml has a mistake in it, so bunny couldn't edit it. Run `bunny check` to see it.",
    )
}

/// Whether the array is written one item per line.
fn is_multiline(array: &Array) -> bool {
    array.is_empty()
        || array.iter().any(|v| {
            v.decor()
                .prefix()
                .and_then(|p| p.as_str())
                .is_some_and(|p| p.contains('\n'))
        })
}

fn packages_array(doc: &mut DocumentMut) -> Result<&mut Array> {
    if !doc.contains_key("packages") {
        doc.insert("packages", Item::Value(Value::Array(Array::new())));
    }
    match doc["packages"].as_array_mut() {
        Some(array) => Ok(array),
        None => {
            bail!("`packages` in bunny.toml should be a list, like packages = [\"app-misc/tmux\"].")
        }
    }
}

/// The text before an item: the previous line's comment (if any), a line
/// break and the indentation.
fn prefix_of(value: &Value) -> String {
    value
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or("\n    ")
        .to_string()
}

const ITEM_PREFIX: &str = "\n    ";

/// Adds a repository to `repositories` (made if it isn't there, above the
/// first section). Returns the text unchanged if it's already listed.
pub fn add_repository(text: &str, name: &str) -> Result<String> {
    let mut doc = parse(text)?;
    if !doc.contains_key("repositories") {
        let mut array = Array::new();
        array.push(name);
        let mut item = Item::Value(Value::Array(array));
        if let Some(value) = item.as_value_mut() {
            value.decor_mut().set_prefix(" ");
        }
        doc.insert("repositories", item);
        if let Some(mut key) = doc.key_mut("repositories") {
            key.leaf_decor_mut().set_prefix(
                "\n# Package repositories besides Gentoo's own (see the Guide: bunny.toml).\n",
            );
        }
        return Ok(doc.to_string());
    }
    let Some(array) = doc["repositories"].as_array_mut() else {
        bail!("`repositories` in bunny.toml should be a list, like repositories = [\"guru\"].");
    };
    if array.iter().any(|v| v.as_str() == Some(name)) {
        return Ok(text.to_string());
    }
    array.push(name);
    Ok(doc.to_string())
}

/// Sets `keywords` for a package (`[pkg."category/name"]`), keeping its
/// other settings.
pub fn set_keywords(text: &str, atom: &str, keywords: &[&str]) -> Result<String> {
    let mut doc = parse(text)?;
    if !doc.contains_key("pkg") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        doc.insert("pkg", Item::Table(table));
    }
    let Some(pkg) = doc["pkg"].as_table_mut() else {
        bail!("`pkg` in bunny.toml should be a section, like [pkg.\"app-misc/tmux\"].");
    };
    if !pkg.contains_key(atom) {
        pkg.insert(atom, Item::Table(toml_edit::Table::new()));
    }
    let Some(entry) = pkg[atom].as_table_like_mut() else {
        bail!("[pkg.\"{atom}\"] in bunny.toml should be a section.");
    };
    let mut array = Array::new();
    for k in keywords {
        array.push(*k);
    }
    entry.insert("keywords", Item::Value(Value::Array(array)));
    Ok(doc.to_string())
}

/// Adds packages to `packages`, in alphabetical position.
///
/// In TOML, a comment after an item is stored at the start of the *next*
/// item (or after the last one), so inserting moves it along: a new item
/// takes over the text before the item it's inserted in front of.
pub fn add_packages(text: &str, names: &[String]) -> Result<Edited> {
    let mut doc = parse(text)?;
    let array = packages_array(&mut doc)?;
    let multiline = is_multiline(array);
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();

    for name in names {
        if array.iter().any(|v| v.as_str() == Some(name)) {
            unchanged.push(name.clone());
            continue;
        }
        let position = array
            .iter()
            .position(|v| v.as_str().is_some_and(|s| s > name.as_str()))
            .unwrap_or(array.len());
        let mut value = Value::from(name.as_str());
        if multiline {
            let prefix = if position < array.len() {
                let before_next = prefix_of(array.get(position).expect("in range"));
                if let Some(next) = array.get_mut(position) {
                    next.decor_mut().set_prefix(ITEM_PREFIX);
                }
                before_next
            } else {
                // Appending: a comment after the old last item stays on its line.
                let trailing = array.trailing().as_str().unwrap_or("").to_string();
                array.set_trailing("\n");
                format!("{}{ITEM_PREFIX}", trailing.trim_end_matches('\n'))
            };
            value.decor_mut().set_prefix(prefix);
            value.decor_mut().set_suffix("");
        }
        array.insert_formatted(position, value);
        changed.push(name.clone());
    }

    if multiline {
        array.set_trailing_comma(true);
        if array
            .trailing()
            .as_str()
            .is_some_and(|t| !t.ends_with('\n'))
        {
            array.set_trailing("\n");
        }
    } else {
        array.fmt();
    }
    Ok(Edited {
        text: doc.to_string(),
        changed,
        unchanged,
    })
}

/// Removes packages from `packages`. Each name can be the full name or the
/// short one (`tmux` for `app-misc/tmux`). The removed item's own comment goes
/// with it; the comment of the item before it stays.
pub fn remove_packages(text: &str, names: &[String]) -> Result<Edited> {
    let mut doc = parse(text)?;
    let array = packages_array(&mut doc)?;
    let multiline = is_multiline(array);
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();

    for name in names {
        let found = array.iter().position(|v| {
            v.as_str().is_some_and(|s| {
                let plain = s.split(':').next().unwrap_or(s);
                s == name || plain == name || plain.rsplit('/').next() == Some(name.as_str())
            })
        });
        let Some(index) = found else {
            unchanged.push(name.clone());
            continue;
        };
        let full = array
            .get(index)
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .to_string();
        if multiline {
            let before = prefix_of(array.get(index).expect("in range"));
            if index + 1 < array.len() {
                if let Some(next) = array.get_mut(index + 1) {
                    next.decor_mut().set_prefix(before);
                }
            } else {
                // Removing the last item: the comment before it ends the list.
                array.set_trailing(format!(
                    "{}\n",
                    before.trim_end_matches(' ').trim_end_matches('\n')
                ));
            }
        }
        array.remove(index);
        changed.push(full);
    }

    if !multiline {
        array.fmt();
    }
    Ok(Edited {
        text: doc.to_string(),
        changed,
        unchanged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_a_repository_and_keywords() {
        use crate::config::Config;
        let text =
            "# My system\npackages = [\n    \"app-misc/tmux\",\n]\n\n[boot]\nundo_entry = true\n";
        let with_repo = add_repository(text, "guru").unwrap();
        let c = Config::parse(&with_repo).unwrap_or_else(|e| panic!("{e}\n{with_repo}"));
        assert_eq!(c.repositories, ["guru"]);
        assert!(c.boot.undo_entry, "{with_repo}");
        assert!(with_repo.starts_with("# My system\n"));
        // Already there: unchanged.
        assert_eq!(add_repository(&with_repo, "guru").unwrap(), with_repo);
        let two = add_repository(&with_repo, "hyproverlay").unwrap();
        assert_eq!(
            Config::parse(&two).unwrap().repositories,
            ["guru", "hyproverlay"]
        );

        let kw = set_keywords(&two, "gui-wm/mangowm", &["~amd64"]).unwrap();
        let kw = set_keywords(&kw, "gui-libs/scenefx", &["~amd64"]).unwrap();
        let c = Config::parse(&kw).unwrap_or_else(|e| panic!("{e}\n{kw}"));
        assert_eq!(c.keywords()["gui-wm/mangowm"], ["~amd64"]);
        assert_eq!(c.keywords()["gui-libs/scenefx"], ["~amd64"]);
        assert!(kw.contains("[pkg.\"gui-wm/mangowm\"]"), "{kw}");
        assert!(c.problems().is_empty());

        // Keeps a package's USE flags.
        let use_too = "[pkg.\"gui-wm/dwl\"]\nuse = [\"X\"]\n";
        let c = Config::parse(&set_keywords(use_too, "gui-wm/dwl", &["~amd64"]).unwrap()).unwrap();
        assert_eq!(c.pkg["gui-wm/dwl"].use_flags, ["X"]);
        assert_eq!(c.pkg["gui-wm/dwl"].keywords, ["~amd64"]);
    }
    use crate::config::Config;

    const CONFIG: &str = r#"# My desktop
packages = [
    "app-editors/neovim",
    "www-client/firefox",  # my browser
]

services = ["sshd"]

[pkg."www-client/firefox"]
use = ["pgo"]
"#;

    #[test]
    fn adds_in_order_keeping_comments_and_layout() {
        let out = add_packages(CONFIG, &["app-misc/tmux".into(), "zzz-cat/last".into()]).unwrap();
        assert_eq!(out.changed, ["app-misc/tmux", "zzz-cat/last"]);
        let expected = r#"# My desktop
packages = [
    "app-editors/neovim",
    "app-misc/tmux",
    "www-client/firefox",  # my browser
    "zzz-cat/last",
]

services = ["sshd"]

[pkg."www-client/firefox"]
use = ["pgo"]
"#;
        assert_eq!(out.text, expected);
        assert!(Config::parse(&out.text).unwrap().problems().is_empty());
    }

    #[test]
    fn adding_twice_changes_nothing() {
        let out = add_packages(CONFIG, &["app-editors/neovim".into()]).unwrap();
        assert!(out.changed.is_empty());
        assert_eq!(out.unchanged, ["app-editors/neovim"]);
        assert_eq!(out.text, CONFIG);
    }

    #[test]
    fn adds_to_a_config_without_packages() {
        let out = add_packages("[system]\ninit = \"openrc\"\n", &["app-misc/tmux".into()]).unwrap();
        let config = Config::parse(&out.text).unwrap_or_else(|e| panic!("{e}\n{}", out.text));
        assert_eq!(config.packages, ["app-misc/tmux"]);
        assert!(out.text.starts_with("packages = ["), "{}", out.text);
    }

    #[test]
    fn single_line_lists_stay_single_line() {
        let out = add_packages("packages = [\"b/b\"]\n", &["a/a".into()]).unwrap();
        assert_eq!(out.text, "packages = [\"a/a\", \"b/b\"]\n");
    }

    #[test]
    fn removes_by_full_or_short_name() {
        let out = remove_packages(CONFIG, &["firefox".into(), "app-misc/nope".into()]).unwrap();
        assert_eq!(out.changed, ["www-client/firefox"]);
        assert_eq!(out.unchanged, ["app-misc/nope"]);
        assert!(
            out.text.starts_with(
                "# My desktop\npackages = [\n    \"app-editors/neovim\",\n]\n\nservices"
            ),
            "{}",
            out.text
        );
        assert!(Config::parse(&out.text).is_ok());
    }

    #[test]
    fn removes_the_first_item_keeping_layout() {
        let out = remove_packages(CONFIG, &["app-editors/neovim".into()]).unwrap();
        assert!(
            out.text
                .contains("packages = [\n    \"www-client/firefox\",  # my browser\n]"),
            "{}",
            out.text
        );
    }

    #[test]
    fn broken_config_is_explained() {
        let err = add_packages("packages = [", &["a/a".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("bunny check"));
    }
}
