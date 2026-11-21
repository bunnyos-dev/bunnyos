//! `bunny search`, `add` and `remove`: finding packages and editing the
//! config for you.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use super::switch::{switch_inner, Switched};
use super::{elevate, read_config_text};
use crate::{apply, edit, lock, repo, security};

pub(crate) fn search_cmd(query: &str, root: &Path) -> anyhow::Result<ExitCode> {
    let packages = repo::all(root);
    if packages.is_empty() {
        anyhow::bail!(
            "Couldn't find any package repositories in {}.\n  \
             Sync them first: {} emerge --sync",
            root.join("var/db/repos").display(),
            security::admin()
        );
    }
    let results = repo::search(&packages, query);
    if results.is_empty() {
        println!("Nothing matches \"{query}\". Try a shorter or different word.");
        return Ok(ExitCode::SUCCESS);
    }
    let installed: std::collections::BTreeSet<String> = lock::installed_packages(root)?
        .into_keys()
        .map(|k| k.split(':').next().unwrap_or(&k).to_string())
        .collect();
    const SHOWN: usize = 20;
    let width = results
        .iter()
        .take(SHOWN)
        .map(|p| p.name.len())
        .max()
        .unwrap_or(0);
    for p in results.iter().take(SHOWN) {
        let mark = if installed.contains(&p.name) {
            "  (installed)"
        } else {
            ""
        };
        println!(
            "  {:width$}  {}{mark}",
            p.name,
            p.description.as_deref().unwrap_or("")
        );
    }
    if results.len() > SHOWN {
        println!(
            "\n  …and {} more. Add another word to narrow it down.",
            results.len() - SHOWN
        );
    }
    println!("\nTo install one: bunny add <name>");
    Ok(ExitCode::SUCCESS)
}

/// The answer to "add this repository?": yes, no, or None to ask again
/// (Enter, or anything unclear). Never assumed either way.
fn repository_answer(answer: &str) -> Option<bool> {
    match answer.trim().to_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// What `bunny add` resolved: full names, plus repositories (and testing
/// versions) to add for packages that live outside Gentoo's own tree.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Resolved {
    pub names: Vec<String>,
    pub homes: Vec<crate::overlays::Home>,
}

/// Turns what the user typed into full package names, or explains why not.
/// For a well-known package that lives in another repository, says which and
/// asks whether to add it (`ask` gives the answer; Enter means no).
pub(crate) fn resolve_all(
    root: &Path,
    names: &[String],
    repositories: &[String],
    ask: &mut dyn FnMut(&str) -> anyhow::Result<bool>,
) -> anyhow::Result<Option<Resolved>> {
    let packages = repo::all(root);
    if packages.is_empty() {
        anyhow::bail!(
            "Couldn't find any package repositories in {}.\n  \
             Sync them first: {} emerge --sync",
            root.join("var/db/repos").display(),
            security::admin()
        );
    }
    let mut resolved = Resolved::default();
    let mut ok = true;
    for name in names {
        match repo::resolve(&packages, name) {
            repo::Resolved::Found(full) => resolved.names.push(full),
            repo::Resolved::Ambiguous(options) => {
                ok = false;
                eprintln!("✗ Several packages are called \"{name}\". Pick one:");
                for option in options {
                    eprintln!("    bunny add {option}");
                }
            }
            repo::Resolved::NotFound(_) if crate::overlays::known_home(name).is_some() => {
                let home = crate::overlays::known_home(name).expect("checked");
                if repositories.iter().any(|r| r == home.repository) {
                    // Listed but not downloaded yet: the switch gets it.
                    resolved.names.push(home.package.to_string());
                    resolved.homes.push(home);
                    continue;
                }
                eprintln!(
                    "· {} isn't in Gentoo's own package tree. It's in the {} repository.",
                    home.package, home.repository
                );
                for line in crate::overlays::trust_note(home.official).lines() {
                    eprintln!("  {line}");
                }
                if !home.testing.is_empty() {
                    eprintln!(
                        "  It only has testing versions there, so bunny also accepts those for:\n    {}",
                        home.testing.join(", ")
                    );
                }
                let question = format!(
                    "Add the {} repository to your config, and install {} from it? [y/n]",
                    home.repository, home.package
                );
                if ask(&question)? {
                    resolved.names.push(home.package.to_string());
                    resolved.homes.push(home);
                } else {
                    ok = false;
                }
            }
            repo::Resolved::NotFound(similar) => {
                ok = false;
                eprintln!("✗ There's no package called \"{name}\".");
                if !similar.is_empty() {
                    eprintln!("  Did you mean: {}?", similar.join(", "));
                }
                eprintln!("  Search for it with: bunny search {name}");
            }
        }
    }
    Ok(ok.then_some(resolved))
}

