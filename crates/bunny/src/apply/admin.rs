//! Admin rights from `[security]`: the sudo or doas file and the wheel group.

use anyhow::{bail, Context, Result};

use super::{cmd, Outcome, Runner};
use crate::plan::Plan;

/// Writes the sudo/doas file and adds admins to `wheel`. The file is checked
/// with the tool's own checker first; if it fails, nothing is replaced and the
/// switch stops before depclean, so the tool you have keeps working.
pub(super) fn apply_security(
    plan: &Plan,
    runner: &mut dyn Runner,
    outcome: &Outcome,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (Some(privilege), Some((path, text))) = (plan.security.privilege, &plan.security.file)
    else {
        return add_admins(plan, runner);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Couldn't create {}", parent.display()))?;
    }
    let mut staged = path.clone().into_os_string();
    staged.push(".bunny-new");
    let staged = std::path::PathBuf::from(staged);
    std::fs::write(&staged, text)
        .with_context(|| format!("Couldn't write {}", staged.display()))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(privilege.mode()))?;

    let (ok, said) = runner.run_quiet(&privilege.check_command(&staged), &mut |_| {})?;
    if !ok {
        let _ = std::fs::remove_file(&staged);
        bail!(
            "{} rejected the file bunny generated for it, so it wasn't put in place,\n  \
             and no packages were removed. Your current admin setup still works.\n  \
             It said: {}\n  \
             Please report this: https://github.com/bunnyos-dev/bunnyos/issues",
            privilege.command(),
            said.trim()
        );
    }
    if path.exists() {
        let name = path.file_name().unwrap_or_default();
        std::fs::copy(path, outcome.backup_dir.join(name))
            .with_context(|| format!("Couldn't back up {}", path.display()))?;
    }
    std::fs::rename(&staged, path)
        .with_context(|| format!("Couldn't put {} in place", path.display()))?;
    add_admins(plan, runner)
}

pub(super) fn add_admins(plan: &Plan, runner: &mut dyn Runner) -> Result<()> {
    for user in &plan.security.add_to_wheel {
        if !runner.run(&cmd(&["gpasswd", "-a", user, "wheel"]))? {
            bail!("Couldn't add {user} to the wheel group.");
        }
    }
    Ok(())
}
