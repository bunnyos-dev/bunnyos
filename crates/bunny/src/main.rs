//! bunny: declarative, reproducible system management for Bunny Linux.

mod applied;
mod apply;
mod boot;
mod commands;
mod config;
mod config_repo;
mod console_font;
mod desktop;
mod dev;
mod diff;
mod edit;
mod emerge;
mod explain;
mod finish_undo;
mod glsa;
mod helper;
mod import;
mod init;
mod install;
mod interrupt;
mod kernels;
mod lock;
mod notes;
mod overlays;
mod plan;
mod portage;
mod progress;
mod repo;
mod rollback;
mod security;
mod status;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};

use crate::commands::{
    add_cmd, clean_cmd, diff_cmd, explain_cmd, generations, lock_cmd, remove_cmd, report,
    report_problems, rollback_cmd, search_cmd, switch, update_cmd,
};
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
    /// Show emerge's full output as it runs (instead of a summary and progress line)
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Set by bunny itself when it re-runs a command that's already being
    /// recorded (developer mode), so it isn't recorded twice.
    #[arg(long, global = true, hide = true)]
    in_session: bool,
    /// Set by bunny itself when it re-runs a command through sudo or doas,
    /// so a failure as root can be told apart from sudo or doas refusing.
    #[arg(long, global = true, hide = true)]
    elevated: bool,
}

/// Where bunny looks for its config unless told otherwise.
const DEFAULT_CONFIG: &str = "/etc/bunny/bunny.toml";

