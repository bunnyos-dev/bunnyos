//! Your config folder (`/etc/bunny`) as a git repository.
//!
//! `bunny install` starts it (branch `main`, a first commit "As installed"),
//! so every later change has a history. `bunny publish` sends it to a
//! private GitHub repository with *your* GitHub sign-in: root's git asks your
//! own `gh` for the password each time, so no token is copied into root's
//! files.
//!
//! Every git command goes through the `Runner`, so a dry run shows them
//! exactly and tests can check them.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::apply::Runner;

/// What git writes on each commit.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl Identity {
    /// For a new install: your user name, and a placeholder email made from
    /// the computer's name (`you@laptop.local`). It's never used to send
    /// mail; it's only what git writes on each commit.
    pub fn placeholder(user: &str, host: &str) -> Identity {
        Identity {
            name: user.to_string(),
            email: format!("{user}@{host}.local"),
        }
    }
}

/// What's never shared, even in a private repository.
pub const GITIGNORE: &str = "\
# Kept on this computer only, never committed (written by bunny).

# The key that unlocks the /home disk.
/keys/

# Copies bunny and editors leave while working.
*.bunny-new
*~
*.swp
";

/// `git -C <dir> <args>`.
pub fn git(dir: &Path, args: &[&str]) -> Vec<String> {
    let mut cmd = vec![
        "git".to_string(),
        "-C".to_string(),
        dir.to_string_lossy().into_owned(),
    ];
    cmd.extend(args.iter().map(|a| a.to_string()));
    cmd
}

/// Whether the folder already has git history.
pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Makes `dir` a git repository on branch `main`, with a repository-only
/// identity, the `.gitignore`, and a first commit of everything in it.
/// Returns false if a git command failed (it says which).
pub fn start(dir: &Path, who: &Identity, message: &str, runner: &mut dyn Runner) -> Result<bool> {
    let steps: [Vec<String>; 3] = [
        git(dir, &["init", "--quiet", "-b", "main"]),
        git(dir, &["config", "user.name", &who.name]),
        git(dir, &["config", "user.email", &who.email]),
    ];
    for cmd in steps {
        if !runner.run(&cmd)? {
            println!("  ✗ `{}` didn't work.", cmd[3..].join(" "));
            return Ok(false);
        }
    }
    write_gitignore(dir, runner)?;
    for cmd in [
        git(dir, &["add", "--all"]),
        git(dir, &["commit", "--quiet", "-m", message]),
    ] {
        if !runner.run(&cmd)? {
            println!("  ✗ `{}` didn't work.", cmd[3..].join(" "));
            return Ok(false);
        }
    }
    Ok(true)
}

/// git's system-wide settings, where `safe.directory` is read from.
pub const SYSTEM_GITCONFIG: &str = "etc/gitconfig";

/// Whether git's system-wide settings at `root` already trust `dir` (a path
/// inside that system, like `/etc/bunny`): a `directory = /etc/bunny` (or
/// `*`) line in a `[safe]` section.
pub fn trusted(root: &Path, dir: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join(SYSTEM_GITCONFIG)) else {
        return false;
    };
    let mut in_safe = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_safe = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .eq_ignore_ascii_case("safe");
            continue;
        }
        if !in_safe {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().trim_matches('"');
            if key.trim().eq_ignore_ascii_case("directory")
                && (value == "*" || value.trim_end_matches('/') == dir.trim_end_matches('/'))
            {
                return true;
            }
        }
    }
    false
}

/// Lets every user on the system read the history of `dir` (owned by root):
/// git refuses ("detected dubious ownership") unless it's listed in
/// `safe.directory`. Adds it to git's system-wide settings (`/etc/gitconfig`
/// at `root`), once. Returns whether anything was added.
pub fn trust(root: &Path, dir: &str, runner: &dyn Runner) -> Result<bool> {
    if trusted(root, dir) {
        return Ok(false);
    }
    let path = root.join(SYSTEM_GITCONFIG);
    if runner.dry() {
        println!("  (would add) safe.directory = {dir} to {}", path.display());
        return Ok(true);
    }
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "# Added by bunny: lets you read the history of {dir}, which belongs to root.\n\
         [safe]\n\tdirectory = {dir}\n"
    ));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Couldn't create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, text)
        .map_err(|e| anyhow::anyhow!("Couldn't write {}: {e}", path.display()))?;
    Ok(true)
}

