//! bunny: declarative, reproducible system management for BunnyOS.

mod apply;
mod config;
mod import;
mod init;
mod plan;
mod portage;
mod rollback;

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
        /// Generation name from `bunny generations`
        name: Option<String>,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
        /// System root (for testing on a fake system; commands are only printed)
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
            Ok(ExitCode::SUCCESS)
        }
        Command::Switch { config, yes, root } => switch(&config, yes, &root),
        Command::Generations { root } => generations(&root),
        Command::Rollback { name, yes, root } => rollback_cmd(name, yes, &root),
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

fn switch(config: &Path, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }

    let cfg = Config::load(config)?;
    if report_problems(&cfg, config) != ExitCode::SUCCESS {
        eprintln!("Fix these first, then run `bunny switch` again.");
        return Ok(ExitCode::FAILURE);
    }
    let plan = plan::make(&cfg, root)?;
    print!("{}", plan.render());
    if plan.is_empty() {
        return Ok(ExitCode::SUCCESS);
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
    if !real_system {
        println!("\nThis is a test root, so commands are only printed, not run.");
    }

    if !yes && !confirm("\nApply these changes?")? {
        println!("Nothing was changed.");
        return Ok(ExitCode::SUCCESS);
    }

    let opts = apply::Options {
        yes,
        stamp: timestamp(),
    };
    let outcome = if real_system {
        apply::switch(&cfg, root, &plan, &mut apply::SystemRunner, &opts)?
    } else {
        apply::switch(&cfg, root, &plan, &mut apply::PrintRunner, &opts)?
    };

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
    println!("Saved generations (oldest first):\n");
    for g in &gens {
        let label = if g.is_pre_rollback() {
            "  (your system just before a rollback)"
        } else {
            "  (your system just before a switch)"
        };
        println!("  {}   {}{label}", g.name, g.when());
    }
    println!("\nTo go back to one: bunny rollback <name>");
    Ok(ExitCode::SUCCESS)
}

fn rollback_cmd(name: Option<String>, yes: bool, root: &Path) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let gens = rollback::list(root)?;
    let name = match name {
        Some(name) => name,
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
    if !yes && !confirm("\nRoll back?")? {
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
    println!(
        "  Changed your mind? After rebooting, run: bunny rollback {}",
        outcome.before
    );
    println!(
        "  The replaced system is kept as the subvolume {} until you remove it.",
        outcome.replaced
    );
    Ok(ExitCode::SUCCESS)
}

/// Whether we're running as root (checks who owns our own /proc entry).
fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    use std::io::Write;
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
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
