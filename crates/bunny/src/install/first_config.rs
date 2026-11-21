//! The new system's config, and its first `bunny switch`.
//!
//! The config is either the starter one bunny writes from your answers, or
//! your own (`--config`: a file, or a git repository with `bunny.toml` at
//! its top). Your own is checked before any disk is touched. It goes in
//! `/etc/bunny/` with this computer's host file, and the first switch and
//! lock run inside the new system, so it starts already matching its config
//! (generation 1, "as installed").

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::layout::{Choices, Uuids};
use super::starter;
use super::system::Account;
use crate::apply::Runner;
use crate::config::{self, Config};

/// Where the config goes in the new system, relative to its root.
pub const DIR: &str = "etc/bunny";
/// Where `--config` repositories are cloned before the install.
const CLONE_DIR: &str = "/run/bunny-install-config";

/// Where the config comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Written by bunny from your answers.
    Starter,
    /// Your own `bunny.toml`.
    File(PathBuf),
    /// A git repository with `bunny.toml` at the top, already cloned here.
    Repo { url: String, dir: PathBuf },
}

/// A `--config` value that's a git repository, as a URL git understands.
/// `github:you/my-system` is short for `https://github.com/you/my-system`.
pub fn git_url(given: &str) -> Option<String> {
    if let Some(rest) = given.strip_prefix("github:") {
        let rest = rest.trim_matches('/');
        return (rest.split('/').count() == 2 && !rest.contains(' '))
            .then(|| format!("https://github.com/{rest}"));
    }
    let looks_like_git = given.starts_with("https://")
        || given.starts_with("http://")
        || given.starts_with("ssh://")
        || given.starts_with("git@")
        || given.ends_with(".git");
    (looks_like_git && !Path::new(given).exists()).then(|| given.to_string())
}

/// Gets `--config` ready and checks it, before anything is erased.
pub fn prepare(given: Option<&str>, runner: &mut dyn Runner) -> Result<Source> {
    let Some(given) = given else {
        return Ok(Source::Starter);
    };
    let source = match git_url(given) {
        Some(url) => {
            println!("Getting your config from {url}");
            let dir = PathBuf::from(CLONE_DIR);
            if !runner.dry() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            let ok = runner.run(&[
                "git".into(),
                "clone".into(),
                "--quiet".into(),
                url.clone(),
                dir.to_string_lossy().into_owned(),
            ])?;
            if !ok {
                bail!(
                    "Couldn't get {url}.\n  \
                     Check the address and that this computer is online. A private\n  \
                     repository needs its https:// address and a token, or copy the\n  \
                     config onto the USB and use --config with the file instead."
                );
            }
            Source::Repo { url, dir }
        }
        None => {
            let path = PathBuf::from(given);
            if !path.is_file() {
                bail!(
                    "--config: there's no file called {given}.\n  \
                     Give the path to a bunny.toml, or a git repository like github:you/my-system."
                );
            }
            Source::File(path)
        }
    };
    if runner.dry() && matches!(source, Source::Repo { .. }) {
        println!("  (dry run: the repository isn't cloned, so its config isn't checked)");
        return Ok(source);
    }
    let file = config_file(&source).expect("not the starter");
    if !file.is_file() {
        bail!(
            "The repository has no bunny.toml at its top.\n  \
             Put your config there (with any host files in hosts/), then try again."
        );
    }
    let cfg = Config::load_for_host(&file, None)?;
    let problems = cfg.problems();
    if !problems.is_empty() {
        let mut text = format!("{} has problems, so nothing was erased:\n", file.display());
        for p in problems {
            text.push_str(&format!("\n  • {p}\n"));
        }
        text.push_str("\nFix them, then run bunny install again.");
        bail!(text);
    }
    println!("✓ Your config ({}) looks good.", file.display());
    Ok(source)
}

/// The `bunny.toml` a source brings (none for the starter).
fn config_file(source: &Source) -> Option<PathBuf> {
    match source {
        Source::Starter => None,
        Source::File(path) => Some(path.clone()),
        Source::Repo { dir, .. } => Some(dir.join("bunny.toml")),
    }
}