/// Writes `.gitignore`, keeping one that's already there.
fn write_gitignore(dir: &Path, runner: &dyn Runner) -> Result<()> {
    let path = dir.join(".gitignore");
    if runner.dry() {
        println!("  (would write) {}", path.display());
        return Ok(());
    }
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, GITIGNORE)
        .map_err(|e| anyhow::anyhow!("Couldn't write {}: {e}", path.display()))
}

/// The person running `bunny publish` through sudo or doas: their GitHub
/// sign-in is the one used, never root's.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub name: String,
    pub home: PathBuf,
}

impl Person {
    /// From `SUDO_USER` or `DOAS_USER` (`None` when root logged in
    /// directly), with the home folder from `<root>/etc/passwd`.
    pub fn invoking(root: &Path) -> Option<Person> {
        let name = ["SUDO_USER", "DOAS_USER"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|n| !n.is_empty() && n != "root")?;
        Some(Person::named(&name, root))
    }

    /// A user and their home folder from `<root>/etc/passwd`
    /// (`/home/<name>` if they aren't listed).
    pub fn named(name: &str, root: &Path) -> Person {
        let home = std::fs::read_to_string(root.join("etc/passwd"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split(':').collect::<Vec<_>>())
            .find(|f| f.len() >= 6 && f[0] == name)
            .map(|f| PathBuf::from(f[5]))
            .unwrap_or_else(|| PathBuf::from(format!("/home/{name}")));
        Person {
            name: name.to_string(),
            home,
        }
    }

    /// The start of a command run as this person, with their own settings
    /// (so `gh` finds their sign-in).
    fn prefix(&self) -> Vec<String> {
        let home = self.home.display();
        vec![
            "runuser".into(),
            "-u".into(),
            self.name.clone(),
            "--".into(),
            "env".into(),
            format!("HOME={home}"),
            format!("XDG_CONFIG_HOME={home}/.config"),
        ]
    }

    /// `cmd`, run as this person.
    pub fn run_as(&self, cmd: &[&str]) -> Vec<String> {
        let mut out = self.prefix();
        out.extend(cmd.iter().map(|a| a.to_string()));
        out
    }

    /// The git credential helper that asks this person's `gh` for the
    /// GitHub password, so root's git pushes with their sign-in.
    pub fn credential_helper(&self) -> String {
        format!("!{} gh auth git-credential", self.prefix().join(" "))
    }
}

/// `owner/name` from what was typed: `you/my-system`, `my-system` (yours),
/// `github:you/my-system` or a `https://github.com/…` address.
pub fn repo_name(given: &str, login: &str) -> Option<String> {
    let given = given.trim();
    let rest = given
        .strip_prefix("github:")
        .or_else(|| given.strip_prefix("https://github.com/"))
        .or_else(|| given.strip_prefix("git@github.com:"))
        .unwrap_or(given);
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    let ok = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    match parts[..] {
        [name] if ok(name) && !given.contains(':') => Some(format!("{login}/{name}")),
        [owner, name] if ok(owner) && ok(name) => Some(format!("{owner}/{name}")),
        _ => None,
    }
}

/// The name `bunny publish` uses when none is given.
pub const DEFAULT_REPO: &str = "my-system";

/// What `bunny publish` found, and so what it will do.
#[derive(Debug, Clone, PartialEq)]
pub struct Publish {
    pub dir: PathBuf,
    pub person: Person,
    /// `owner/name` on GitHub.
    pub repo: String,
    /// The folder isn't a git repository yet.
    pub start: bool,
    /// The GitHub repository doesn't exist yet (made private).
    pub create: bool,
    /// The GitHub repository exists and anyone can see it.
    pub public: bool,
    /// The folder already sends to this repository.
    pub connected: bool,
    /// Files changed since the last commit (committed first).
    pub uncommitted: Vec<String>,
    /// The identity for a new repository's commits.
    pub identity: Identity,
    /// Not yet readable by your user: the system's root and the folder's
    /// path in it, for git's `safe.directory` (set by `bunny publish`).
    pub trust: Option<(PathBuf, String)>,
}

/// Looks at the folder and at GitHub, changing nothing.
pub fn look(
    dir: &Path,
    person: &Person,
    host: &str,
    given: Option<&str>,
    gh_installed: bool,
    runner: &mut dyn Runner,
) -> Result<Publish> {
    let login = match runner
        .output(&person.run_as(&["gh", "api", "user", "--jq", ".login"]))
        .ok()
        .flatten()
    {
        Some(login) if !login.trim().is_empty() => login.trim().to_string(),
        _ if runner.dry() => "<your GitHub name>".to_string(),
        _ if !gh_installed => bail!(
            "GitHub's command-line tool isn't installed. bunny publish uses it to sign in\n  \
             to GitHub as you. Install it and sign in, then run bunny publish again:\n\n    \
             bunny add dev-util/github-cli\n    gh auth login"
        ),
        _ => bail!(
            "{} isn't signed in to GitHub. Sign in as yourself (not with sudo), then\n  \
             run bunny publish again:\n\n    gh auth login\n\n  \
             Without a desktop it can't open a browser or copy the code; that's\n  \
             expected. Open github.com/login/device on another device and type the code.",
            person.name
        ),
    };
    let start = !is_repo(dir);
    let mut origin = None;
    let mut uncommitted = Vec::new();
    if !start {
        if let Some(url) = runner
            .output(&git(dir, &["remote", "get-url", "origin"]))
            .ok()
            .flatten()
        {
            let url = url.trim().to_string();
            match repo_name(&url, &login) {
                Some(name) if url.contains("github") => origin = Some(name),
                _ => bail!(
                    "{} already sends to {url}, which isn't on GitHub. bunny publish only\n  \
                     knows GitHub; send it there with: cd {} && sudo git push",
                    dir.display(),
                    dir.display()
                ),
            }
        }
        if let Some(status) = runner
            .output(&git(dir, &["status", "--porcelain"]))
            .ok()
            .flatten()
        {
            uncommitted = status
                .lines()
                .filter(|l| l.len() > 3)
                .map(|l| l[3..].to_string())
                .collect();
        }
    }
    let wanted = match given {
        Some(given) => Some(repo_name(given, &login).ok_or_else(|| {
            anyhow::anyhow!(
                "\"{given}\" isn't a GitHub repository name. Use a name like my-system,\n  \
                 or you/my-system."
            )
        })?),
        None => None,
    };
    let repo = match (&origin, wanted) {
        (Some(have), Some(want)) if !have.eq_ignore_ascii_case(&want) => bail!(
            "{} already sends to github.com/{have}, so nothing was changed.\n  \
             To send it to github.com/{want} instead, run:\n\n    \
             sudo git -C {} remote set-url origin https://github.com/{want}.git\n    \
             bunny publish",
            dir.display(),
            dir.display()
        ),
        (Some(have), _) => have.clone(),
        (None, Some(want)) => want,
        (None, None) => format!("{login}/{DEFAULT_REPO}"),
    };
    let visibility = runner
        .output(&person.run_as(&[
            "gh",
            "repo",
            "view",
            &repo,
            "--json",
            "visibility",
            "--jq",
            ".visibility",
        ]))
        .ok()
        .flatten();
    let (create, public) = match visibility {
        Some(v) => (false, v.trim().eq_ignore_ascii_case("public")),
        None => (!runner.dry() || origin.is_none(), false),
    };
    Ok(Publish {
        dir: dir.to_path_buf(),
        person: person.clone(),
        repo,
        start,
        create,
        public,
        connected: origin.is_some(),
        uncommitted,
        identity: Identity::placeholder(&person.name, host),
        trust: None,
    })
}

impl Publish {
    /// What will happen, in plain words.
    pub fn preview(&self) -> String {
        let dir = self.dir.display();
        let mut out = format!(
            "bunny publish sends {dir} to github.com/{}, signed in to GitHub as {}:\n\n",
            self.repo, self.person.name
        );
        if self.start {
            out.push_str(&format!(
                "  • Start its git history (branch main, commits made as {} <{}>)\n",
                self.identity.name, self.identity.email
            ));
        }
        if self.create {
            out.push_str(&format!(
                "  • Make the repository github.com/{} (private: only you can see it)\n",
                self.repo
            ));
        }
        if !self.connected {
            out.push_str(
                "  • Connect the folder to it, using your own GitHub sign-in (root's git asks\n    \
                 your gh each time; no password is copied)\n",
            );
        }
        if !self.uncommitted.is_empty() {
            out.push_str(&format!(
                "  • Save what changed since the last commit: {}\n",
                self.uncommitted.join(", ")
            ));
        }
        if let Some((_, dir)) = &self.trust {
            out.push_str(&format!(
                "  • Let your user read its history (`git log`): add {dir} to git's safe.directory\n    \
                 in /etc/gitconfig\n"
            ));
        }
        out.push_str("  • Send the history to GitHub\n");
        if self.public {
            out.push_str(&format!(
                "\n⚠ github.com/{} is public: anyone can see your config, including this\n  \
                 computer's name, your user name and disk details. Make it private on GitHub\n  \
                 first (Settings → Danger Zone), or pick another name.\n",
                self.repo
            ));
        }
        out
    }

