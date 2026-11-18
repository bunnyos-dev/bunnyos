//! `bunny search`, `add` and `remove`: finding packages and editing the
//! config for you.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use super::switch::{switch_inner, Switched};
use super::{elevate, read_config_text};
use crate::{edit, lock, repo, security};

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

/// Turns what the user typed into full package names, or explains why not.
pub(crate) fn resolve_all(root: &Path, names: &[String]) -> anyhow::Result<Option<Vec<String>>> {
    let packages = repo::all(root);
    if packages.is_empty() {
        anyhow::bail!(
            "Couldn't find any package repositories in {}.\n  \
             Sync them first: {} emerge --sync",
            root.join("var/db/repos").display(),
            security::admin()
        );
    }
    let mut resolved = Vec::new();
    let mut ok = true;
    for name in names {
        match repo::resolve(&packages, name) {
            repo::Resolved::Found(full) => resolved.push(full),
            repo::Resolved::Ambiguous(options) => {
                ok = false;
                eprintln!("✗ Several packages are called \"{name}\". Pick one:");
                for option in options {
                    eprintln!("    bunny add {option}");
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
    let Some(full) = resolve_all(root, names)? else {
        eprintln!("Nothing was changed.");
        return Ok(ExitCode::FAILURE);
    };
    let old = read_config_text(config)?;
    let edited = edit::add_packages(&old, &full)?;
    for name in &edited.unchanged {
        println!("{name} is already in {}.", config.display());
    }
    if edited.changed.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Adding {} to {}.\n",
        edited.changed.join(", "),
        config.display()
    );
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
        Err(err) => Err(err),
    }
}
