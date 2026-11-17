//! bunny: declarative, reproducible system management for BunnyOS.

mod apply;
mod config;
mod edit;
mod explain;
mod import;
mod init;
mod lock;
mod notes;
mod plan;
mod portage;
mod repo;
mod rollback;
mod status;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "bunny",
    version,
    about = "Manage your whole system from one config file"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show at a glance whether everything is in order (changes nothing)
    Status {
        /// Path to the config
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// System root to look at (for testing against another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Check bunny.toml for mistakes and explain how to fix them
    Check {
        /// Path to the config
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
    },
    /// Show what would change to make this system match the config (changes nothing)
    Plan {
        /// Path to the config
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// System root to compare against (for testing against another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Make this system match the config (shows the changes and asks first)
    Switch {
        /// Path to the config
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// Don't ask before applying or installing (removing packages always asks)
        #[arg(short, long)]
        yes: bool,
        /// System root to change. Anything but / only writes files and prints
        /// the commands it would run (for testing on a fake system).
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// List the saved generations you can roll back to
    Generations {
        /// System root (for testing against another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Put the system back to a saved generation (the latest one if no name is given)
    Rollback {
        /// Number or name from `bunny generations`, or `list` to show them
        name: Option<String>,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
        /// System root (for testing on a fake system; commands are only printed)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Remove old generations and leftovers from rollbacks (shows them and asks first)
    Clean {
        /// How many of the newest generations to keep
        #[arg(long, default_value_t = 5)]
        keep: usize,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
        /// System root (for testing on a fake system; commands are only printed)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Sync the package tree, update everything, and record the new versions in bunny.lock
    Update {
        /// Path to the config
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// Don't ask first (removing packages always asks)
        #[arg(short, long)]
        yes: bool,
        /// System root (for testing on a fake system; commands are only printed)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Find packages by name or description
    Search {
        /// What to look for, e.g. "browser" or "firefox"
        #[arg(required = true)]
        query: Vec<String>,
        /// System root (for testing against another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Add packages to your config and install them (shows the changes and asks first)
    Add {
        /// Package names, short (firefox) or full (www-client/firefox)
        #[arg(required = true)]
        names: Vec<String>,
        /// Path to the config
        #[arg(long, default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// Don't ask before applying or installing
        #[arg(short, long)]
        yes: bool,
        /// System root (for testing on a fake system; commands are only printed)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Remove packages from your config and the system (shows the changes and asks first)
    Remove {
        /// Package names, short (firefox) or full (www-client/firefox)
        #[arg(required = true)]
        names: Vec<String>,
        /// Path to the config
        #[arg(long, default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// Don't ask before applying (removing packages from the system always asks)
        #[arg(short, long)]
        yes: bool,
        /// System root (for testing on a fake system; commands are only printed)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Explain an emerge message in plain English (paste it in, or give a file)
    Explain {
        /// A file with emerge's output. Without it, paste the message and press Ctrl+D.
        file: Option<PathBuf>,
    },
    /// Record the exact versions installed now in bunny.lock, next to the config
    Lock {
        /// Path to the config (the lock is written next to it)
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// System root to read (for testing against another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Write a bunny.toml that describes this system as it is now (changes nothing)
    Import {
        /// Save to this file instead of printing it
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Overwrite the output file if it already exists
        #[arg(long)]
        force: bool,
        /// System root to read (for importing another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("✗ {err}");
            for cause in err.chain().skip(1) {
                eprintln!("  {cause}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.command {
        Command::Check { config } => {
            let cfg = Config::load(&config)?;
            Ok(report_problems(&cfg, &config))
        }
        Command::Plan { config, root } => {
            let cfg = Config::load(&config)?;
            if report_problems(&cfg, &config) != ExitCode::SUCCESS {
                eprintln!("Fix these first, then run `bunny plan` again.");
                return Ok(ExitCode::FAILURE);
            }
            let plan = plan::make(&cfg, &root)?;
            print!("{}", plan.render());
            let lock_path = lock::path_for(&config);
            if lock_path.exists() {
                let locked = lock::load(&lock_path)?;
                let now = lock::from_system(&root)?;
                println!();
                print!("{}", lock::compare(&locked, &now).render());
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Status { config, root } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            print!("{}", status::gather(&config, &root, now).render());
            Ok(ExitCode::SUCCESS)
        }
        Command::Switch { config, yes, root } => switch(&config, yes, &root),
        Command::Generations { root } => generations(&root),
        Command::Rollback { name, yes, root } => rollback_cmd(name, yes, &root),
        Command::Clean { keep, yes, root } => clean_cmd(keep, yes, &root),
        Command::Lock { config, root } => lock_cmd(&config, &root),
        Command::Explain { file } => explain_cmd(file.as_deref()),
        Command::Search { query, root } => search_cmd(&query.join(" "), &root),
        Command::Add {
            names,
            config,
            yes,
            root,
        } => add_cmd(&names, &config, yes, &root),
        Command::Remove {
            names,
            config,
            yes,
            root,
        } => remove_cmd(&names, &config, yes, &root),
        Command::Update { config, yes, root } => update_cmd(&config, yes, &root),
        Command::Import {
            output,
            force,
            root,
        } => {
            let imported = import::from_system(&root)?;
            let text = imported.render();
            match output {
                Some(path) => {
                    if path.exists() && !force {
                        anyhow::bail!(
                            "{} already exists, so it wasn't overwritten.\n  \
                             Pick another name, or add --force to replace it.",
                            path.display()
                        );
                    }
                    std::fs::write(&path, text)
                        .with_context(|| format!("Couldn't write {}", path.display()))?;
                    eprintln!("✓ Saved to {}.", path.display());
                    eprintln!("{}", imported.summary());
                    eprintln!(
                        "Next: review it, then run `bunny plan {}`. It should say nothing needs to change.",
                        path.display()
                    );
                }
                None => {
                    print!("{text}");
                    eprintln!("\n{}", imported.summary());
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// How a switch ended, so `add` and `remove` know whether to keep their edit.
#[derive(Debug, PartialEq)]
enum Switched {
    /// Changes were applied (or at least started).
    Applied,
    /// The system already matched.
    NothingToDo,
    /// Stopped before changing anything (problems found, or cancelled).
    NotApplied(ExitCode),
}

fn switch(config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    Ok(match switch_inner(config, yes, root, false)? {
        Switched::Applied | Switched::NothingToDo => ExitCode::SUCCESS,
        Switched::NotApplied(code) => code,
    })
}

/// The switch itself, for a caller that's already root.
fn switch_inner(
    config: &Path,
    yes: bool,
    root: &Path,
    removal_requested: bool,
) -> anyhow::Result<Switched> {
    let real_system = root == Path::new("/");

    let cfg = Config::load(config)?;
    if report_problems(&cfg, config) != ExitCode::SUCCESS {
        eprintln!("Fix these first, then run `bunny switch` again.");
        return Ok(Switched::NotApplied(ExitCode::FAILURE));
    }
    let mut plan = plan::make(&cfg, root)?;
    plan.removal_requested = removal_requested;
    print!("{}", plan.render());
    if !plan.unknown_packages.is_empty() {
        eprintln!(
            "Nothing was changed. Fix the package names in {}, then run `bunny switch` again.",
            config.display()
        );
        return Ok(Switched::NotApplied(ExitCode::FAILURE));
    }
    let lock_path = lock::path_for(config);
    if plan.is_empty() {
        for message in lock::ensure(root, &lock_path)? {
            println!("{message}");
        }
        return Ok(Switched::NothingToDo);
    }

    let foreign = apply::foreign_package_use_files(root);
    if !foreign.is_empty() {
        println!(
            "\nbunny will take over your USE flag settings. These package.use files will be\n\
             moved to a backup and replaced by one generated from your config:\n  {}\n\
             Lines bunny can't manage yet are kept as they are.",
            foreign.join(", ")
        );
    }
    if lock_path.exists() {
        println!(
            "\nPackages stay at the versions in {}. To move to newer versions, use `bunny update`.",
            lock_path.display()
        );
    }
    if !real_system {
        println!("\nThis is a test root, so commands are only printed, not run.");
    }

    if !yes
        && !confirm(
            "\nApply these changes?",
            // Enter only means yes when a snapshot makes this undoable.
            if root.join(".snapshots").is_dir() {
                Default::Yes
            } else {
                println!(
                    "\nNote: there's no /.snapshots folder, so this switch can't be rolled back."
                );
                Default::No
            },
        )?
    {
        println!("Nothing was changed.");
        return Ok(Switched::NotApplied(ExitCode::SUCCESS));
    }

    let opts = apply::Options {
        yes,
        stamp: timestamp(),
        update_all: false,
        color: std::io::IsTerminal::is_terminal(&std::io::stdout()),
    };
    let mut runner = runner_for(real_system);
    let (outcome, change) = lock::switch(&cfg, root, &plan, &lock_path, runner.as_mut(), &opts)?;

    println!("\n✓ This system now matches {}.", config.display());
    if let (true, Some(snapshot)) = (real_system, &outcome.snapshot) {
        println!(
            "  The system before this switch is saved in {}.",
            snapshot.display()
        );
    }
    println!(
        "  Old files are backed up in {}.",
        outcome.backup_dir.display()
    );
    for note in &outcome.notes {
        println!("  Note: {note}");
    }
    report_lock_change(&change);
    print_tips(&outcome.tips);
    Ok(Switched::Applied)
}

fn print_tips(tips: &[String]) {
    if tips.is_empty() {
        return;
    }
    println!("\nFrom emerge:");
    for tip in tips {
        for line in tip.lines() {
            println!("  {line}");
        }
    }
}

fn runner_for(real_system: bool) -> Box<dyn apply::Runner> {
    if real_system {
        Box::new(apply::SystemRunner)
    } else {
        Box::new(apply::PrintRunner)
    }
}

fn report_lock_change(change: &lock::LockChange) {
    if change.created {
        println!(
            "  Created {}: the exact versions installed now.",
            change.path.display()
        );
        return;
    }
    let d = &change.drift;
    if d.is_empty() {
        println!("  {} is unchanged.", change.path.display());
        return;
    }
    println!("  Updated {}:", change.path.display());
    for (name, old, new) in &d.changed {
        println!("    ~ {name}: {old} → {new}");
    }
    for (name, v) in &d.extra {
        println!("    + {name} {v}");
    }
    for (name, v) in &d.missing {
        println!("    - {name} {v}");
    }
    for (repo, old, new) in &d.repos {
        println!("    ~ {repo} package tree: {old} → {new}");
    }
}

fn update_cmd(config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let cfg = Config::load(config)?;
    if report_problems(&cfg, config) != ExitCode::SUCCESS {
        eprintln!("Fix these first, then run `bunny update` again.");
        return Ok(ExitCode::FAILURE);
    }
    println!(
        "\nThis will sync the Gentoo package tree, update everything to the newest\n\
         versions, and record them in {}.",
        lock::path_for(config).display()
    );
    let undoable = root.join(".snapshots").is_dir();
    if undoable {
        println!("A snapshot is taken first, so you can roll back if you don't like the result.");
    } else {
        println!("Note: there's no /.snapshots folder, so this update can't be rolled back.");
    }
    if !real_system {
        println!("\nThis is a test root, so commands are only printed, not run.");
    }
    let default = if undoable { Default::Yes } else { Default::No };
    if !yes && !confirm("\nUpdate now?", default)? {
        println!("Nothing was changed.");
        return Ok(ExitCode::SUCCESS);
    }
    let opts = apply::Options {
        yes,
        stamp: timestamp(),
        update_all: true,
        color: std::io::IsTerminal::is_terminal(&std::io::stdout()),
    };
    let mut runner = runner_for(real_system);
    let (outcome, change) =
        lock::update(&cfg, root, &lock::path_for(config), runner.as_mut(), &opts)?;

    println!("\n✓ Updated.");
    if let (true, Some(snapshot)) = (real_system, &outcome.snapshot) {
        println!(
            "  The system before this update is saved in {}.",
            snapshot.display()
        );
    }
    for note in &outcome.notes {
        println!("  Note: {note}");
    }
    report_lock_change(&change);
    print_tips(&outcome.tips);
    Ok(ExitCode::SUCCESS)
}

/// Re-runs bunny through doas when changing the real system without root,
/// so nobody has to type `doas bunny`. Returns the exit code if it did.
fn elevate(real_system: bool) -> anyhow::Result<Option<ExitCode>> {
    if !real_system || is_root() {
        return Ok(None);
    }
    let status = std::process::Command::new("doas")
        .arg(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        .status()
        .context("bunny needs to run as root to change the system, and couldn't start doas")?;
    Ok(Some(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }))
}

fn generations(root: &Path) -> anyhow::Result<ExitCode> {
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
                    println!("\nReboot to finish the rollback: doas reboot");
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

fn clean_cmd(keep: usize, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
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

fn rollback_cmd(name: Option<String>, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
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

    // Check this before asking anything.
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")
        .context("Couldn't read the list of mounted filesystems")?;
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

    println!("\n✓ Done. Reboot to start the restored system: doas reboot");
    // Refer to the undo point by its number in `bunny generations`.
    let undo = rollback::list(root)?
        .iter()
        .position(|g| g.name == outcome.before)
        .map(|i| (i + 1).to_string())
        .unwrap_or_else(|| outcome.before.clone());
    println!("  Changed your mind? After rebooting, run: bunny rollback {undo}");
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

fn lock_cmd(config: &Path, root: &Path) -> anyhow::Result<ExitCode> {
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

fn explain_cmd(file: Option<&Path>) -> anyhow::Result<ExitCode> {
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

fn search_cmd(query: &str, root: &Path) -> anyhow::Result<ExitCode> {
    let packages = repo::all(root);
    if packages.is_empty() {
        anyhow::bail!(
            "Couldn't find any package repositories in {}.\n  \
             Sync them first: doas emerge --sync",
            root.join("var/db/repos").display()
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
fn resolve_all(root: &Path, names: &[String]) -> anyhow::Result<Option<Vec<String>>> {
    let packages = repo::all(root);
    if packages.is_empty() {
        anyhow::bail!(
            "Couldn't find any package repositories in {}.\n  \
             Sync them first: doas emerge --sync",
            root.join("var/db/repos").display()
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

fn add_cmd(names: &[String], config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
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

fn remove_cmd(names: &[String], config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
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

fn read_config_text(config: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(config).with_context(|| {
        format!(
            "Couldn't read {}.\n  If you don't have a config yet, make one from this system: bunny import -o {}",
            config.display(),
            config.display()
        )
    })
}

/// Saves the edited config and switches. If nothing gets applied (problems,
/// or the user says no), the old config is put back.
fn edit_then_switch(
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

/// Whether we're running as root (checks who owns our own /proc entry).
fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// What pressing Enter means at a yes/no question.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Default {
    /// For changes that can be undone (a snapshot is taken first).
    Yes,
    /// For changes that can't be undone (no snapshot, erasing a disk).
    No,
}

/// Asks a yes/no question. Anything that isn't clearly yes or no (like a
/// pasted command) is asked again instead of being guessed.
fn confirm(question: &str, default: Default) -> anyhow::Result<bool> {
    use std::io::Write;
    let hint = match default {
        Default::Yes => "[Y/n]",
        Default::No => "[y/N]",
    };
    print!("{question} {hint} ");
    loop {
        std::io::stdout().flush()?;
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer)? == 0 {
            return Ok(false); // no more input: never assume yes
        }
        match parse_answer(&answer, default) {
            Some(false) if default == Default::No && answer.trim().is_empty() => {
                println!(
                    "(Enter means no here, because this can't be undone. \
                     To go ahead, run it again and type y.)"
                );
                return Ok(false);
            }
            Some(yes) => return Ok(yes),
            None => print!(
                "Didn't catch that (got \"{}\"). Type y to continue or n to cancel: ",
                answer.trim()
            ),
        }
    }
}

/// `Some(true)` for yes, `Some(false)` for no, Enter means `default`,
/// `None` for anything else.
fn parse_answer(answer: &str, default: Default) -> Option<bool> {
    match answer.trim().to_lowercase().as_str() {
        "" => Some(default == Default::Yes),
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// `YYYYMMDD-HHMMSS` in UTC, for naming snapshots and backups.
fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Prints any problems in the config. Returns failure if there were some.
fn report_problems(cfg: &Config, path: &std::path::Path) -> ExitCode {
    let problems = cfg.problems();
    if problems.is_empty() {
        println!("✓ {} looks good.", path.display());
        return ExitCode::SUCCESS;
    }
    let count = problems.len();
    eprintln!(
        "✗ Found {count} problem{} in {}:\n",
        if count == 1 { "" } else { "s" },
        path.display()
    );
    for problem in problems {
        eprintln!("  • {problem}\n");
    }
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::{parse_answer, Default};

    #[test]
    fn answers() {
        assert_eq!(parse_answer("y\n", Default::No), Some(true));
        assert_eq!(parse_answer("YES", Default::No), Some(true));
        assert_eq!(parse_answer("n", Default::Yes), Some(false));
        assert_eq!(
            parse_answer("doas rc-service cronie status", Default::Yes),
            None
        );
    }

    #[test]
    fn enter_means_the_default() {
        assert_eq!(parse_answer("\n", Default::Yes), Some(true));
        assert_eq!(parse_answer("\n", Default::No), Some(false));
    }
}
