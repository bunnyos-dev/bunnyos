//! Step 1b of a switch: the repositories in `repositories`. New ones are
//! looked up in Gentoo's list, get a repos.conf file, and are downloaded;
//! ones taken out of the config lose bunny's repos.conf file. Comes right
//! after the snapshot and before the package files, so packages from a new
//! repository can be checked and installed in the same switch.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::{cmd, Options, Outcome};
use crate::config::Config;
use crate::overlays;
use crate::plan::Plan;
use crate::portage;

pub(super) fn set_up_repositories(
    config: &Config,
    root: &Path,
    plan: &Plan,
    runner: &mut dyn super::Runner,
    opts: &Options,
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
        download(root, name, runner, opts)?;
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

/// Downloads one repository with `emaint sync`. By default emaint's own
/// output (git's clone lines, "Performing Global Updates", every
/// profiles/updates file) goes to the log and one line is shown instead:
/// `✓ Downloaded guru · 17.5 MB`. `--verbose` (and a test root) shows
/// emaint's output as it comes, like before.
fn download(root: &Path, name: &str, runner: &mut dyn super::Runner, opts: &Options) -> Result<()> {
    let sync = cmd(&["emaint", "sync", "--repo", name]);
    if opts.verbose || runner.dry() {
        println!("\n==> Downloading the {name} repository");
        if !runner.run(&sync)? {
            bail!(
                "Couldn't download the {name} repository (the message is above).\n  \
                 Check that this computer is online, then run this again. Nothing was installed."
            );
        }
        return Ok(());
    }

    let mut progress = crate::progress::Ticker::new();
    progress.step(&format!("Downloading {name}…"), None, None);
    let (ok, said) = runner.run_quiet(&sync, &mut |_| {})?;
    progress.finish();
    let said = crate::explain::strip_ansi(&said);
    let log = super::install::save_log(
        root,
        &opts.stamp,
        &format!("download-{name}"),
        &format!("$ {}\n{said}", sync.join(" ")),
    );
    if !ok || sync_failed(&said) {
        let mut text = format!(
            "Couldn't download the {name} repository. The end of emaint's message:\n\n{}\n\n  \
             Check that this computer is online, then run this again. Nothing was installed.",
            crate::emerge::tail(&said, 10)
        );
        if let Some(log) = &log {
            text.push_str(&format!(
                "\n  emaint's full output is in {}.",
                log.display()
            ));
        }
        bail!("{text}");
    }
    println!(
        "  ✓ Downloaded {name} · {}",
        size(folder_size(&root.join("var/db/repos").join(name)))
    );
    Ok(())
}

/// emaint can exit 0 even when a repository's sync failed; its summary line
/// says so: `Action: sync for repo: guru, returned code = 1`.
fn sync_failed(output: &str) -> bool {
    output.lines().any(|line| {
        line.split_once("returned code = ")
            .and_then(|(_, code)| code.trim().parse::<i32>().ok())
            .is_some_and(|code| code != 0)
    })
}

/// The size of everything in a folder (not following links), in bytes.
fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => folder_size(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// `17.5 MB`, `240 MB`, `1.2 GB`, `512 KB`.
fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1e6 {
        format!("{:.0} KB", b / 1e3)
    } else if b < 100e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b < 1e9 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.1} GB", b / 1e9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_emaints_summary_line() {
        assert!(!sync_failed(
            "Action: sync for repo: guru, returned code = 0\n"
        ));
        assert!(sync_failed(
            "Action: sync for repo: guru, returned code = 1\n"
        ));
        assert!(!sync_failed("Cloning into '/var/db/repos/guru'...\n"));
    }

    #[test]
    fn sizes_read_like_a_file_manager() {
        assert_eq!(size(512_000), "512 KB");
        assert_eq!(size(17_500_000), "17.5 MB");
        assert_eq!(size(240_000_000), "240 MB");
        assert_eq!(size(1_230_000_000), "1.2 GB");
    }

    #[test]
    fn folder_size_adds_up_everything_inside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/one"), [0u8; 100]).unwrap();
        std::fs::write(dir.path().join("a/b/two"), [0u8; 50]).unwrap();
        assert_eq!(folder_size(dir.path()), 150);
        assert_eq!(folder_size(&dir.path().join("missing")), 0);
    }
}