fn write(runner: &dyn Runner, path: &Path, text: &str) -> Result<()> {
    if runner.dry() {
        super::would(runner, "write", &path.display().to_string());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Couldn't create {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("Couldn't write {}", path.display()))
}

/// Puts the config and this computer's host file in the new system.
/// Returns the config's path inside the new system's mount.
pub fn put_in_place(
    source: &Source,
    choices: &Choices,
    account: &Account,
    uuids: &Uuids,
    runner: &mut dyn Runner,
) -> Result<PathBuf> {
    let dir = choices.mnt.join(DIR);
    let config_path = dir.join("bunny.toml");
    super::say(
        runner,
        &format!("Writing your config to /{DIR}/bunny.toml"),
        &format!(
            "{}.\nAlso writes hosts/{}.toml with this computer's disk details.",
            match source {
                Source::Starter => "Writes a starter config from your answers",
                Source::File(_) => "Copies your config",
                Source::Repo { .. } => "Copies your config's repository (with its history)",
            },
            account.hostname
        ),
    );
    match source {
        Source::Starter => write(runner, &config_path, &starter::config(account))?,
        Source::File(path) => {
            if runner.dry() {
                super::would(
                    runner,
                    "copy",
                    &format!("{} to /{DIR}/bunny.toml", path.display()),
                );
            } else {
                std::fs::create_dir_all(&dir)?;
                std::fs::copy(path, &config_path)
                    .with_context(|| format!("Couldn't copy {}", path.display()))?;
            }
        }
        Source::Repo { dir: from, .. } => {
            // The whole repository, with its history, so `git pull` works later.
            if !runner.run(&[
                "cp".into(),
                "-a".into(),
                format!("{}/.", from.display()),
                dir.to_string_lossy().into_owned(),
            ])? {
                bail!("Couldn't copy your config into the new system.");
            }
        }
    }
    let host_path = config::host_path(&config_path, &account.hostname);
    let host_text = starter::host_file(choices, account, uuids);
    if !runner.dry() && host_path.is_file() {
        // The repository already has this computer's file: keep it, and put
        // the new record next to it.
        let beside = host_path.with_extension("toml.from-install");
        write(runner, &beside, &host_text)?;
        println!(
            "  Kept your hosts/{0}.toml. The disk details from this install are in\n  \
             hosts/{0}.toml.from-install if you want to copy them in.",
            account.hostname
        );
    } else {
        write(runner, &host_path, &host_text)?;
    }
    Ok(config_path)
}

/// What the install put in that the config doesn't have, so the first
/// switch would remove it. Empty means the config covers the whole system.
pub fn missing(cfg: &Config, choices: &Choices) -> Vec<&'static str> {
    starter::boot_packages(choices)
        .into_iter()
        .filter(|atom| !cfg.packages.iter().any(|p| p == atom))
        .collect()
}

/// Lets you edit the config before it's applied (`--edit`), until it's
/// valid or you stop.
pub fn edit(config_path: &Path, host: &str, runner: &mut dyn Runner) -> Result<()> {
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| {
            if Path::new("/usr/bin/nano").exists() {
                "nano".into()
            } else {
                "vi".into()
            }
        });
    println!(
        "\n==> Your config is ready to edit. When you save and close {editor}, bunny checks it\n    \
         and applies it. The Guide has examples: https://bunnyos-dev.github.io/bunnyos/"
    );
    loop {
        if !runner.run(&[editor.clone(), config_path.to_string_lossy().into_owned()])? {
            println!("{editor} didn't finish normally.");
        }
        if runner.dry() {
            return Ok(());
        }
        match Config::load_for_host(config_path, Some(host)) {
            Ok(cfg) if cfg.problems().is_empty() => {
                println!("✓ The config looks good.");
                return Ok(());
            }
            Ok(cfg) => {
                for p in cfg.problems() {
                    println!("  • {p}\n");
                }
            }
            Err(err) => println!("  • {err:#}\n"),
        }
        print!("Edit it again? (no keeps the starter settings that work) [Y/n] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let answer = crate::interrupt::read_line()?.unwrap_or_default();
        if answer.trim().eq_ignore_ascii_case("n") || answer.trim().eq_ignore_ascii_case("no") {
            bail!("The config still has problems");
        }
    }
}

