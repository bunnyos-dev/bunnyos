//! `bunny report`: sending developer-mode recordings.

use std::path::Path;
use std::process::ExitCode;

use super::{confirm, Default};
use crate::dev;

/// `bunny report`: sends the recorded commands that haven't been sent yet.
pub(crate) fn report(config: &Path, dry_run: bool, yes: bool) -> anyhow::Result<ExitCode> {
    let dev = dev::settings(config);
    let dir = dev::sessions_dir();
    let unsent = dev::unsent(&dir);
    if unsent.is_empty() {
        if dev.report == dev::ReportMode::Off {
            println!(
                "Developer mode is off, so nothing is recorded. To turn it on, add this to {}:\n\n  \
                 [dev]\n  report = \"auto\"\n\nThen every bunny command is recorded, and problems are \
                 sent by themselves.",
                config.display()
            );
        } else {
            println!("✓ Nothing to send: every recorded command has been sent.");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let (title, body) = dev::prepare(&dev, &unsent, false)?;
    if dry_run {
        println!(
            "Would send to {} (nothing is sent):\n\n# {title}\n\n{body}",
            dev.repo()
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Ready to send {} recorded command{} to {} ({} KB), with personal details swapped out.",
        unsent.len(),
        if unsent.len() == 1 { "" } else { "s" },
        dev.repo(),
        body.len().div_ceil(1024)
    );
    println!("  `bunny report --dry-run` shows exactly what's sent.");
    if !yes && !confirm("Send it?", Default::Yes)? {
        println!("Nothing sent.");
        return Ok(ExitCode::SUCCESS);
    }
    let url = dev::send_sessions(&dev, &unsent, false)?;
    println!("✓ Sent: {url}");
    Ok(ExitCode::SUCCESS)
}
