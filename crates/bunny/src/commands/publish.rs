//! `bunny publish`: send your config folder to a private GitHub repository,
//! with your own GitHub sign-in.

use std::path::Path;
use std::process::ExitCode;

use anyhow::bail;

use super::{confirm, elevate, Default};
use crate::apply::{self, Runner};
use crate::config_repo::{self, Person};

/// Reads for real, but only prints what it would change (`--dry-run`).
struct LookOnly(apply::SystemRunner);

impl Runner for LookOnly {
    fn run(&mut self, cmd: &[String]) -> anyhow::Result<bool> {
        println!("  (would run) {}", cmd.join(" "));
        Ok(true)
    }
    fn output(&mut self, cmd: &[String]) -> anyhow::Result<Option<String>> {
        self.0.output(cmd)
    }
    fn dry(&self) -> bool {
        true
    }
}

/// `bunny publish [repository]`.
pub(crate) fn publish(
    config: &Path,
    repo: Option<&str>,
    yes: bool,
    dry_run: bool,
    root: &Path,
) -> anyhow::Result<ExitCode> {
    let real_system = root == Path::new("/");
    if let Some(code) = elevate(real_system)? {
        return Ok(code);
    }
    let Some(person) = Person::invoking(root) else {
        bail!(
            "bunny publish uses your own GitHub sign-in, not root's, so run it as yourself:\n\n    \
             bunny publish\n\n  It asks for your password when it needs to change /etc/bunny."
        );
    };
    let dir = root.join(
        config
            .parent()
            .unwrap_or(Path::new("/etc/bunny"))
            .strip_prefix("/")
            .unwrap_or(config),
    );
    if !dir.join("bunny.toml").is_file() && !dir.join(".git").exists() {
        bail!(
            "There's no config in {}, so there's nothing to publish.",
            dir.display()
        );
    }
    let host = crate::config::this_host(root).unwrap_or_else(|| "computer".into());
    let mut runner: Box<dyn Runner> = if !real_system {
        Box::new(apply::PrintRunner)
    } else if dry_run {
        Box::new(LookOnly(apply::SystemRunner))
    } else {
        Box::new(apply::SystemRunner)
    };
    let gh_installed = root.join("usr/bin/gh").exists();
    let plan = config_repo::look(&dir, &person, &host, repo, gh_installed, runner.as_mut())?;
    print!("{}", plan.preview());
    if !runner.dry() {
        let default = if plan.public {
            Default::No
        } else {
            Default::Yes
        };
        if !yes && !confirm("\nGo ahead?", default)? {
            println!("Nothing was changed.");
            return Ok(ExitCode::SUCCESS);
        }
    }
    plan.carry_out(runner.as_mut())?;
    if runner.dry() {
        println!("\n✓ Dry run finished. Nothing was changed.");
    } else {
        println!(
            "\n✓ Your config is on GitHub: https://github.com/{}\n  \
             After a change, run bunny publish again to send it.\n  \
             To install the same system on another computer, start it from the USB and run:\n    \
             bunny install --config github:{}",
            plan.repo, plan.repo
        );
    }
    Ok(ExitCode::SUCCESS)
}