/// The first `bunny switch` and `bunny lock`, inside the new system.
/// Returns false (after explaining) if they didn't finish; the system still
/// starts either way, so this never stops the install.
pub fn first_switch(
    choices: &Choices,
    account: &Account,
    config_path: &Path,
    runner: &mut dyn Runner,
) -> Result<bool> {
    let mnt = choices.mnt.to_string_lossy().into_owned();
    if !runner.dry() {
        let cfg = Config::load_for_host(config_path, Some(&account.hostname))?;
        let gone = missing(&cfg, choices);
        if !gone.is_empty() {
            println!(
                "\n⚠ Your config doesn't list some packages this computer needs to start:\n    {}\n  \
                 A switch would remove them, so bunny didn't run one. After restarting,\n  \
                 add them to hosts/{}.toml and run: sudo bunny switch",
                gone.join("\n    "),
                account.hostname
            );
            return Ok(false);
        }
    }
    super::say(
        runner,
        "Applying your config (the first bunny switch)",
        "Runs bunny switch and bunny lock inside the new system, so it starts\n\
         already matching your config, with its first saved version.",
    );
    let switched = runner.run(&[
        "chroot".into(),
        mnt.clone(),
        "/usr/local/bin/bunny".into(),
        "switch".into(),
        "--yes".into(),
        format!("/{DIR}/bunny.toml"),
    ])?;
    if !switched {
        println!(
            "\n⚠ The first bunny switch didn't finish (the message is above). The system is\n  \
             installed and will start. After restarting, run: sudo bunny switch"
        );
        return Ok(false);
    }
    let locked = runner.run(&[
        "chroot".into(),
        mnt,
        "/usr/local/bin/bunny".into(),
        "lock".into(),
        format!("/{DIR}/bunny.toml"),
    ])?;
    if !locked {
        println!("\n⚠ Couldn't write bunny.lock. After restarting, run: sudo bunny lock");
        return Ok(false);
    }
    Ok(true)
}

/// Makes the new system's `/etc/bunny` a git repository with a first commit,
/// "As installed", so every change after it has a history. A repository
/// your config came from keeps its own history. Problems here are explained
/// and never stop the install.
pub fn start_history(
    source: &Source,
    choices: &Choices,
    account: &Account,
    runner: &mut dyn Runner,
) -> Result<()> {
    let dir = choices.mnt.join(DIR);
    if matches!(source, Source::Repo { .. }) {
        println!(
            "\n  /{DIR} keeps the git history of the repository it came from. To save\n  \
             this computer's hosts/{}.toml there too: cd /{DIR} && sudo git add hosts && sudo git commit",
            account.hostname
        );
        return Ok(());
    }
    super::say(
        runner,
        &format!("Starting your config's history (git, in /{DIR})"),
        "Makes /etc/bunny a git repository with a first commit, \"As installed\",\nso every later change can be saved and sent to GitHub with `bunny publish`.",
    );
    let who = crate::config_repo::Identity::placeholder(&account.username, &account.hostname);
    if !crate::config_repo::start(&dir, &who, "As installed", runner)? {
        println!(
            "\n⚠ Couldn't start the git history of /{DIR} (the message is above). Everything\n  \
             else is installed. After restarting, `bunny publish` starts it."
        );
        return Ok(());
    }
    if !runner.dry() && !choices.mnt.join("usr/bin/git").exists() {
        println!(
            "  ⚠ Your config doesn't install git, so the new system can't read this history\n  \
             yet. After restarting: bunny add dev-vcs/git"
        );
    }
    println!(
        "  ✓ First commit: \"As installed\". Commits there are made as {} <{}>\n    \
         (a placeholder email; `bunny publish` puts it on GitHub).",
        who.name, who.email
    );
    Ok(())
}

