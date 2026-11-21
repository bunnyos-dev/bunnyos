//! Commands that look and explain: `bunny diff`, `lock` and `explain`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use super::report_problems;
use crate::config::Config;
use crate::{diff, explain, lock, plan};

/// Exits 0 when there are no differences and 1 when there are, like diff(1).
pub(crate) fn diff_cmd(config: &Path, root: &Path) -> anyhow::Result<ExitCode> {
    let cfg = Config::load(config)?;
    if !cfg.problems().is_empty() {
        let _ = report_problems(&cfg, config);
        eprintln!("Fix these first, then run `bunny diff` again.");
        return Ok(ExitCode::FAILURE);
    }
    let plan = plan::make(&cfg, root)?;
    let lock_path = lock::path_for(config);
    let drift = if lock_path.exists() {
        Some(lock::compare(
            &lock::load(&lock_path)?,
            &lock::from_system(root)?,
        ))
    } else {
        None
    };
    let shown = config.display().to_string();
    let d = diff::Diff {
        config: &shown,
        plan: &plan,
        drift: drift.as_ref(),
    };
    print!("{}", d.render());
    Ok(if d.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

pub(crate) fn lock_cmd(config: &Path, root: &Path) -> anyhow::Result<ExitCode> {
    let path = lock::path_for(config);
    let new = lock::from_system(root)?;
    if new.packages.is_empty() {
        anyhow::bail!(
            "Couldn't find any installed packages under {}.\n  \
             Is this a Gentoo system?",
            root.join("var/db/pkg").display()
        );
    }
    if path.exists() {
        let old = lock::load(&path)?;
        let drift = lock::compare(&old, &new);
        if drift.is_empty() {
            println!("✓ {} is already up to date.", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        println!(
            "Updating {}. What changed since it was written:",
            path.display()
        );
        for (name, old_v, new_v) in &drift.changed {
            println!("  ~ {name}: {old_v} → {new_v}");
        }
        for (name, v) in &drift.missing {
            println!("  - {name} {v}");
        }
        for (name, v) in &drift.extra {
            println!("  + {name} {v}");
        }
        for (repo, old_s, new_s) in &drift.repos {
            println!("  ~ {repo} package tree: {old_s} → {new_s}");
        }
    }
    std::fs::write(&path, lock::render(&new)?)
        .with_context(|| format!("Couldn't write {}", path.display()))?;
    // Keep the version pins in step, if they're in use. They live in /etc, so
    // without root they're refreshed on the next `bunny switch` instead.
    if root.join(lock::PIN_FILE).exists() && lock::write_pins(root, &new).is_err() {
        println!("  The version pins will catch up on the next `bunny switch`.");
    }
    println!(
        "✓ Saved {} ({} packages, {} package tree{}).",
        path.display(),
        new.packages.len(),
        new.repos.len(),
        if new.repos.len() == 1 { "" } else { "s" }
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn explain_cmd(file: Option<&Path>) -> anyhow::Result<ExitCode> {
    use std::io::{IsTerminal, Read};
    let text = match file {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("Couldn't read {}", path.display()))?,
        None => {
            if std::io::stdin().is_terminal() {
                eprintln!("Paste emerge's message, then press Ctrl+D on an empty line:");
            }
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        }
    };
    let found = explain::explain(&text);
    if found.is_empty() {
        println!(
            "bunny doesn't recognise this message yet.\n\
             If it's an error, please share it so bunny can learn to explain it."
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!();
    for (i, e) in found.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print!("{}", e.render());
    }
    Ok(ExitCode::SUCCESS)
}