    /// Does it. Stops at the first step that fails, saying what to do.
    pub fn carry_out(&self, runner: &mut dyn Runner) -> Result<()> {
        let dir = &self.dir;
        if self.start {
            println!("\n==> Starting the history of {}", dir.display());
            if !start(dir, &self.identity, "My system", runner)? {
                bail!("Couldn't start the git history (the message is above). Nothing was sent.");
            }
        }
        if let Some((root, inside)) = &self.trust {
            if let Err(e) = trust(root, inside, runner) {
                println!("  ⚠ {e:#}. Your user can't read its history with git yet.");
            }
        }
        if self.create {
            println!("\n==> Making github.com/{} (private)", self.repo);
            if !runner.run(&self.person.run_as(&[
                "gh",
                "repo",
                "create",
                &self.repo,
                "--private",
                "--description",
                "My Bunny Linux system (bunny.toml)",
            ]))? {
                bail!(
                    "GitHub didn't make github.com/{} (the message is above). Nothing was sent.",
                    self.repo
                );
            }
        }
        if !self.connected {
            println!("\n==> Connecting {} to it", dir.display());
            let key = "credential.https://github.com.helper";
            // Clear helpers root may have from elsewhere, then use yours.
            let _ = runner.run(&git(dir, &["config", "--unset-all", key]))?;
            let helper = self.person.credential_helper();
            let url = format!("https://github.com/{}.git", self.repo);
            for cmd in [
                git(dir, &["config", "--add", key, ""]),
                git(dir, &["config", "--add", key, &helper]),
                git(dir, &["remote", "add", "origin", &url]),
            ] {
                if !runner.run(&cmd)? {
                    bail!("`{}` didn't work. Nothing was sent.", cmd[3..].join(" "));
                }
            }
        }
        if !self.uncommitted.is_empty() {
            println!("\n==> Saving what changed");
            for cmd in [
                git(dir, &["add", "--all"]),
                git(
                    dir,
                    &["commit", "--quiet", "-m", "Saved with bunny publish"],
                ),
            ] {
                if !runner.run(&cmd)? {
                    bail!("`{}` didn't work. Nothing was sent.", cmd[3..].join(" "));
                }
            }
        }
        println!("\n==> Sending it to GitHub");
        if !runner.run(&git(dir, &["push", "--quiet", "-u", "origin", "HEAD"]))? {
            bail!(
                "GitHub didn't take it (the message is above). If the repository already had\n  \
                 something in it (like a README made on GitHub), use a new, empty one:\n\n    \
                 sudo git -C {d} remote set-url origin https://github.com/{}/<new name>.git\n    \
                 bunny publish",
                self.repo.split('/').next().unwrap_or("you"),
                d = dir.display()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Records commands, answers `output` from a list, and does nothing.
    #[derive(Default)]
    pub(crate) struct Recorded {
        pub ran: Vec<String>,
        /// (part of the command, what it prints); no match means it failed.
        pub answers: Vec<(String, String)>,
        /// Commands that start with one of these fail.
        pub failing: Vec<String>,
        pub dry: bool,
    }

    impl Runner for Recorded {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            let line = cmd.join(" ");
            let ok = !self.failing.iter().any(|f| line.starts_with(f.as_str()));
            self.ran.push(line);
            Ok(ok)
        }
        fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
            let line = cmd.join(" ");
            self.ran.push(line.clone());
            Ok(self
                .answers
                .iter()
                .find(|(part, _)| line.contains(part.as_str()))
                .map(|(_, out)| out.clone()))
        }
        fn dry(&self) -> bool {
            self.dry
        }
    }

    #[test]
    fn starts_a_repository_on_main_with_its_own_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Recorded::default();
        let who = Identity::placeholder("you", "laptop");
        assert_eq!(who.email, "you@laptop.local");
        assert!(start(dir.path(), &who, "As installed", &mut r).unwrap());
        let d = dir.path().display();
        assert_eq!(
            r.ran,
            [
                format!("git -C {d} init --quiet -b main"),
                format!("git -C {d} config user.name you"),
                format!("git -C {d} config user.email you@laptop.local"),
                format!("git -C {d} add --all"),
                format!("git -C {d} commit --quiet -m As installed"),
            ]
        );
        let ignore = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == "/keys/"), "{ignore}");
    }

    #[test]
    fn keeps_a_gitignore_that_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "mine\n").unwrap();
        let mut r = Recorded::default();
        start(dir.path(), &Identity::placeholder("you", "pc"), "x", &mut r).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
            "mine\n"
        );
    }

    #[test]
    fn stops_at_the_first_git_command_that_fails() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display();
        let mut r = Recorded {
            failing: vec![format!("git -C {d} init")],
            ..Default::default()
        };
        assert!(!start(dir.path(), &Identity::placeholder("you", "pc"), "x", &mut r).unwrap());
        assert_eq!(r.ran.len(), 1);
        assert!(!dir.path().join(".gitignore").exists());
    }

    fn you() -> Person {
        Person {
            name: "you".into(),
            home: "/home/you".into(),
        }
    }

    fn answers(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn understands_repository_names() {
        let r = |g| repo_name(g, "octo");
        assert_eq!(r("my-system").as_deref(), Some("octo/my-system"));
        assert_eq!(r("you/my-system").as_deref(), Some("you/my-system"));
        assert_eq!(r("github:you/my-system").as_deref(), Some("you/my-system"));
        assert_eq!(
            r("https://github.com/you/my-system.git").as_deref(),
            Some("you/my-system")
        );
        assert_eq!(
            r("git@github.com:you/my-system.git").as_deref(),
            Some("you/my-system")
        );
        assert_eq!(r("a/b/c"), None);
        assert_eq!(r("my system"), None);
        assert_eq!(r(""), None);
    }

    #[test]
    fn runs_github_commands_as_you_not_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc")).unwrap();
        std::fs::write(
            root.path().join("etc/passwd"),
            "root:x:0:0::/root:/bin/bash\nyou:x:1000:1000::/home/elsewhere:/bin/bash\n",
        )
        .unwrap();
        let p = Person::named("you", root.path());
        assert_eq!(p.home, PathBuf::from("/home/elsewhere"));
        assert_eq!(
            Person::named("nobody-here", root.path()).home,
            PathBuf::from("/home/nobody-here")
        );
        assert_eq!(
            p.run_as(&["gh", "auth", "status"]).join(" "),
            "runuser -u you -- env HOME=/home/elsewhere XDG_CONFIG_HOME=/home/elsewhere/.config gh auth status"
        );
        assert_eq!(
            p.credential_helper(),
            "!runuser -u you -- env HOME=/home/elsewhere XDG_CONFIG_HOME=/home/elsewhere/.config gh auth git-credential"
        );
    }

    #[test]
    fn publishes_a_folder_that_has_no_history_yet() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display();
        let mut r = Recorded {
            answers: answers(&[("gh api user", "octo\n")]),
            ..Default::default()
        };
        let plan = look(dir.path(), &you(), "laptop", None, true, &mut r).unwrap();
        assert_eq!(plan.repo, "octo/my-system");
        assert!(plan.start && plan.create && !plan.connected && !plan.public);
        let preview = plan.preview();
        assert!(
            preview.contains("github.com/octo/my-system (private"),
            "{preview}"
        );
        assert!(preview.contains("you <you@laptop.local>"), "{preview}");
        r.ran.clear();
        plan.carry_out(&mut r).unwrap();
        let me = "runuser -u you -- env HOME=/home/you XDG_CONFIG_HOME=/home/you/.config";
        let key = "credential.https://github.com.helper";
        assert_eq!(
            r.ran,
            [
                format!("git -C {d} init --quiet -b main"),
                format!("git -C {d} config user.name you"),
                format!("git -C {d} config user.email you@laptop.local"),
                format!("git -C {d} add --all"),
                format!("git -C {d} commit --quiet -m My system"),
                format!("{me} gh repo create octo/my-system --private --description My Bunny Linux system (bunny.toml)"),
                format!("git -C {d} config --unset-all {key}"),
                format!("git -C {d} config --add {key} "),
                format!("git -C {d} config --add {key} !{me} gh auth git-credential"),
                format!("git -C {d} remote add origin https://github.com/octo/my-system.git"),
                format!("git -C {d} push --quiet -u origin HEAD"),
            ]
        );
    }

    #[test]
    fn publishing_again_saves_changes_and_sends_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let d = dir.path().display();
        let mut r = Recorded {
            answers: answers(&[
                ("gh api user", "octo\n"),
                (
                    "remote get-url origin",
                    "https://github.com/octo/my-system.git\n",
                ),
                ("status --porcelain", " M bunny.toml\n M bunny.lock\n"),
                ("gh repo view", "PRIVATE\n"),
            ]),
            ..Default::default()
        };
        let plan = look(dir.path(), &you(), "laptop", None, true, &mut r).unwrap();
        assert!(!plan.start && !plan.create && plan.connected);
        assert_eq!(plan.uncommitted, ["bunny.toml", "bunny.lock"]);
        r.ran.clear();
        plan.carry_out(&mut r).unwrap();
        assert_eq!(
            r.ran,
            [
                format!("git -C {d} add --all"),
                format!("git -C {d} commit --quiet -m Saved with bunny publish"),
                format!("git -C {d} push --quiet -u origin HEAD"),
            ]
        );
    }

    #[test]
    fn wont_switch_to_another_repository_by_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let mut r = Recorded {
            answers: answers(&[
                ("gh api user", "octo\n"),
                (
                    "remote get-url origin",
                    "https://github.com/octo/my-system.git\n",
                ),
            ]),
            ..Default::default()
        };
        let err = look(dir.path(), &you(), "pc", Some("other"), true, &mut r)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("already sends to github.com/octo/my-system"),
            "{err}"
        );
        assert!(
            err.contains("remote set-url origin https://github.com/octo/other.git"),
            "{err}"
        );
    }

    #[test]
    fn explains_a_missing_github_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Recorded::default();
        let err = look(dir.path(), &you(), "pc", None, true, &mut r)
            .unwrap_err()
            .to_string();
        assert!(err.contains("you isn't signed in to GitHub"), "{err}");
        assert!(err.contains("github.com/login/device"), "{err}");
        let err = look(dir.path(), &you(), "pc", None, false, &mut r)
            .unwrap_err()
            .to_string();
        assert!(err.contains("bunny add dev-util/github-cli"), "{err}");
    }

    #[test]
    fn warns_before_sending_to_a_public_repository() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Recorded {
            answers: answers(&[("gh api user", "octo\n"), ("gh repo view", "PUBLIC\n")]),
            ..Default::default()
        };
        let plan = look(dir.path(), &you(), "pc", Some("dotfiles"), true, &mut r).unwrap();
        assert!(plan.public && !plan.create);
        assert!(plan
            .preview()
            .contains("⚠ github.com/octo/dotfiles is public"));
    }

    #[test]
    fn a_failed_push_explains_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let d = dir.path().display();
        let mut r = Recorded {
            answers: answers(&[
                ("gh api user", "octo\n"),
                (
                    "remote get-url origin",
                    "https://github.com/octo/my-system.git\n",
                ),
                ("gh repo view", "PRIVATE\n"),
            ]),
            failing: vec![format!("git -C {d} push")],
            ..Default::default()
        };
        let plan = look(dir.path(), &you(), "pc", None, true, &mut r).unwrap();
        let err = plan.carry_out(&mut r).unwrap_err().to_string();
        assert!(err.contains("GitHub didn't take it"), "{err}");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Recorded {
            dry: true,
            ..Default::default()
        };
        assert!(start(dir.path(), &Identity::placeholder("you", "pc"), "x", &mut r).unwrap());
        assert!(!dir.path().join(".gitignore").exists());
        assert_eq!(r.ran.len(), 5);
    }

    #[test]
    fn lets_everyone_read_the_history_once() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let real = Recorded::default();
        assert!(!trusted(r, "/etc/bunny"));
        assert!(trust(r, "/etc/bunny", &real).unwrap());
        let text = std::fs::read_to_string(r.join(SYSTEM_GITCONFIG)).unwrap();
        assert!(
            text.contains("[safe]\n\tdirectory = /etc/bunny\n"),
            "{text}"
        );
        assert!(trusted(r, "/etc/bunny"));
        // Twice changes nothing.
        assert!(!trust(r, "/etc/bunny", &real).unwrap());
        assert_eq!(
            std::fs::read_to_string(r.join(SYSTEM_GITCONFIG)).unwrap(),
            text
        );
    }

    #[test]
    fn keeps_what_the_system_gitconfig_already_says() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        std::fs::create_dir_all(r.join("etc")).unwrap();
        let mine = "[core]\n\teditor = vim\n[safe]\n\tdirectory = /srv/repo";
        std::fs::write(r.join(SYSTEM_GITCONFIG), mine).unwrap();
        assert!(!trusted(r, "/etc/bunny"));
        trust(r, "/etc/bunny", &Recorded::default()).unwrap();
        let text = std::fs::read_to_string(r.join(SYSTEM_GITCONFIG)).unwrap();
        assert!(text.starts_with(&format!("{mine}\n")), "{text}");
        assert!(trusted(r, "/etc/bunny"));
        assert!(trusted(r, "/srv/repo"));

        // Trusting everything (`*`) already covers it.
        let all = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(all.path().join("etc")).unwrap();
        std::fs::write(
            all.path().join(SYSTEM_GITCONFIG),
            "[Safe]\n  directory = *\n",
        )
        .unwrap();
        assert!(trusted(all.path(), "/etc/bunny"));
        // Only in a [safe] section.
        std::fs::write(
            all.path().join(SYSTEM_GITCONFIG),
            "[core]\n  directory = /etc/bunny\n",
        )
        .unwrap();
        assert!(!trusted(all.path(), "/etc/bunny"));
    }

    #[test]
    fn a_dry_run_adds_no_safe_directory() {
        let root = tempfile::tempdir().unwrap();
        let dry = Recorded {
            dry: true,
            ..Default::default()
        };
        assert!(trust(root.path(), "/etc/bunny", &dry).unwrap());
        assert!(!root.path().join(SYSTEM_GITCONFIG).exists());
    }

    #[test]
    fn publish_lets_your_user_read_the_history() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("etc/bunny");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let mut r = Recorded {
            answers: answers(&[
                ("gh api user", "octo\n"),
                ("remote get-url", "https://github.com/octo/my-system.git\n"),
                ("status --porcelain", ""),
                ("gh repo view", "PRIVATE\n"),
            ]),
            ..Default::default()
        };
        let mut plan = look(&dir, &you(), "pc", None, true, &mut r).unwrap();
        plan.trust = Some((root.path().to_path_buf(), "/etc/bunny".into()));
        let preview = plan.preview();
        assert!(
            preview.contains("add /etc/bunny to git's safe.directory"),
            "{preview}"
        );
        plan.carry_out(&mut r).unwrap();
        assert!(trusted(root.path(), "/etc/bunny"));
    }
}