pub(crate) fn add_cmd(
    names: &[String],
    config: &Path,
    yes: bool,
    root: &Path,
) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let old = read_config_text(config)?;
    let listed = crate::config::Config::parse(&old)
        .map(|c| c.repositories)
        .unwrap_or_default();
    let mut ask = |q: &str| -> anyhow::Result<bool> {
        use std::io::Write;
        print!("{q} ");
        loop {
            std::io::stdout().flush()?;
            let Some(answer) = crate::interrupt::read_line()? else {
                println!("\n(No answer, so the repository wasn't added.)");
                return Ok(false);
            };
            match repository_answer(&answer) {
                Some(yes) => return Ok(yes),
                None => print!(
                    "Adding a repository is your call, so Enter doesn't pick for you.\n\
                     Type y to add it, or n to skip: "
                ),
            }
        }
    };
    let Some(found) = resolve_all(root, names, &listed, &mut ask)? else {
        eprintln!("Nothing was changed.");
        return Ok(ExitCode::FAILURE);
    };
    let mut text = old.clone();
    for home in &found.homes {
        text = edit::add_repository(&text, home.repository)?;
        for atom in home.testing {
            text = edit::set_keywords(&text, atom, &["~amd64"])?;
        }
    }
    let edited = edit::add_packages(&text, &found.names)?;
    for name in &edited.unchanged {
        println!("{name} is already in {}.", config.display());
    }
    if edited.changed.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    let repos: Vec<&str> = found.homes.iter().map(|h| h.repository).collect();
    if repos.is_empty() {
        println!(
            "Adding {} to {}.\n",
            edited.changed.join(", "),
            config.display()
        );
    } else {
        println!(
            "Adding {} (and the {} repository) to {}.\n",
            edited.changed.join(", "),
            repos.join(", "),
            config.display()
        );
    }
    edit_then_switch(config, &old, &edited.text, yes, root, false)
}

pub(crate) fn remove_cmd(
    names: &[String],
    config: &Path,
    yes: bool,
    root: &Path,
) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let old = read_config_text(config)?;
    let edited = edit::remove_packages(&old, names)?;
    for name in &edited.unchanged {
        println!(
            "\"{name}\" isn't in {}, so there's nothing to remove.",
            config.display()
        );
    }
    if edited.changed.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Removing {} from {}.\n",
        edited.changed.join(", "),
        config.display()
    );
    edit_then_switch(config, &old, &edited.text, yes, root, true)
}

/// Saves the edited config and switches. If nothing gets applied (problems,
/// or the user says no), the old config is put back.
pub(crate) fn edit_then_switch(
    config: &Path,
    old: &str,
    new: &str,
    yes: bool,
    root: &Path,
    removing: bool,
) -> anyhow::Result<ExitCode> {
    std::fs::write(config, new).with_context(|| format!("Couldn't write {}", config.display()))?;
    let result = switch_inner(config, yes, root, removing);
    let put_back = || {
        std::fs::write(config, old)
            .with_context(|| format!("Couldn't put {} back", config.display()))
    };
    match result {
        Ok(Switched::Applied) | Ok(Switched::NothingToDo) => Ok(ExitCode::SUCCESS),
        Ok(Switched::NotApplied(code)) => {
            put_back()?;
            println!("{} wasn't changed either.", config.display());
            Ok(code)
        }
        Err(err) if apply::changed_nothing(&err) => {
            put_back()?;
            eprintln!("✗ {err:#}");
            println!("{} wasn't changed either.", config.display());
            Ok(ExitCode::FAILURE)
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_repo() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let cache = root
            .path()
            .join("var/db/repos/gentoo/metadata/md5-cache/app-misc");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("tmux-3.5"), "DESCRIPTION=Terminal multiplexer\n").unwrap();
        root
    }

    #[test]
    fn enter_doesnt_answer_the_repository_question() {
        assert_eq!(repository_answer("y\n"), Some(true));
        assert_eq!(repository_answer(" Yes "), Some(true));
        assert_eq!(repository_answer("n"), Some(false));
        assert_eq!(repository_answer(""), None);
        assert_eq!(repository_answer("\n"), None);
        assert_eq!(repository_answer("bunny status"), None);
    }

    #[test]
    fn offers_the_repository_a_known_package_lives_in() {
        let root = fake_repo();
        let names = vec!["tmux".to_string(), "mangowm".to_string()];
        let mut asked = Vec::new();
        let mut yes = |q: &str| {
            asked.push(q.to_string());
            Ok(true)
        };
        let found = resolve_all(root.path(), &names, &[], &mut yes)
            .unwrap()
            .unwrap();
        assert_eq!(found.names, ["app-misc/tmux", "gui-wm/mangowm"]);
        assert_eq!(found.homes[0].repository, "guru");
        assert!(asked[0].contains("Add the guru repository"), "{asked:?}");

        // Saying no changes nothing.
        let mut no = |_: &str| Ok(false);
        assert!(resolve_all(root.path(), &names, &[], &mut no)
            .unwrap()
            .is_none());

        // Already in `repositories` (not downloaded yet): no question.
        let mut never = |_: &str| -> anyhow::Result<bool> { panic!("asked") };
        let found = resolve_all(
            root.path(),
            &["mangowm".into()],
            &["guru".into()],
            &mut never,
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.names, ["gui-wm/mangowm"]);
    }
}
