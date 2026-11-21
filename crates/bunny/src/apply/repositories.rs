//! Step 1b of a switch: the repositories in `repositories`. New ones are
//! looked up in Gentoo's list, get a repos.conf file, and are downloaded;
//! ones taken out of the config lose bunny's repos.conf file. Comes right
//! after the snapshot and before the package files, so packages from a new
//! repository can be checked and installed in the same switch.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::{cmd, Outcome};
use crate::config::Config;
use crate::overlays;
use crate::plan::Plan;
use crate::portage;

pub(super) fn set_up_repositories(
    config: &Config,
    root: &Path,
    plan: &Plan,
    runner: &mut dyn super::Runner,
    outcome: &mut Outcome,
) -> Result<()> {
    let to_sync: Vec<&String> = config
        .repositories
        .iter()
        .filter(|r| !root.join("var/db/repos").join(r).is_dir())
        .collect();
    if plan.add_repositories.is_empty() && plan.remove_repositories.is_empty() && to_sync.is_empty()
    {
        return Ok(());
    }

    let conf_dir = root.join(portage::REPOS_CONF_DIR);
    if conf_dir.is_file() {
        // Portage reads a single repos.conf file or a folder of them; bunny
        // needs the folder, so the file moves into it unchanged.
        let tmp = root.join("etc/portage/.repos.conf.bunny-move");
        std::fs::rename(&conf_dir, &tmp)?;
        std::fs::create_dir_all(&conf_dir)?;
        std::fs::rename(&tmp, conf_dir.join("00-yours.conf"))?;
        outcome.notes.push(
            "/etc/portage/repos.conf was a single file; it's now repos.conf/00-yours.conf \
             (unchanged), so bunny's repository files can sit next to it."
                .into(),
        );
    }

    for name in &plan.remove_repositories {
        let path = portage::repo_conf_path(root, name);
        if path.exists() {
            let backup = outcome.backup_dir.join("repos.conf");
            std::fs::create_dir_all(&backup)?;
            std::fs::copy(&path, backup.join(path.file_name().unwrap_or_default()))?;
            std::fs::remove_file(&path)
                .with_context(|| format!("Couldn't remove {}", path.display()))?;
        }
        outcome.notes.push(format!(
            "bunny stopped using the {name} repository. Its files are still in \
             /var/db/repos/{name}; delete them with `{} rm -r /var/db/repos/{name}` if you like.",
            crate::security::admin()
        ));
    }

    if !plan.add_repositories.is_empty() {
        let list = if runner.dry() {
            None
        } else {
            Some(overlays::fetch_list(runner)?)
        };
        for name in &plan.add_repositories {
            let uri = match &list {
                None => "<git address from Gentoo's list>".to_string(),
                Some(list) => {
                    let Some(listed) = list.iter().find(|l| &l.name == name) else {
                        bail!(
                            "There's no repository called \"{name}\" in Gentoo's list of repositories.\n  \
                             Check the name at https://repos.gentoo.org and fix `repositories` in your config.\n  \
                             Nothing was installed."
                        );
                    };
                    let Some(git) = &listed.git else {
                        bail!(
                            "The {name} repository can't be downloaded with git, which is how bunny gets\n  \
                             repositories. Nothing was installed."
                        );
                    };
                    println!("\n==> Adding the {name} repository: {}", listed.description);
                    git.clone()
                }
            };
            std::fs::create_dir_all(&conf_dir)?;
            let path = portage::repo_conf_path(root, name);
            std::fs::write(&path, portage::render_repo_conf(name, &uri))
                .with_context(|| format!("Couldn't write {}", path.display()))?;
        }
    }

    let to_sync: Vec<&String> = config
        .repositories
        .iter()
        .filter(|r| !root.join("var/db/repos").join(r).is_dir())
        .collect();
    if !to_sync.is_empty() && !runner.dry() && !root.join("usr/bin/git").exists() {
        bail!(
            "Repositories are downloaded with git, which isn't installed.\n  \
             Add it first: bunny add dev-vcs/git\n  \
             Then run this again. Nothing was installed."
        );
    }
    for name in to_sync {
        println!("\n==> Downloading the {name} repository");
        if !runner.run(&cmd(&["emaint", "sync", "--repo", name]))? {
            bail!(
                "Couldn't download the {name} repository (the message is above).\n  \
                 Check that this computer is online, then run this again. Nothing was installed."
            );
        }
    }

    // Packages from the new repositories can be checked now.
    if plan.check_after_sync && !runner.dry() {
        let unknown = crate::plan::unknown_packages(root, &plan.add_packages);
        if !unknown.is_empty() {
            bail!(
                "These packages aren't in any of your repositories, even the new ones:\n    {}\n  \
                 Check the spelling at https://packages.gentoo.org and fix them in your config.\n  \
                 The repositories were added and stay; nothing was installed.",
                unknown.join("\n    ")
            );
        }
    }
    Ok(())
}
