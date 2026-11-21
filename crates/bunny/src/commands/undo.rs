//! `bunny generations`, `rollback` and `clean`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;

use super::{confirm, elevate, timestamp, Default};
use crate::{apply, kernels, notes, rollback, security};

pub(crate) fn generations(root: &Path) -> anyhow::Result<ExitCode> {
    let gens = rollback::list(root)?;
    if gens.is_empty() {
        println!("No generations yet. One is saved every time you run `bunny switch`.");
        return Ok(ExitCode::SUCCESS);
    }
    let number = |name: &str| {
        gens.iter()
            .position(|g| g.name == name)
            .map(|i| format!("#{}", i + 1))
            .unwrap_or_else(|| name.to_string())
    };
    let state = notes::read_state(root);
    let running = match &state {
        Some(notes::State {
            last: notes::Kind::Rollback,
            to: Some(to),
            ..
        }) => Some(to.as_str()),
        _ => None,
    };

    println!("Saved generations (oldest first):\n");
    for (i, g) in gens.iter().enumerate() {
        let what = match notes::read_note(root, &g.name) {
            Some(n) if n.kind == notes::Kind::Rollback => match &n.to {
                Some(to) => format!("before rolling back to {}", number(to)),
                None => "before a rollback".to_string(),
            },
            Some(n) if !n.summary.is_empty() => format!("before a switch: {}", n.summary),
            Some(_) => "before a switch".to_string(),
            None if g.is_pre_rollback() => "before a rollback".to_string(),
            None => "before a switch".to_string(),
        };
        let marker = if running == Some(g.name.as_str()) {
            "   ← running a copy of this"
        } else {
            ""
        };
        println!("  {:>2}. {}   {what}{marker}", i + 1, g.when());
        println!("      {}", g.name);
    }

    match &state {
        Some(s) if s.last == notes::Kind::Switch => println!(
            "\nRunning now: your system after the switch at {}.",
            rollback::Generation {
                name: format!("bunny-{}", s.at)
            }
            .when()
        ),
        _ => {}
    }
    if root == Path::new("/") {
        if let Ok(info) = std::fs::read_to_string("/proc/self/mountinfo") {
            if let Ok((_, subvol)) = rollback::root_mount(&info) {
                if subvol.starts_with("@replaced-") {
                    println!(
                        "\nReboot to finish the rollback: {} reboot",
                        security::admin()
                    );
                }
            }
        }
    }
    println!(
        "\nTo go back to one: bunny rollback <number>   (for example: bunny rollback {})",
        gens.len()
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn clean_cmd(keep: usize, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let keep = keep.max(1);
    let (device, running) = if real_system {
        let info = std::fs::read_to_string("/proc/self/mountinfo")
            .context("Couldn't read the list of mounted filesystems")?;
        rollback::root_mount(&info)?
    } else {
        ("test-device".to_string(), "@".to_string())
    };

    let mut runner: Box<dyn apply::Runner> = if real_system {
        Box::new(apply::SystemRunner)
    } else {
        Box::new(apply::PrintRunner)
    };
    let top = root.join("run/bunny/top");
    std::fs::create_dir_all(&top)?;
    let top_str = top.to_string_lossy().into_owned();
    let run = |runner: &mut Box<dyn apply::Runner>, parts: &[&str]| {
        runner.run(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };
    if !run(
        &mut runner,
        &["mount", "-o", "subvolid=5", &device, &top_str],
    )? {
        anyhow::bail!("Couldn't open the btrfs volume, so nothing was removed.");
    }

    let result = (|| -> anyhow::Result<ExitCode> {
        let top_names: Vec<String> = std::fs::read_dir(&top)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let gens = rollback::list(root)?;
        let running_copy = match notes::read_state(root) {
            Some(notes::State {
                last: notes::Kind::Rollback,
                to,
                ..
            }) => to,
            _ => None,
        };
        let mut plan =
            rollback::clean_plan(&gens, keep, &top_names, &running, running_copy.as_deref());
        for name in &plan.replaced {
            let nested = rollback::nested_subvolumes(runner.as_mut(), &top, name)?;
            if !nested.is_empty() {
                plan.nested.insert(name.clone(), nested);
            }
        }
        if plan.is_empty() {
            println!(
                "✓ Nothing to clean. You have {} generation(s), and bunny keeps the newest {keep}.",
                gens.len()
            );
            return Ok(ExitCode::SUCCESS);
        }
        if !plan.generations.is_empty() {
            println!("Old generations to remove (keeping the newest {keep}):");
            for name in &plan.generations {
                let g = rollback::Generation { name: name.clone() };
                println!("  - {}   {name}", g.when());
            }
        }
        if !plan.replaced.is_empty() {
            println!("Old systems left behind by rollbacks:");
            for name in &plan.replaced {
                match plan.nested.get(name) {
                    Some(inner) => {
                        let inside: Vec<_> = inner
                            .iter()
                            .map(|p| format!("/{}", p.split_once('/').map_or(p.as_str(), |x| x.1)))
                            .collect();
                        println!("  - {name}   (with {} inside it)", inside.join(", "));
                    }
                    None => println!("  - {name}"),
                }
            }
        }
        if !real_system {
            println!("\nThis is a test root, so commands are only printed, not run.");
        }
        println!("\nThis can't be undone: you won't be able to roll back to these anymore.");
        if !yes && !confirm("Remove them?", Default::No)? {
            println!("Nothing was removed.");
            return Ok(ExitCode::SUCCESS);
        }
        let failed = rollback::clean(root, &top, &plan, runner.as_mut())?;
        // Kernel copies on /boot that no remaining generation uses.
        if real_system {
            if let Err(e) = kernels::tidy(root) {
                println!("Note: couldn't remove old kernel copies from /boot ({e:#}).");
            }
        }
        if failed.is_empty() {
            println!("✓ Removed.");
        } else {
            println!("Some couldn't be removed: {}", failed.join(", "));
        }
        Ok(ExitCode::SUCCESS)
    })();

    run(&mut runner, &["umount", &top_str])?;
    result
}

pub(crate) fn rollback_cmd(
    name: Option<String>,
    yes: bool,
    root: &Path,
) -> anyhow::Result<ExitCode> {
    // Listing doesn't need root.
    if name.as_deref() == Some("list") {
        return generations(root);
    }
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let gens = rollback::list(root)?;
    let name = match name {
        Some(input) => rollback::resolve(&gens, &input).unwrap_or(input),
        None => match gens.iter().rev().find(|g| !g.is_pre_rollback()) {
            Some(g) => g.name.clone(),
            None => anyhow::bail!(
                "There are no generations to roll back to yet.\n  \
                 One is saved every time you run `bunny switch`."
            ),
        },
    };
    let when = gens
        .iter()
        .find(|g| g.name == name)
        .map(|g| g.when())
        .unwrap_or_default();

    // Check this before asking anything. A test root brings its own.
    let mountinfo_path = if root == Path::new("/") {
        PathBuf::from("/proc/self/mountinfo")
    } else {
        root.join("proc/self/mountinfo")
    };
    let mountinfo = std::fs::read_to_string(&mountinfo_path).with_context(|| {
        format!(
            "Couldn't read the list of mounted filesystems ({})",
            mountinfo_path.display()
        )
    })?;
    rollback::root_mount(&mountinfo)?;
    if !gens.iter().any(|g| g.name == name) {
        anyhow::bail!(
            "There's no generation called \"{name}\".\n  \
             Run `bunny generations` to see the ones you have."
        );
    }

    println!("This will put your system back the way it was at {when} ({name}).");
    println!("Your files in /home, logs and caches are not touched.");
    println!("The system as it is now will be saved first, so you can undo this.");
    if kernels::separate_boot(&mountinfo) {
        match kernels::restore_plan(root, &name) {
            Some(plan) if plan.versions_now != plan.versions_then => println!(
                "The kernel goes back too: {} → {}.",
                plan.versions_now.join(", "),
                plan.versions_then.join(", ")
            ),
            Some(plan) if !plan.is_empty() => {
                println!("The kernel's startup image in /boot goes back too.")
            }
            Some(_) => {}
            None => println!(
                "/boot keeps the kernel it has now (this version was saved before bunny\n\
                 kept a copy of each kernel)."
            ),
        }
    }
    if !real_system {
        println!("\nThis is a test root, so commands are only printed, not run.");
    }
    if !yes && !confirm("\nRoll back?", Default::Yes)? {
        println!("Nothing was changed.");
        return Ok(ExitCode::SUCCESS);
    }

    let stamp = timestamp();
    let outcome = if real_system {
        rollback::rollback(root, &name, &mountinfo, &mut apply::SystemRunner, &stamp)?
    } else {
        rollback::rollback(root, &name, &mountinfo, &mut apply::PrintRunner, &stamp)?
    };

    println!(
        "\n✓ Done. Reboot to start the restored system: {} reboot",
        security::admin()
    );
    // Refer to the undo point by its number in `bunny generations`.
    let undo = rollback::list(root)?
        .iter()
        .position(|g| g.name == outcome.before)
        .map(|i| (i + 1).to_string())
        .unwrap_or_else(|| outcome.before.clone());
    println!("  Changed your mind? After rebooting, run: bunny rollback {undo}");
    if let Some(restored) = &outcome.config_restored {
        match &restored.host {
            Some(host) => println!(
                "  {} and {} are put back as they were then,\n  so they match the restored system.",
                restored.config.display(),
                host.display()
            ),
            None => println!(
                "  {} is put back as it was then, so it matches the restored system.",
                restored.config.display()
            ),
        }
    }
    if let Some(done) = &outcome.kernel_restored {
        if done.versions_now != done.versions_then {
            println!(
                "  /boot has kernel {} again, as it did then.",
                done.versions_then.join(", ")
            );
        } else {
            println!("  /boot's kernel files are back as they were then.");
        }
    }
    if let Some(note) = &outcome.kernel_note {
        println!("  Note: {note}");
    }
    if outcome.undo_entry_dropped {
        println!(
            "  The boot menu's \"undo the last change\" entry is gone until your next change,\n  \
             so picking it can't take you further back by accident."
        );
    }
    if !outcome.carried.is_empty() {
        println!(
            "  Kept as they are (separate subvolumes, like /home): {}",
            outcome.carried.join(", ")
        );
    }
    if !outcome.left_behind.is_empty() {
        println!(
            "  Note: couldn't bring over {}. Their contents are still in {} until you remove it.",
            outcome.left_behind.join(", "),
            outcome.replaced
        );
    }
    println!(
        "  The replaced system is kept as the subvolume {} until you remove it (bunny clean).",
        outcome.replaced
    );
    Ok(ExitCode::SUCCESS)
}