/// `--verbose`, for every command that runs emerge.
static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Subcommand)]
// Parsed once at start-up; boxing `install`'s options would only add noise.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Show at a glance whether everything is in order (changes nothing)
    Status {
        /// Path to the config
        #[arg(default_value = "/etc/bunny/bunny.toml")]
        config: PathBuf,
        /// System root to look at (for testing against another system)
        #[arg(long, default_value = "/")]
        root: PathBuf,
        /// Exit with code 1 if anything needs a look (for scripts and scheduled checks)
        #[arg(long)]
        check: bool,
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
    /// Run bunny's Portage helper directly and print its JSON (for checks and debugging)
    #[command(hide = true)]
    Helper {
        /// What to ask, e.g. `plan app-misc/tmux`
        args: Vec<String>,
    },
    /// Show your config and your system side by side, and how to fix each difference (changes nothing)
    Diff {
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
    /// Finish an undo from the boot menu: put /boot's kernel back (run at start-up)
    #[command(hide = true)]
    FinishUndo {
        /// Say nothing when there's nothing to do
        #[arg(long)]
        quiet: bool,
        /// System root (for testing on a fake system; commands are only printed)
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
    /// Send recorded commands to the reports repository (developer mode)
    Report {
        /// Show what would be sent, without sending it
        #[arg(long)]
        dry_run: bool,
        /// Don't ask before sending
        #[arg(short, long)]
        yes: bool,
        #[arg(default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },
    /// Send your config folder (/etc/bunny) to a private GitHub repository, with your own GitHub sign-in (shows what it will do and asks first)
    Publish {
        /// The GitHub repository: a name (my-system, the default) or you/my-system
        repo: Option<String>,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
        /// Show what it would do, without changing or sending anything
        #[arg(long)]
        dry_run: bool,
        /// Path to the config (its folder is what's sent)
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
        /// System root (for testing on a fake system; commands are only printed)
        #[arg(long, default_value = "/")]
        root: PathBuf,
    },
    /// Install Bunny Linux on this computer, from the live USB (erases a disk; shows exactly what first)
    Install {
        /// The disk to install on, e.g. /dev/nvme0n1 (asks if not given)
        #[arg(long)]
        disk: Option<String>,
        /// Don't encrypt the disk
        #[arg(long)]
        no_encrypt: bool,
        /// Size of the EFI partition (default 2G)
        #[arg(long, value_name = "SIZE")]
        efi_size: Option<String>,
        /// Also make a swap file on disk of this size, e.g. 8G (default: none)
        #[arg(long, value_name = "SIZE")]
        swap: Option<String>,
        /// No compressed swap in memory (zram, on by default)
        #[arg(long)]
        no_zram: bool,
        /// Set up hibernation (a swapfile as big as your memory, unless --swap says otherwise)
        #[arg(long)]
        hibernate: bool,
        /// Use a second disk, all of it, for /home
        #[arg(long, value_name = "DISK")]
        home_disk: Option<String>,
        /// Start the SSH server at boot
        #[arg(long)]
        ssh: bool,
        /// Stop before the config is applied, so you can edit it first
        #[arg(long)]
        edit: bool,
        /// Install from your own config: a bunny.toml, or a git repository (github:you/my-system)
        #[arg(long, value_name = "FILE OR REPO")]
        config: Option<String>,
        /// Show each step and what it does, without erasing or changing anything (with --verbose: every command)
        #[arg(long)]
        dry_run: bool,
        /// Take the recommended setup without asking (encrypted, zram swap)
        #[arg(long)]
        recommended: bool,
        /// Your user name (asks if not given)
        #[arg(long, value_name = "NAME")]
        user: Option<String>,
        /// A name for this computer (asks if not given)
        #[arg(long, value_name = "NAME")]
        hostname: Option<String>,
        /// Time zone, like America/Toronto (asks if not given)
        #[arg(long, value_name = "ZONE")]
        timezone: Option<String>,
        /// Keyboard layout, like us or de (asks if not given)
        #[arg(long, value_name = "LAYOUT")]
        keymap: Option<String>,
        /// Read the password from standard input (one line) instead of asking
        #[arg(long)]
        password_stdin: bool,
        /// Don't ask to type the disk's name to confirm (needs --disk)
        #[arg(long, requires = "disk")]
        yes: bool,
        /// Print the disks as JSON and stop (for the graphical installer)
        #[arg(long)]
        list_disks: bool,
        /// Read the list of disks from this file (lsblk --json output) instead of lsblk
        #[arg(long, hide = true)]
        lsblk: Option<PathBuf>,
        /// Install onto partitions you made and mounted here yourself (nothing is erased)
        #[arg(long, value_name = "FOLDER",
              conflicts_with_all = ["disk", "no_encrypt", "efi_size", "swap", "no_zram", "hibernate", "home_disk"])]
        mounted: Option<PathBuf>,
        /// Read what's mounted from this file (findmnt --json output) instead of findmnt
        #[arg(long, hide = true)]
        findmnt: Option<PathBuf>,
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
    interrupt::install();
    VERBOSE.store(cli.verbose, std::sync::atomic::Ordering::Relaxed);
    let elevated = cli.elevated;
    if let Some(code) = dev::record_if_on(Path::new(DEFAULT_CONFIG), cli.in_session) {
        return commands::elevated_exit(code, elevated);
    }
    let code = match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("✗ {err}");
            for cause in err.chain().skip(1) {
                eprintln!("  {cause}");
            }
            ExitCode::FAILURE
        }
    };
    commands::elevated_exit(code, elevated)
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.command {
        Command::Report {
            dry_run,
            yes,
            config,
        } => report(&config, dry_run, yes),
        Command::Publish {
            repo,
            yes,
            dry_run,
            config,
            root,
        } => commands::publish(&config, repo.as_deref(), yes, dry_run, &root),
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
        Command::Status {
            config,
            root,
            check,
        } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let status = status::gather(&config, &root, now);
            print!("{}", status.render());
            Ok(if check && !status.all_good() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Diff { config, root } => diff_cmd(&config, &root),
        Command::Helper { args } => {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let cmd = helper::command(&args);
            let status = std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .status()
                .context("Couldn't start python3, which the Portage helper needs")?;
            Ok(if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Switch { config, yes, root } => switch(&config, yes, &root),
        Command::Generations { root } => generations(&root),
        Command::Rollback { name, yes, root } => rollback_cmd(name, yes, &root),
        Command::FinishUndo { quiet, root } => commands::finish_undo_cmd(quiet, &root),
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
        Command::Install {
            disk,
            no_encrypt,
            efi_size,
            swap,
            no_zram,
            hibernate,
            home_disk,
            ssh,
            edit,
            config,
            dry_run,
            recommended,
            user,
            hostname,
            timezone,
            keymap,
            password_stdin,
            yes,
            list_disks,
            lsblk,
            mounted,
            findmnt,
        } => commands::install_cmd(commands::InstallArgs {
            disk,
            no_encrypt,
            efi_size,
            swap,
            no_zram,
            hibernate,
            home_disk,
            ssh,
            edit,
            config,
            dry_run,
            recommended,
            user,
            hostname,
            timezone,
            keymap,
            password_stdin,
            yes,
            list_disks,
            lsblk,
            mounted,
            findmnt,
        }),
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