/// Removes a cloned `--config` repository from the live system.
pub fn clean_up(source: &Source, runner: &dyn Runner) {
    if let Source::Repo { dir, .. } = source {
        if !runner.dry() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            username: "you".into(),
            hostname: "laptop".into(),
            ssh: false,
            timezone: "UTC".into(),
            keymap: "us".into(),
        }
    }

    /// Records commands and does nothing.
    #[derive(Default)]
    struct Recorded(Vec<String>);
    impl Runner for Recorded {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.0.push(cmd.join(" "));
            Ok(true)
        }
    }

    #[test]
    fn understands_git_addresses() {
        assert_eq!(
            git_url("github:you/my-system").as_deref(),
            Some("https://github.com/you/my-system")
        );
        assert_eq!(
            git_url("https://example.org/you/system.git").as_deref(),
            Some("https://example.org/you/system.git")
        );
        assert_eq!(git_url("github:you"), None);
        assert_eq!(git_url("/root/bunny.toml"), None);
        assert_eq!(git_url("bunny.toml"), None);
    }

    #[test]
    fn checks_your_config_before_anything_is_erased() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.toml");
        std::fs::write(&good, "packages = [\"app-misc/tmux\"]\n").unwrap();
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "packages = [\"tmux\"]\n").unwrap();
        let mut r = Recorded::default();
        assert_eq!(
            prepare(Some(good.to_str().unwrap()), &mut r).unwrap(),
            Source::File(good.clone())
        );
        let err = prepare(Some(bad.to_str().unwrap()), &mut r)
            .unwrap_err()
            .to_string();
        assert!(err.contains("nothing was erased"), "{err}");
        assert!(err.contains("isn't a full package name"), "{err}");
        let err = prepare(Some("/nope.toml"), &mut r).unwrap_err().to_string();
        assert!(err.contains("there's no file called /nope.toml"), "{err}");
        assert_eq!(prepare(None, &mut r).unwrap(), Source::Starter);
        assert!(r.0.is_empty());
    }

    #[test]
    fn writes_the_starter_config_and_host_file() {
        let mnt = tempfile::tempdir().unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        let path = put_in_place(
            &Source::Starter,
            &choices,
            &account(),
            &Uuids::default(),
            &mut Recorded::default(),
        )
        .unwrap();
        assert_eq!(path, mnt.path().join("etc/bunny/bunny.toml"));
        let cfg = Config::load_for_host(&path, Some("laptop")).unwrap();
        assert!(cfg.problems().is_empty());
        assert_eq!(cfg.host.as_ref().unwrap().name, "laptop");
        // Everything the install put in is in the config, so the first
        // switch removes nothing.
        assert!(missing(&cfg, &choices).is_empty());
        for atom in super::super::system::packages(&choices) {
            assert!(cfg.packages.iter().any(|p| p == atom), "{atom}");
        }
    }

    #[test]
    fn keeps_a_host_file_your_repository_already_has() {
        let mnt = tempfile::tempdir().unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        let hosts = mnt.path().join("etc/bunny/hosts");
        std::fs::create_dir_all(&hosts).unwrap();
        std::fs::write(hosts.join("laptop.toml"), "packages = []\n").unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut r = Recorded::default();
        put_in_place(
            &Source::Repo {
                url: "https://example.org/x".into(),
                dir: repo.path().to_path_buf(),
            },
            &choices,
            &account(),
            &Uuids::default(),
            &mut r,
        )
        .unwrap();
        assert!(r.0[0].starts_with("cp -a "), "{:?}", r.0);
        assert_eq!(
            std::fs::read_to_string(hosts.join("laptop.toml")).unwrap(),
            "packages = []\n"
        );
        assert!(hosts.join("laptop.toml.from-install").is_file());
    }

    #[test]
    fn starts_the_history_after_the_first_switch() {
        let mnt = tempfile::tempdir().unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        std::fs::create_dir_all(mnt.path().join("etc/bunny")).unwrap();
        let mut r = Recorded::default();
        start_history(&Source::Starter, &choices, &account(), &mut r).unwrap();
        let d = mnt.path().join("etc/bunny");
        let d = d.display();
        assert_eq!(
            r.0,
            [
                format!("git -C {d} init --quiet -b main"),
                format!("git -C {d} config user.name you"),
                format!("git -C {d} config user.email you@laptop.local"),
                format!("git -C {d} add --all"),
                format!("git -C {d} commit --quiet -m As installed"),
            ]
        );
        assert!(mnt.path().join("etc/bunny/.gitignore").is_file());

        // A config from a repository keeps that repository's history.
        let mut r = Recorded::default();
        let repo = Source::Repo {
            url: "https://example.org/x".into(),
            dir: "/run/x".into(),
        };
        start_history(&repo, &choices, &account(), &mut r).unwrap();
        assert!(r.0.is_empty(), "{:?}", r.0);
    }

    #[test]
    fn wont_switch_away_what_the_computer_needs_to_start() {
        let mnt = tempfile::tempdir().unwrap();
        let choices = Choices {
            disk: "/dev/vda".into(),
            mnt: mnt.path().to_path_buf(),
            ..Default::default()
        };
        let path = mnt.path().join("etc/bunny/bunny.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "packages = [\"app-misc/tmux\"]\n").unwrap();
        let mut r = Recorded::default();
        assert!(!first_switch(&choices, &account(), &path, &mut r).unwrap());
        assert!(r.0.is_empty(), "no switch was run: {:?}", r.0);

        // With the host file, it runs the switch and the lock inside.
        let host = config::host_path(&path, "laptop");
        std::fs::create_dir_all(host.parent().unwrap()).unwrap();
        std::fs::write(
            &host,
            starter::host_file(&choices, &account(), &Uuids::default()),
        )
        .unwrap();
        let mut r = Recorded::default();
        assert!(first_switch(&choices, &account(), &path, &mut r).unwrap());
        let m = mnt.path().display();
        assert_eq!(
            r.0,
            [
                format!("chroot {m} /usr/local/bin/bunny switch --yes /etc/bunny/bunny.toml"),
                format!("chroot {m} /usr/local/bin/bunny lock /etc/bunny/bunny.toml"),
            ]
        );
    }
}
