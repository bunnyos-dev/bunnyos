use super::install::interrupted_message;
use super::*;

#[test]
fn explains_a_ctrl_c_during_install() {
    let outcome = Outcome {
        snapshot: Some(PathBuf::from("/.snapshots/bunny-20261118-042343")),
        log: Some(PathBuf::from("/var/log/bunny/20261118-042343-emerge.log")),
        ..Outcome::default()
    };
    let text = interrupted_message(
        Some(&(102, 102, "www-client/firefox".to_string())),
        &["www-client/firefox".to_string()],
        &outcome,
        "bunny switch",
    );
    assert!(
        text.starts_with("Stopped with Ctrl+C while installing."),
        "{text}"
    );
    assert!(
        text.contains(
            "101 of 102 packages finished installing.\n  www-client/firefox was in progress"
        ),
        "{text}"
    );
    assert!(
        text.contains("Your config still lists www-client/firefox"),
        "{text}"
    );
    assert!(text.contains("`bunny rollback`, then restart"), "{text}");
    assert!(text.contains("/.snapshots/bunny-20261118-042343"), "{text}");
    assert!(
        text.contains("`bunny switch` picks up where this stopped"),
        "{text}"
    );
    assert!(
        text.contains("emerge's output so far is in /var/log/bunny/"),
        "{text}"
    );

    let early = interrupted_message(None, &[], &Outcome::default(), "bunny update");
    assert!(early.contains("`bunny update` picks up"), "{early}");
    assert!(
        early.contains("hadn't started installing anything"),
        "{early}"
    );
    assert!(
        !early.contains("rollback"),
        "no snapshot, so no rollback to offer: {early}"
    );
}
use crate::plan;

/// Records commands instead of running them.
#[derive(Default)]
struct Recorder {
    commands: Vec<String>,
    fail: Option<&'static str>,
    /// What emerge "prints" when its output is captured.
    emerge_output: String,
}

impl Runner for Recorder {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        let line = cmd.join(" ");
        let ok = self.fail.is_none_or(|f| !line.contains(f));
        self.commands.push(line);
        Ok(ok)
    }

    fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
        Ok((self.run(cmd)?, self.emerge_output.clone()))
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn system_runner_captures_and_passes_through() {
    // A question with no newline (like emerge's "[Yes/No]"), output on
    // both stdout and stderr, and a failing exit code.
    let cmd = cmd(&[
        "sh",
        "-c",
        "printf 'Continue? [Yes/No] '; echo oops >&2; echo done; exit 3",
    ]);
    let (ok, text) = SystemRunner.run_captured(&cmd).unwrap();
    assert!(!ok);
    assert!(text.contains("Continue? [Yes/No] "), "{text}");
    assert!(text.contains("oops"), "{text}");
    assert!(text.contains("done"), "{text}");
}

#[test]
fn failed_emerge_is_explained() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder {
        fail: Some("@world"),
        emerge_output: fixture("use-change-swtpm.txt"),
        ..Default::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &opts())
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("emerge stopped"), "{err}");
    assert!(err.contains("✗ swtpm needs gnutls"), "{err}");
    assert!(err.contains("[pkg.\"net-libs/gnutls\"]"), "{err}");
    assert!(err.contains("Nothing is lost"), "{err}");
    assert!(err.contains("bunny-20261116-120000"), "{err}");
}

#[test]
fn unknown_emerge_failure_says_so() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder {
        fail: Some("@world"),
        emerge_output: "something new and strange\n".into(),
        ..Default::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &opts())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("doesn't recognise emerge's message yet"),
        "{err}"
    );
}

#[test]
fn emerge_notices_become_tips() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder {
        emerge_output: format!("\u{1b}[32m*\u{1b}[0m{}", fixture("news-and-depclean.txt")),
        ..Default::default()
    };
    let outcome = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap();
    // The news is passed on; emerge's "remove unused packages" reminder
    // isn't, since bunny just did that.
    assert_eq!(outcome.tips.len(), 1, "{:#?}", outcome.tips);
    assert!(outcome.tips[0].contains("26 announcements"));
}

#[test]
fn kept_lines_remember_their_file_on_later_switches() {
    let root = fake_system();
    let r = root.path();
    let use_dir = r.join(portage::PACKAGE_USE_DIR);
    std::fs::create_dir_all(&use_dir).unwrap();
    std::fs::write(use_dir.join("ranges"), "<dev-lang/rust-2 nightly\n").unwrap();
    let config = Config::parse(CONFIG).unwrap();
    for _ in 0..2 {
        let plan = plan::make(&config, r).unwrap();
        let mut runner = Recorder::default();
        switch(&config, r, &plan, &mut runner, &opts()).unwrap();
    }
    let kept = std::fs::read_to_string(use_dir.join(portage::KEPT_USE_FILE)).unwrap();
    assert!(
        kept.contains("# from /etc/portage/package.use/ranges\n<dev-lang/rust-2 nightly"),
        "{kept}"
    );
    assert!(!kept.contains(portage::KEPT_USE_FILE), "{kept}");
}

#[test]
fn asks_emerge_for_colors_on_a_terminal() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder::default();
    let opts = Options {
        color: true,
        ..opts()
    };
    switch(&config, root.path(), &plan, &mut runner, &opts).unwrap();
    assert!(
        runner.commands[1].contains("--color=y @world"),
        "{}",
        runner.commands[1]
    );
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn fake_system() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        portage::WORLD_FILE,
        "app-editors/neovim\napp-misc/old\n",
    );
    write(
        root,
        "etc/portage/package.use/mine",
        "www-client/firefox pgo\n",
    );
    write(root, "etc/portage/package.use/odd", "<dev-libs/x-2 y\n");
    write(root, "etc/init.d/sshd", "");
    std::fs::create_dir_all(root.join(".snapshots")).unwrap();
    dir
}

fn opts() -> Options {
    Options {
        yes: false,
        stamp: "20261116-120000".into(),
        update_all: false,
        color: false,
        // These tests cover emerge's own output and questions; the quiet
        // path has its own tests below.
        verbose: true,
        confirm: |_, _| Ok(true),
    }
}

const CONFIG: &str = r#"
    packages = ["app-editors/neovim", "app-shells/zsh"]
    services = ["sshd", "bluetooth"]
    [pkg."www-client/firefox"]
    use = ["pgo", "-telemetry"]
"#;

#[test]
fn a_switch_sets_up_the_console_font_on_an_existing_system() {
    // An existing Gentoo system: kbd's font and OpenRC's consolefont
    // service, with Gentoo's stock setting. Only bunny install set the
    // font up before.
    let root = fake_system();
    let r = root.path();
    write(r, "usr/share/consolefonts/eurlatgr.psfu.gz", "font");
    write(r, "etc/init.d/consolefont", "");
    write(r, "etc/conf.d/consolefont", "consolefont=\"default8x16\"\n");
    let config = Config::parse(r#"packages = ["app-editors/neovim", "app-misc/old"]"#).unwrap();
    let plan = plan::make(&config, r).unwrap();
    assert_eq!(plan.console_font.len(), 3, "{:?}", plan.console_font);
    let shown = plan.render();
    assert!(shown.contains("Console font, so ✓ ✗ ⚠ show"), "{shown}");
    assert!(plan.summary().contains("console font"));

    let mut runner = Recorder::default();
    let outcome = switch(&config, r, &plan, &mut runner, &opts()).unwrap();
    let c = &runner.commands;
    assert!(c.iter().any(|l| l.starts_with("sh -c set -e;")), "{c:#?}");
    assert!(
        c.iter().any(|l| l == "rc-update add consolefont boot"),
        "{c:#?}"
    );
    assert_eq!(
        std::fs::read_to_string(r.join("etc/conf.d/consolefont")).unwrap(),
        "consolefont=\"/usr/local/share/consolefonts/bunny.psfu\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(outcome.backup_dir.join("etc/conf.d/consolefont")).unwrap(),
        "consolefont=\"default8x16\"\n"
    );
}

#[test]
fn switches_a_system() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder::default();
    let outcome = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap();

    // Commands, in order: snapshot, install, remove (asks), enable + start service.
    assert_eq!(runner.commands.len(), 5, "{:#?}", runner.commands);
    assert!(runner.commands[0].starts_with("btrfs subvolume snapshot -r "));
    assert!(runner.commands[0].ends_with(".snapshots/bunny-20261116-120000"));
    assert_eq!(
        runner.commands[1],
        "emerge --update --deep --newuse --verbose --ask @world"
    );
    assert_eq!(runner.commands[2], "emerge --depclean --ask");
    assert_eq!(runner.commands[3], "rc-update add sshd default");
    assert_eq!(runner.commands[4], "rc-service sshd start");

    // bluetooth isn't installed: noted, not enabled.
    assert!(outcome.notes.iter().any(|n| n.contains("bluetooth")));

    // Files: world written, package.use adopted, old files backed up.
    let r = root.path();
    assert_eq!(
        std::fs::read_to_string(r.join(portage::WORLD_FILE)).unwrap(),
        "app-editors/neovim\napp-shells/zsh\n"
    );
    let use_dir = r.join(portage::PACKAGE_USE_DIR);
    let bunny_file = std::fs::read_to_string(use_dir.join(portage::BUNNY_USE_FILE)).unwrap();
    assert!(bunny_file.contains("www-client/firefox pgo -telemetry"));
    let kept = std::fs::read_to_string(use_dir.join(portage::KEPT_USE_FILE)).unwrap();
    assert!(kept.contains("<dev-libs/x-2 y"));
    assert!(!use_dir.join("mine").exists());
    assert!(outcome.backup_dir.join("package.use/mine").exists());
    assert!(outcome.backup_dir.join("world").exists());

    // Afterwards the plan has no USE or world differences left.
    let after = plan::make(&config, r).unwrap();
    assert!(after.use_changes.is_empty(), "{:#?}", after.use_changes);
    assert!(after.add_packages.is_empty() && after.remove_packages.is_empty());
}

#[test]
fn desktop_section_sets_up_elogind() {
    let root = fake_system();
    let r = root.path();
    write(r, "etc/init.d/dbus", "");
    write(r, "etc/init.d/elogind", "");
    let config = Config::parse(
        "packages = [\"app-editors/neovim\"]\nservices = [\"sshd\"]\n\
         [desktop]\nwayland = true\n",
    )
    .unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = Recorder::default();
    let outcome = switch(&config, r, &plan, &mut runner, &opts()).unwrap();

    let services: Vec<&String> = runner
        .commands
        .iter()
        .filter(|c| c.starts_with("rc-"))
        .collect();
    assert_eq!(
        services,
        [
            "rc-update add dbus default",
            "rc-service dbus start",
            "rc-update add elogind boot",
            "rc-service elogind start",
            "rc-update add sshd default",
            "rc-service sshd start",
        ]
    );
    assert_eq!(
        std::fs::read_to_string(r.join(portage::WORLD_FILE)).unwrap(),
        "app-editors/neovim\nsys-apps/dbus\nsys-auth/elogind\n"
    );
    let bunny_file = std::fs::read_to_string(
        r.join(portage::PACKAGE_USE_DIR)
            .join(portage::BUNNY_USE_FILE),
    )
    .unwrap();
    assert!(
        bunny_file.contains("sys-auth/pambase elogind\n"),
        "{bunny_file}"
    );
    assert!(
        bunny_file.contains("sys-auth/seatd elogind\n"),
        "{bunny_file}"
    );
    assert!(
        outcome
            .notes
            .iter()
            .any(|n| n.contains("Restart the computer once before starting a desktop")),
        "{:#?}",
        outcome.notes
    );
    // One note about it, not "log out" as well.
    assert!(
        !outcome
            .notes
            .iter()
            .any(|n| n.contains("Log out and back in")),
        "{:#?}",
        outcome.notes
    );

    // Afterwards there's nothing left to do for it.
    write(r, "etc/runlevels/boot/elogind", "");
    write(r, "etc/runlevels/default/dbus", "");
    write(r, "etc/runlevels/default/sshd", "");
    let after = plan::make(&config, r).unwrap();
    assert!(after.use_changes.is_empty(), "{:#?}", after.use_changes);
    assert!(after.enable_services.is_empty() && after.add_packages.is_empty());
}

#[test]
fn yes_skips_emerge_questions_but_never_for_removal() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder::default();
    let opts = Options {
        yes: true,
        ..opts()
    };
    switch(&config, root.path(), &plan, &mut runner, &opts).unwrap();
    assert!(!runner.commands[1].contains("--ask"));
    assert_eq!(runner.commands[2], "emerge --depclean --ask");
}

#[test]
fn failed_snapshot_changes_nothing() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder {
        fail: Some("snapshot"),
        ..Default::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap_err();
    assert!(err.to_string().contains("nothing was changed"));
    assert_eq!(runner.commands.len(), 1);
    assert!(root.path().join("etc/portage/package.use/mine").exists());
}

#[test]
fn failed_emerge_points_to_the_backup() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder {
        fail: Some("@world"),
        ..Default::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("backed up in"), "{text}");
    assert!(text.contains("bunny-20261116-120000"), "{text}");
}

#[test]
fn running_twice_is_safe() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    for stamp in ["a", "b"] {
        let plan = plan::make(&config, root.path()).unwrap();
        let opts = Options {
            yes: true,
            stamp: stamp.into(),
            update_all: false,
            color: false,
            verbose: true,
            confirm: |_, _| Ok(true),
        };
        switch(&config, root.path(), &plan, &mut Recorder::default(), &opts).unwrap();
    }
    let kept = std::fs::read_to_string(
        root.path()
            .join(portage::PACKAGE_USE_DIR)
            .join(portage::KEPT_USE_FILE),
    )
    .unwrap();
    // The kept line is still there exactly once.
    assert_eq!(kept.matches("<dev-libs/x-2 y").count(), 1, "{kept}");
}

/// Answers each command with canned output (matched by a piece of the
/// command line), and feeds it line by line like emerge would.
#[derive(Default)]
struct Scripted {
    commands: Vec<String>,
    replies: Vec<(&'static str, bool, String)>,
    /// What bunny's Portage helper answers, per command (`plan`,
    /// `depclean`). Not listed: the helper isn't available.
    helper: Vec<(&'static str, String)>,
}

impl Scripted {
    fn reply(&mut self, line: &str) -> (bool, String) {
        self.commands.push(line.to_string());
        self.replies
            .iter()
            .find(|(pat, _, _)| line.contains(pat))
            .map(|(_, ok, out)| (*ok, out.clone()))
            .unwrap_or((true, String::new()))
    }
}

impl Runner for Scripted {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        Ok(self.reply(&cmd.join(" ")).0)
    }

    fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
        // The helper's command line holds the whole script; record just
        // what it was asked.
        if cmd[0] == "python3" && cmd[1] == "-c" {
            let asked = cmd[3..].join(" ");
            self.commands.push(format!("(helper) {asked}"));
            return Ok(self
                .helper
                .iter()
                .find(|(what, _)| *what == asked)
                .map(|(_, json)| json.clone()));
        }
        Ok(None)
    }

    fn run_quiet(
        &mut self,
        cmd: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(bool, String)> {
        let (ok, out) = self.reply(&cmd.join(" "));
        out.lines().for_each(on_line);
        Ok((ok, out))
    }
}

fn quiet(yes: bool, answer: bool) -> Options {
    Options {
        yes,
        verbose: false,
        confirm: if answer {
            |_, _| Ok(true)
        } else {
            |_, _| Ok(false)
        },
        ..opts()
    }
}

const PRETEND: &str = "[binary   N     ] acct-group/nullmail-0-r2::gentoo\n\
                       [binary   N     ] app-misc/tmux-3.6a::gentoo\n\
                       [binary   N     ] app-editors/neovim-0.11.5::gentoo\n\
                       [binary   N     ] app-shells/zsh-5.9-r6::gentoo\n";
const INSTALL: &str = ">>> Emerging binary (1 of 2) acct-group/nullmail-0-r2::gentoo\n\
                       >>> Emerging binary (2 of 2) app-misc/tmux-3.6a::gentoo\n\
                       \n * Messages for package app-misc/tmux-3.6a:\n\n * Read the manual.\n";

#[test]
fn quiet_install_asks_emerge_first_then_installs_without_its_questions() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        replies: vec![
            ("--pretend --update", true, PRETEND.into()),
            ("--quiet-build=y", true, INSTALL.into()),
        ],
        ..Scripted::default()
    };
    let outcome = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(false, true),
    )
    .unwrap();
    let emerges: Vec<&String> = runner
        .commands
        .iter()
        .filter(|c| c.starts_with("emerge --"))
        .collect();
    assert!(emerges[0].starts_with("emerge --pretend --update --deep --newuse"));
    assert!(emerges[1].contains("--quiet-build=y @world"));
    // emerge never asks itself, even if EMERGE_DEFAULT_OPTS says --ask.
    assert!(emerges[1].contains("--ask=n"), "{}", emerges[1]);
    assert!(
        !emerges[1].replace("--ask=n", "").contains("--ask"),
        "{}",
        emerges[1]
    );
    assert_eq!(
        outcome.messages,
        [(
            "app-misc/tmux".to_string(),
            vec!["Read the manual.".to_string()]
        )]
    );
    let log = outcome.log.expect("log saved");
    assert!(std::fs::read_to_string(log)
        .unwrap()
        .contains(">>> Emerging binary (2 of 2)"));
}

/// emerge's answer for a package that only has a testing version, asked
/// about by name.
const MASKED: &str = "!!! All ebuilds that could satisfy \"app-shells/zsh\" have been masked.\n\
                      !!! One of the following masked packages is required to complete your request:\n\
                      - app-shells/zsh-5.9-r6::gentoo (masked by: ~amd64 keyword)\n";

#[test]
fn a_package_emerge_quietly_leaves_out_is_explained_not_ignored() {
    // Updating @world, emerge skips a package it can't install (only a
    // testing version) with a warning, and plans nothing for it. bunny
    // must not then say the system matches the config.
    let root = fake_system();
    let world = portage::world_path(root.path());
    let world_before = std::fs::read_to_string(&world).unwrap();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    assert!(plan.add_packages.contains(&"app-shells/zsh".to_string()));
    let mut runner = Scripted {
        replies: vec![
            (
                "--pretend --update",
                true,
                "[binary   N     ] app-editors/neovim-0.11.5::gentoo\n".into(),
            ),
            (
                "--pretend --verbose --color=n --ask=n app-shells/zsh",
                false,
                MASKED.into(),
            ),
        ],
        ..Scripted::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &quiet(true, true))
        .unwrap_err()
        .to_string();
    assert!(err.contains("nothing was changed"), "{err}");
    assert!(err.contains("app-shells/zsh"), "{err}");
    assert!(!runner.commands.iter().any(|c| c.contains("--quiet-build")));
    assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
}

#[test]
fn saying_no_to_extra_packages_puts_everything_back() {
    let root = fake_system();
    let use_dir = root.path().join(portage::PACKAGE_USE_DIR);
    let world = portage::world_path(root.path());
    let world_before = std::fs::read_to_string(&world).unwrap();
    let files_before: Vec<_> = std::fs::read_dir(&use_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        replies: vec![("--pretend --update", true, PRETEND.into())],
        ..Scripted::default()
    };
    let err = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(false, false),
    )
    .unwrap_err();
    assert!(err.is::<Cancelled>());
    assert!(
        runner
            .commands
            .iter()
            .any(|c| c.starts_with("btrfs subvolume delete")),
        "the empty generation is deleted: {:?}",
        runner.commands
    );
    assert!(!runner.commands.iter().any(|c| c.contains("--quiet-build")));
    assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
    let mut files_after: Vec<_> = std::fs::read_dir(&use_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let mut files_before = files_before;
    files_before.sort();
    files_after.sort();
    assert_eq!(files_after, files_before);
}

#[test]
fn a_refusal_before_installing_changes_nothing_and_is_explained() {
    let root = fake_system();
    let world = portage::world_path(root.path());
    let world_before = std::fs::read_to_string(&world).unwrap();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        replies: vec![("--pretend --update", false, fixture("use-change-swtpm.txt"))],
        ..Scripted::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &quiet(true, true))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("before installing anything, so nothing was changed"),
        "{err}"
    );
    assert!(err.contains("gnutls"), "{err}");
    assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
}

#[test]
fn a_refusal_is_explained_from_the_helpers_data() {
    let root = fake_system();
    let world = portage::world_path(root.path());
    let world_before = std::fs::read_to_string(&world).unwrap();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    // The helper says why; emerge's text is still asked for the log, and
    // says something only it knows about (a slot conflict).
    let text = format!(
        "{}\n\n{}",
        fixture("keyword-testing.txt"),
        fixture("slot-conflict.txt")
    );
    let mut runner = Scripted {
        replies: vec![("--pretend --update", false, text)],
        helper: vec![(
            "plan",
            r#"{"ok": false, "reason": "portage couldn't work out a plan", "problems": [{"kind": "keywords", "name": "dev-qt/qtbase", "version": "6.10.1", "keyword": "~amd64", "required_by": ["gui-apps/quickshell", "gui-wm/hyprland"]}], "helper": 3}"#
                .into(),
        )],
        ..Scripted::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &quiet(true, true))
        .unwrap_err()
        .to_string();
    assert!(runner.commands.iter().any(|c| c == "(helper) plan"));
    assert!(
        err.contains("before installing anything, so nothing was changed"),
        "{err}"
    );
    // From the data: the chain the text doesn't have, and only once.
    assert!(err.contains("quickshell → hyprland"), "{err}");
    assert_eq!(
        err.matches("only available as a testing version").count(),
        1,
        "{err}"
    );
    // From the text: the kind the helper didn't report.
    for e in crate::explain::explain(&fixture("slot-conflict.txt")) {
        assert!(err.contains(&e.title), "{err}");
    }
    assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
}

#[test]
fn the_profile_changes_before_emerge_and_comes_back_when_cancelled() {
    const PLAIN: &str = "../../var/db/repos/gentoo/profiles/default/linux/amd64/23.0";
    let link = |root: &std::path::Path| {
        std::fs::read_link(root.join(crate::profile::LINK))
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    let config = Config::parse(&format!("{CONFIG}\n[system]\nprofile = \"desktop\"\n")).unwrap();

    // Said no at emerge's list: the profile goes back.
    let root = fake_system();
    std::os::unix::fs::symlink(PLAIN, root.path().join(crate::profile::LINK)).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    assert!(
        plan.render().contains("Profile: minimal → desktop"),
        "{}",
        plan.render()
    );
    assert!(
        plan.summary().contains("desktop profile"),
        "{}",
        plan.summary()
    );
    // A new profile changes features, so emerge rebuilds more than the plan
    // listed: that's what makes it ask.
    let list = "[binary  N    ] app-shells/zsh-5.9::gentoo\n\
                [binary   R    ] dev-libs/glib-2.86.3::gentoo  USE=\"dbus*\"\n";
    let mut runner = Scripted {
        replies: vec![("--pretend", true, list.into())],
        ..Scripted::default()
    };
    let result = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(false, false),
    );
    assert!(result.unwrap_err().is::<super::Cancelled>());
    assert_eq!(link(root.path()), PLAIN);

    // Said yes: emerge plans with the desktop profile already in place.
    let root = fake_system();
    std::os::unix::fs::symlink(PLAIN, root.path().join(crate::profile::LINK)).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        replies: vec![("--pretend", true, list.into())],
        ..Scripted::default()
    };
    switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
    assert_eq!(link(root.path()), format!("{PLAIN}/desktop"));
    // Nothing left to do for it afterwards.
    assert!(matches!(
        plan::make(&config, root.path()).unwrap().profile,
        crate::profile::Wanted::Nothing
    ));
}

#[test]
fn a_new_profile_alone_runs_emerge() {
    // Nothing else changes, but the profile's defaults do: emerge has to
    // bring the packages in line, or the system only says "desktop".
    let plan = Plan {
        profile: crate::profile::Wanted::Change(crate::profile::Change {
            from: crate::profile::Profile::Minimal,
            to: crate::profile::Profile::Desktop,
            target: "../../var/db/repos/gentoo/profiles/default/linux/amd64/23.0/desktop".into(),
        }),
        ..Plan::default()
    };
    assert!(super::needs_emerge(&plan, &opts()));
    assert!(!super::needs_emerge(&Plan::default(), &opts()));
}

#[test]
fn a_refusal_the_resolver_can_fix_asks_for_its_settings() {
    let root = fake_system();
    let world = portage::world_path(root.path());
    let world_before = std::fs::read_to_string(&world).unwrap();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        replies: vec![("--pretend --update", false, fixture("keyword-testing.txt"))],
        helper: vec![
            (
                "plan",
                r#"{"ok": false, "reason": "portage couldn't work out a plan", "problems": [], "helper": 4}"#
                    .into(),
            ),
            (
                "resolve",
                r#"{"ok": true, "resolved": true, "use": {"dev-qt/qtbase": {"wayland": true}}, "keywords": {"dev-qt/qtbase": "~amd64"}, "compile": [], "why": {}, "rounds": 2, "helper": 4}"#
                    .into(),
            ),
        ],
        ..Scripted::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap_err();
    assert!(runner.commands.iter().any(|c| c == "(helper) resolve"));
    let resolution = &err
        .downcast_ref::<super::NeedsChanges>()
        .expect("NeedsChanges")
        .0;
    assert!(resolution.use_flags["dev-qt/qtbase"]["wayland"]);
    assert!(super::changed_nothing(&err));
    // Nothing was installed, and the files were put back.
    assert!(!runner
        .commands
        .iter()
        .any(|c| c.starts_with("emerge --update") && !c.contains("--pretend")));
    assert_eq!(std::fs::read_to_string(&world).unwrap(), world_before);
}

#[test]
fn packages_to_compile_are_never_taken_prebuilt() {
    let root = fake_system();
    let config = Config::parse(&format!(
        "{CONFIG}\n[pkg.\"dev-util/shader-slang\"]\nprebuilt = false\n"
    ))
    .unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    assert_eq!(plan.compile_from_source, ["dev-util/shader-slang"]);
    let mut runner = Scripted::default();
    let _ = switch(&config, root.path(), &plan, &mut runner, &quiet(true, true));
    let c = &runner.commands;
    assert!(
        c.iter()
            .any(|l| l == "(helper) plan --usepkg-exclude=dev-util/shader-slang"),
        "{c:#?}"
    );
    let emerges: Vec<&String> = c
        .iter()
        .filter(|l| l.starts_with("emerge ") && l.contains("--update"))
        .collect();
    assert!(!emerges.is_empty(), "{c:#?}");
    assert!(
        emerges
            .iter()
            .all(|l| l.contains("--usepkg-exclude=dev-util/shader-slang")),
        "{c:#?}"
    );
}

#[test]
fn quiet_removal_lists_and_always_asks() {
    let root = fake_system();
    let config = Config::parse("packages = []").unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    assert!(!plan.remove_packages.is_empty());
    let depclean = "All selected packages: =app-text/tree-2.2.1\n\nNumber to remove:     1\n";
    // --yes, but the user says no at the removal question.
    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, depclean.into())],
        ..Scripted::default()
    };
    let outcome = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(true, false),
    )
    .unwrap();
    assert!(!runner
        .commands
        .iter()
        .any(|c| c == "emerge --depclean --color=n --ask=n"));
    assert!(outcome.notes.iter().any(|n| n.contains("weren't removed")));

    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, depclean.into())],
        ..Scripted::default()
    };
    switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
    assert!(runner
        .commands
        .iter()
        .any(|c| c == "emerge --depclean --color=n --ask=n"));
}

#[test]
fn a_plain_remove_asks_once() {
    // The plan showed "- app-misc/old" and the user said yes to it (not
    // --yes). emerge wants to remove exactly that, so bunny doesn't ask a
    // second time. (The answer here would be no, so a second question
    // would leave it installed.)
    let root = fake_system();
    let config = Config::parse(r#"packages = ["app-editors/neovim"]"#).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    assert_eq!(plan.remove_packages, ["app-misc/old"]);
    let just_that = "All selected packages: =app-misc/old-1.0\n\nNumber to remove:     1\n";
    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, just_that.into())],
        ..Scripted::default()
    };
    let outcome = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(false, false),
    )
    .unwrap();
    assert!(
        runner
            .commands
            .iter()
            .any(|c| c == "emerge --depclean --color=n --ask=n"),
        "{:#?}",
        runner.commands
    );
    assert!(!outcome.notes.iter().any(|n| n.contains("weren't removed")));

    // emerge wants more than the plan showed: ask (and the answer is no).
    let more = "All selected packages: =app-misc/old-1.0 =dev-libs/libold-2\n\n\
                Number to remove:     2\n";
    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, more.into())],
        ..Scripted::default()
    };
    let outcome = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(false, false),
    )
    .unwrap();
    assert!(!runner
        .commands
        .iter()
        .any(|c| c == "emerge --depclean --color=n --ask=n"));
    assert!(outcome.notes.iter().any(|n| n.contains("weren't removed")));
}

#[test]
fn compares_the_removal_list_with_the_plan() {
    use super::install::same_as_planned;
    let r = |name: &str| crate::helper::Removal {
        name: name.into(),
        version: "1".into(),
        needed_by: vec![],
    };
    let planned = vec!["app-text/tree".to_string()];
    assert!(same_as_planned(&[r("app-text/tree")], &planned));
    assert!(same_as_planned(
        &[r("app-text/tree")],
        &["app-text/tree:0".into()]
    ));
    assert!(!same_as_planned(
        &[r("app-text/tree"), r("dev-libs/x")],
        &planned
    ));
    assert!(!same_as_planned(&[r("dev-libs/x")], &planned));
    assert!(!same_as_planned(&[], &[]));
}

const HELPER_DEPCLEAN: &str = r#"{"ok": true, "removals": [{"name": "acct-user/nullmail", "version": "0-r2", "slot": "0", "needed_by": ["mail-mta/nullmailer"]}, {"name": "mail-mta/nullmailer", "version": "2.2-r2", "slot": "0", "needed_by": []}], "helper": 2}"#;

#[test]
fn removal_list_comes_from_the_helper_and_only_those_are_removed() {
    let root = fake_system();
    let config = Config::parse("packages = []").unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        helper: vec![("depclean", HELPER_DEPCLEAN.into())],
        ..Scripted::default()
    };
    switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
    let c = &runner.commands;
    assert!(c.iter().any(|l| l == "(helper) depclean"), "{c:#?}");
    // emerge's text isn't needed.
    assert!(
        !c.iter().any(|l| l.contains("--depclean --pretend")),
        "{c:#?}"
    );
    assert!(
        c.iter().any(|l| l
            == "emerge --depclean --color=n --ask=n =acct-user/nullmail-0-r2 =mail-mta/nullmailer-2.2-r2"),
        "{c:#?}"
    );

    // Saying no removes nothing, as before.
    let mut runner = Scripted {
        helper: vec![("depclean", HELPER_DEPCLEAN.into())],
        ..Scripted::default()
    };
    let outcome = switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(true, false),
    )
    .unwrap();
    assert!(!runner
        .commands
        .iter()
        .any(|l| l.starts_with("emerge --depclean --color=n")));
    assert!(outcome.notes.iter().any(|n| n.contains("weren't removed")));
}

#[test]
fn removal_falls_back_to_emerges_text_when_the_helper_cant_answer() {
    let root = fake_system();
    let config = Config::parse("packages = []").unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let depclean = "All selected packages: =app-text/tree-2.2.1\n\nNumber to remove:     1\n";
    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, depclean.into())],
        helper: vec![(
            "depclean",
            r#"{"ok": false, "reason": "portage couldn't work out what to remove", "helper": 2}"#
                .into(),
        )],
        ..Scripted::default()
    };
    switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
    let c = &runner.commands;
    let asked = c.iter().position(|l| l == "(helper) depclean").unwrap();
    let pretend = c
        .iter()
        .position(|l| l.contains("--depclean --pretend"))
        .unwrap();
    assert!(asked < pretend, "{c:#?}");
    // The text path removes as it always did: emerge's own full list.
    assert!(
        c.iter().any(|l| l == "emerge --depclean --color=n --ask=n"),
        "{c:#?}"
    );
}

#[test]
fn removal_list_says_why_when_portage_does() {
    let list = crate::helper::parse_depclean(HELPER_DEPCLEAN).unwrap();
    let text = super::install::describe_removals(&list);
    assert_eq!(
        text,
        "\nThese packages aren't needed anymore (nothing you asked for uses them):\n  \
         - acct-user/nullmail   0-r2    (only mail-mta/nullmailer used it)\n  \
         - mail-mta/nullmailer  2.2-r2\n"
    );
}

#[test]
fn unreadable_depclean_output_falls_back_to_emerges_own_question() {
    let root = fake_system();
    let config = Config::parse("packages = []").unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, "Number to remove: 3\n".into())],
        ..Scripted::default()
    };
    switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
    assert!(runner
        .commands
        .iter()
        .any(|c| c == "emerge --depclean --ask"));
}

#[test]
fn system_runner_runs_quietly_line_by_line() {
    let cmd = cmd(&[
        "sh",
        "-c",
        "echo '>>> Emerging (1 of 2) a/b-1'; printf 'bad \\377 byte\\n'; echo err >&2; echo '>>> Emerging (2 of 2) a/c-2'; exit 1",
    ]);
    let mut seen = Vec::new();
    let (ok, text) = SystemRunner
        .run_quiet(&cmd, &mut |l| seen.push(l.to_string()))
        .unwrap();
    assert!(!ok);
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(seen.contains(&">>> Emerging (2 of 2) a/c-2".to_string()));
    assert!(text.contains("bad"));
    assert!(text.contains("err"));
}

#[test]
fn no_second_question_when_emerge_wants_only_what_the_plan_showed() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let mut plan = plan::make(&config, root.path()).unwrap();
    plan.add_packages.clear();
    plan.use_changes = vec![crate::plan::UseChange {
        atom: "app-admin/sudo".into(),
        from: vec![],
        to: vec!["-sendmail".into()],
    }];
    let mut runner = Scripted {
        replies: vec![(
            "--pretend --update",
            true,
            "[ebuild   R    ] app-admin/sudo-1.9.17_p2::gentoo\n".into(),
        )],
        ..Scripted::default()
    };
    // The answer would be "no" if asked; it must not be asked.
    switch(
        &config,
        root.path(),
        &plan,
        &mut runner,
        &quiet(false, false),
    )
    .unwrap();
    assert!(runner
        .commands
        .iter()
        .any(|c| c.contains("--quiet-build=y")));
}

/// A system set up for the boot menu's undo entry: GRUB, dracut, one
/// kernel, booted with root=UUID=.
fn bootable_system() -> tempfile::TempDir {
    let dir = fake_system();
    let r = dir.path();
    write(r, "boot/grub/grub.cfg", "");
    write(r, "usr/bin/dracut", "");
    write(r, "boot/vmlinuz-6.12.58", "kernel");
    write(r, "boot/initramfs-6.12.58.img", "old image");
    std::fs::create_dir_all(r.join("lib/modules/6.12.58")).unwrap();
    write(
        r,
        "proc/cmdline",
        "BOOT_IMAGE=/@/boot/vmlinuz-6.12.58 root=UUID=1234-abcd ro rootflags=subvol=@\n",
    );
    write(r, "proc/sys/kernel/osrelease", "6.12.58\n");
    dir
}

const BOOT_CONFIG: &str = "packages = [\"app-editors/neovim\"]\n[boot]\nundo_entry = true\n";

/// Pretends to be btrfs and dracut: the snapshot copies the system's
/// /boot and dracut module into /.snapshots, and dracut writes an image.
struct Builder {
    root: PathBuf,
    dracut_ok: bool,
    commands: Vec<String>,
    /// /boot is its own filesystem, so snapshots don't include it.
    separate_boot: bool,
}

impl Runner for Builder {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        self.commands.push(cmd.join(" "));
        Ok(true)
    }

    fn run_quiet(&mut self, cmd: &[String], _: &mut dyn FnMut(&str)) -> Result<(bool, String)> {
        self.commands.push(cmd.join(" "));
        match cmd[0].as_str() {
            "dracut" if self.dracut_ok => {
                std::fs::write(&cmd[2], "new image with the undo step").unwrap();
                Ok((true, String::new()))
            }
            "dracut" => Ok((false, "dracut: something went wrong".into())),
            "btrfs" => {
                let target = PathBuf::from(&cmd[5]);
                // Module first: copying sets new times, and the image must
                // look newer than the module, as it is on a real snapshot.
                for sub in [crate::boot::MODULE_DIR, "boot"] {
                    let from = self.root.join(sub);
                    if from.exists() && !(self.separate_boot && sub == "boot") {
                        copy_tree(&from, &target.join(sub)).unwrap();
                    }
                }
                if self.separate_boot {
                    // A real snapshot keeps the module's time, which is
                    // older than the startup image built from it.
                    let module = target
                        .join(crate::boot::MODULE_DIR)
                        .join("bunny-rollback.sh");
                    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
                    std::fs::File::options()
                        .write(true)
                        .open(module)
                        .unwrap()
                        .set_modified(old)
                        .unwrap();
                }
                Ok((true, String::new()))
            }
            _ => Ok((true, String::new())),
        }
    }
}

#[test]
fn undo_entry_is_set_up_in_one_switch() {
    let root = bootable_system();
    let r = root.path();
    let config = Config::parse(BOOT_CONFIG).unwrap();
    let plan = plan::make(&config, r).unwrap();
    assert!(plan.boot.install_module.is_some(), "{:?}", plan.boot);
    let mut runner = Builder {
        root: r.to_path_buf(),
        dracut_ok: true,
        commands: vec![],
        separate_boot: false,
    };
    let outcome = switch(&config, r, &plan, &mut runner, &opts()).unwrap();

    // Module first, then the snapshot (so the snapshot has it).
    let dracut = runner
        .commands
        .iter()
        .position(|c| c.starts_with("dracut"))
        .unwrap();
    let snap = runner
        .commands
        .iter()
        .position(|c| c.starts_with("btrfs"))
        .unwrap();
    assert!(dracut < snap, "{:?}", runner.commands);
    assert_eq!(
        std::fs::read_to_string(r.join("boot/initramfs-6.12.58.img")).unwrap(),
        "new image with the undo step"
    );
    assert!(outcome.backup_dir.join("initramfs-6.12.58.img").exists());
    // The start-up service that finishes an undo from the boot menu.
    assert!(r.join("etc/init.d/bunny-finish-undo").is_file());
    assert!(std::fs::symlink_metadata(r.join("etc/runlevels/boot/bunny-finish-undo")).is_ok());

    let cfg = std::fs::read_to_string(r.join(crate::boot::BUNNY_CFG)).unwrap();
    assert!(cfg.contains("linux /@snapshots/bunny-20261116-120000/boot/vmlinuz-6.12.58 root=UUID=1234-abcd ro rootflags=subvol=@ bunny.rollback=bunny-20261116-120000"), "{cfg}");
    assert!(cfg.contains("menuentry 'Undo the last change ("), "{cfg}");
    // Earlier versions lists the generation too, with Go back to it.
    assert!(
        cfg.contains("submenu 'Earlier versions (1 saved)'"),
        "{cfg}"
    );
    assert!(
        cfg.contains("menuentry 'Go back to this version ("),
        "{cfg}"
    );
    // GRUB reads bunny.cfg right after the system's own entries.
    let script = std::fs::read_to_string(r.join(crate::boot::GRUBD_SCRIPT)).unwrap();
    assert!(script.contains("bunny.cfg"));
    assert!(
        runner
            .commands
            .iter()
            .any(|c| c.starts_with("grub-mkconfig -o ")),
        "{:?}",
        runner.commands
    );
    assert!(
        outcome
            .notes
            .iter()
            .any(|n| n.contains("Undo the last change")),
        "{:?}",
        outcome.notes
    );

    // Next time the module is already there: nothing to plan for [boot].
    assert!(plan::make(&config, r).unwrap().boot.is_empty());
}

/// Gentoo's GRUB files, as a Gentoo install leaves them.
fn add_gentoo_grub(r: &Path) {
    use std::os::unix::fs::PermissionsExt;
    write(
        r,
        "etc/default/grub",
        "GRUB_THEME=\"/usr/share/grub/themes/gentoo_glass/theme.txt\"\nGRUB_DISTRIBUTOR=\"Bunny Linux\"\n",
    );
    for name in ["10_linux", "30_uefi-firmware", "31_efi_bootnext"] {
        let path = format!("etc/grub.d/{name}");
        write(r, &path, "#! /bin/sh\n");
        std::fs::set_permissions(r.join(&path), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
}

#[test]
fn a_switch_tidies_the_boot_menu_of_an_existing_system() {
    use crate::boot::tidy::{self, Tidy};
    let root = bootable_system();
    let r = root.path();
    add_gentoo_grub(r);
    // bunny's own menu script is already there from an earlier switch.
    write(r, crate::boot::GRUBD_SCRIPT, crate::boot::GRUBD_TEXT);
    let config = Config::parse(BOOT_CONFIG).unwrap();
    let plan = plan::make(&config, r).unwrap();
    assert_eq!(
        plan.boot.tidy,
        vec![Tidy::Theme, Tidy::Titles, Tidy::BootNext]
    );
    let shown = plan.render();
    assert!(shown.contains("Boot menu, tidied:"), "{shown}");
    assert!(shown.contains("turn off Gentoo's menu theme"), "{shown}");
    assert!(shown.contains("\"(EFI BootNext)\""), "{shown}");

    let mut runner = Builder {
        root: r.to_path_buf(),
        dracut_ok: true,
        commands: vec![],
        separate_boot: false,
    };
    let outcome = switch(&config, r, &plan, &mut runner, &opts()).unwrap();

    let grub = std::fs::read_to_string(r.join(tidy::DEFAULT_GRUB)).unwrap();
    assert!(grub.contains("#GRUB_THEME="), "{grub}");
    assert!(is_executable(&r.join(tidy::OWN_SCRIPT)));
    assert!(!is_executable(&r.join(tidy::LINUX_SCRIPT)));
    assert!(!is_executable(&r.join(tidy::BOOTNEXT_SCRIPT)));
    assert!(is_executable(&r.join("etc/grub.d/30_uefi-firmware")));
    // Backed up as they were.
    let backup = &outcome.backup_dir;
    assert!(std::fs::read_to_string(backup.join(tidy::DEFAULT_GRUB))
        .unwrap()
        .starts_with("GRUB_THEME="));
    assert!(is_executable(&backup.join(tidy::LINUX_SCRIPT)));
    assert!(is_executable(&backup.join(tidy::BOOTNEXT_SCRIPT)));
    // After the files changed, the menu is rebuilt once.
    let mkconfig: Vec<_> = runner
        .commands
        .iter()
        .filter(|c| c.starts_with("grub-mkconfig -o "))
        .collect();
    assert_eq!(mkconfig.len(), 1, "{:?}", runner.commands);
    // In the snapshot? The files are in /etc, so undo puts them back: the
    // snapshot is taken before they change.
    let snap = runner
        .commands
        .iter()
        .position(|c| c.starts_with("btrfs"))
        .unwrap();
    let rebuilt = runner
        .commands
        .iter()
        .position(|c| c.starts_with("grub-mkconfig"))
        .unwrap();
    assert!(snap < rebuilt);

    assert!(plan::make(&config, r).unwrap().boot.tidy.is_empty());
}

#[test]
fn the_boot_menu_is_left_alone_without_undo_entry() {
    let root = bootable_system();
    let r = root.path();
    add_gentoo_grub(r);
    let config = Config::parse("packages = [\"app-editors/neovim\"]\n").unwrap();
    assert!(plan::make(&config, r).unwrap().boot.tidy.is_empty());
}

#[test]
fn a_failed_startup_image_build_changes_nothing() {
    let root = bootable_system();
    let r = root.path();
    let config = Config::parse(BOOT_CONFIG).unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = Builder {
        root: r.to_path_buf(),
        dracut_ok: false,
        commands: vec![],
        separate_boot: false,
    };
    let err = switch(&config, r, &plan, &mut runner, &opts())
        .unwrap_err()
        .to_string();
    assert!(err.contains("Couldn't rebuild the startup image"), "{err}");
    assert!(!runner.commands.iter().any(|c| c.starts_with("btrfs")));
    assert!(!r.join(crate::boot::MODULE_DIR).exists());
    assert_eq!(
        std::fs::read_to_string(r.join("boot/initramfs-6.12.58.img")).unwrap(),
        "old image"
    );
    assert!(!r.join(crate::boot::CUSTOM_CFG).exists());
}

#[test]
fn undo_entry_boots_the_kernel_kept_on_a_separate_boot() {
    let root = bootable_system();
    let r = root.path();
    // /boot is the EFI partition; / is encrypted btrfs.
    write(
        r,
        "proc/self/mountinfo",
        "22 1 0:21 /@ / rw shared:1 - btrfs /dev/mapper/root rw,subvol=/@\n\
         24 22 259:1 / /boot rw shared:3 - vfat /dev/vda1 rw\n",
    );
    write(r, "dev/vda1", "");
    std::fs::create_dir_all(r.join("dev/disk/by-uuid")).unwrap();
    std::os::unix::fs::symlink("../../vda1", r.join("dev/disk/by-uuid/ABCD-1234")).unwrap();
    write(r, "boot/intel-uc.img", "microcode");

    let config = Config::parse(BOOT_CONFIG).unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = Builder {
        root: r.to_path_buf(),
        dracut_ok: true,
        commands: vec![],
        separate_boot: true,
    };
    let outcome = switch(&config, r, &plan, &mut runner, &opts()).unwrap();

    // The generation's kernel files are kept on /boot.
    let saved = crate::kernels::load(r, "bunny-20261116-120000").unwrap();
    assert_eq!(saved.len(), 3, "{saved:?}");
    assert_eq!(
        std::fs::read_to_string(r.join("boot/bunny/files/initramfs-6.12.58.img")).unwrap(),
        "new image with the undo step"
    );

    let cfg = std::fs::read_to_string(r.join(crate::boot::BUNNY_CFG)).unwrap();
    assert!(
        cfg.contains("search --no-floppy --fs-uuid --set=root ABCD-1234"),
        "{cfg}"
    );
    assert!(cfg.contains("linux /bunny/files/vmlinuz-6.12.58 root=UUID=1234-abcd ro rootflags=subvol=@ bunny.rollback=bunny-20261116-120000"), "{cfg}");
    assert!(
        cfg.contains("initrd /bunny/files/intel-uc.img /bunny/files/initramfs-6.12.58.img"),
        "{cfg}"
    );
    assert!(
        outcome
            .notes
            .iter()
            .any(|n| n.contains("Undo the last change")),
        "{:?}",
        outcome.notes
    );
}

/// Records commands, answers `wget` with Gentoo's list of repositories, and
/// "downloads" a repository by making its folder with one package in it.
struct WithRepos {
    root: PathBuf,
    commands: Vec<String>,
    /// What `emaint sync` says when run quietly, and whether it works.
    emaint_says: Option<(bool, &'static str)>,
}

const REPO_LIST: &str = r#"<repositories version="1.0">
  <repo quality="experimental" status="official"><name>guru</name>
    <description lang="en">Gentoo User Repository</description>
    <source type="git">https://anongit.gentoo.org/git/repo/proj/guru.git</source></repo>
</repositories>"#;

impl Runner for WithRepos {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        let line = cmd.join(" ");
        if let Some(name) = line.strip_prefix("emaint sync --repo ") {
            write(
                &self.root,
                &format!("var/db/repos/{name}/metadata/md5-cache/gui-wm/mangowm-0.12.5"),
                "DESCRIPTION=A Wayland compositor\n",
            );
        }
        self.commands.push(line);
        Ok(true)
    }
    fn run_quiet(&mut self, cmd: &[String], _: &mut dyn FnMut(&str)) -> Result<(bool, String)> {
        let ok = self.run(cmd)?;
        Ok(match (cmd[0].as_str(), self.emaint_says) {
            ("emaint", Some((ok, said))) => (ok, said.to_string()),
            _ => (ok, String::new()),
        })
    }
    fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
        self.commands.push(cmd.join(" "));
        Ok(cmd
            .first()
            .is_some_and(|c| c == "wget")
            .then(|| REPO_LIST.to_string()))
    }
}

const WITH_GURU: &str = r#"
    packages = ["app-editors/neovim", "gui-wm/mangowm"]
    repositories = ["guru"]
    [pkg."gui-wm/mangowm"]
    keywords = ["~amd64"]
"#;

#[test]
fn adds_a_repository_and_testing_versions() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/bin/git", "");
    write(
        r,
        "var/db/repos/gentoo/metadata/md5-cache/app-editors/neovim-0.11",
        "DESCRIPTION=Vim fork\n",
    );
    // A package.accept_keywords that's a single file moves into the folder.
    write(r, portage::KEYWORDS_DIR, "dev-libs/mine ~amd64\n");
    let config = Config::parse(WITH_GURU).unwrap();
    let plan = plan::make(&config, r).unwrap();
    assert_eq!(plan.add_repositories, ["guru"]);
    assert!(plan.check_after_sync);
    assert!(
        plan.unknown_packages.is_empty(),
        "checked after the download"
    );
    assert_eq!(plan.keyword_changes[0].to, ["~amd64"]);
    let shown = plan.render();
    assert!(shown.contains("Repositories to add:\n  + guru"), "{shown}");
    assert!(
        shown.contains("aren't checked by Gentoo's developers"),
        "{shown}"
    );
    assert!(
        shown.contains("~ gui-wm/mangowm: stable only → ~amd64"),
        "{shown}"
    );
    assert!(
        plan.summary().starts_with("+ repository guru"),
        "{}",
        plan.summary()
    );

    let mut runner = WithRepos {
        root: r.to_path_buf(),
        commands: Vec::new(),
        emaint_says: None,
    };
    switch(&config, r, &plan, &mut runner, &opts()).unwrap();
    let c = &runner.commands;
    let sync = c
        .iter()
        .position(|l| l == "emaint sync --repo guru")
        .expect("synced");
    let emerge = c
        .iter()
        .position(|l| l.starts_with("emerge --update"))
        .expect("installed");
    assert!(
        sync < emerge,
        "the repository is downloaded before installing: {c:#?}"
    );

    let conf = std::fs::read_to_string(portage::repo_conf_path(r, "guru")).unwrap();
    assert!(conf.contains("[guru]\nlocation = /var/db/repos/guru\nsync-type = git\nsync-uri = https://anongit.gentoo.org/git/repo/proj/guru.git"), "{conf}");
    let kw = r.join(portage::KEYWORDS_DIR);
    assert!(
        std::fs::read_to_string(kw.join(portage::BUNNY_KEYWORDS_FILE))
            .unwrap()
            .contains("gui-wm/mangowm ~amd64")
    );
    assert_eq!(
        std::fs::read_to_string(kw.join(portage::KEPT_KEYWORDS_FILE)).unwrap(),
        "dev-libs/mine ~amd64\n"
    );

    // Afterwards nothing is left to do for repositories or keywords.
    let after = plan::make(&config, r).unwrap();
    assert!(after.add_repositories.is_empty() && after.keyword_changes.is_empty());
    assert!(!after.check_after_sync);

    // Taking guru out of the config stops bunny using it.
    let without = Config::parse("packages = [\"app-editors/neovim\"]").unwrap();
    let plan = plan::make(&without, r).unwrap();
    assert_eq!(plan.remove_repositories, ["guru"]);
    assert_eq!(plan.keyword_changes[0].from, ["~amd64"]);
    let outcome = switch(&without, r, &plan, &mut runner, &opts()).unwrap();
    assert!(!portage::repo_conf_path(r, "guru").exists());
    assert!(!kw.join(portage::BUNNY_KEYWORDS_FILE).exists());
    assert!(outcome
        .notes
        .iter()
        .any(|n| n.contains("stopped using the guru repository")));
}

/// What `emaint sync --repo guru` prints for a first download.
const EMAINT_GURU: &str = "Cloning into '/var/db/repos/guru'...\n\
     Performing Global Updates\n\
     (Could take a couple of minutes if you have a lot of binary packages.)\n\
     /var/db/repos/gentoo/profiles/updates/1Q-2026\n\
     Action: sync for repo: guru, returned code = 0\n";

#[test]
fn downloading_a_repository_keeps_emaints_output_in_the_log() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/bin/git", "");
    let config = Config::parse(WITH_GURU).unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = WithRepos {
        root: r.to_path_buf(),
        commands: Vec::new(),
        emaint_says: Some((true, EMAINT_GURU)),
    };
    let opts = quiet(true, true);
    // The rest of the switch (emerge) isn't what this test is about.
    let _ = switch(&config, r, &plan, &mut runner, &opts);
    assert!(runner
        .commands
        .iter()
        .any(|l| l == "emaint sync --repo guru"));
    let log = r.join(format!("var/log/bunny/{}-download-guru.log", opts.stamp));
    let kept = std::fs::read_to_string(&log).expect("emaint's output is in the log");
    assert!(kept.starts_with("$ emaint sync --repo guru\n"), "{kept}");
    assert!(kept.contains("Performing Global Updates"), "{kept}");
    assert!(kept.contains("returned code = 0"), "{kept}");
}

#[test]
fn a_failed_repository_download_shows_the_end_and_where_the_log_is() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/bin/git", "");
    let config = Config::parse(WITH_GURU).unwrap();
    let plan = plan::make(&config, r).unwrap();
    // emaint exits 0 here, but its summary line says the sync failed.
    let mut runner = WithRepos {
        root: r.to_path_buf(),
        commands: Vec::new(),
        emaint_says: Some((
            true,
            "fatal: unable to access 'https://anongit.gentoo.org/git/repo/proj/guru.git/': \
             Could not resolve host: anongit.gentoo.org\n\
             Action: sync for repo: guru, returned code = 128\n",
        )),
    };
    let opts = quiet(true, true);
    let err = switch(&config, r, &plan, &mut runner, &opts)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Couldn't download the guru repository"),
        "{err}"
    );
    assert!(err.contains("Could not resolve host"), "{err}");
    assert!(err.contains("Check that this computer is online"), "{err}");
    assert!(
        err.contains(&format!("{}-download-guru.log", opts.stamp)),
        "{err}"
    );
    assert!(!runner.commands.iter().any(|l| l.starts_with("emerge")));
}

#[test]
fn a_misspelled_package_is_caught_after_the_new_repository_arrives() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/bin/git", "");
    let config =
        Config::parse("packages = [\"gui-wm/mangowmm\"]\nrepositories = [\"guru\"]\n").unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = WithRepos {
        root: r.to_path_buf(),
        commands: Vec::new(),
        emaint_says: None,
    };
    let err = switch(&config, r, &plan, &mut runner, &opts())
        .unwrap_err()
        .to_string();
    assert!(err.contains("gui-wm/mangowmm"), "{err}");
    assert!(err.contains("nothing was installed"), "{err}");
    assert!(!runner.commands.iter().any(|l| l.starts_with("emerge")));
}

#[test]
fn bunny_linuxs_own_repository_needs_no_list_and_no_warning() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/bin/git", "");
    let config = Config::parse("repositories = [\"bunny\"]\n").unwrap();
    assert!(config.problems().is_empty());
    let plan = plan::make(&config, r).unwrap();
    assert_eq!(plan.add_repositories, ["bunny"]);
    let shown = plan.render();
    assert!(shown.contains("Repositories to add:\n  + bunny"), "{shown}");
    assert!(!shown.contains("aren't checked"), "{shown}");
    let mut runner = WithRepos {
        root: r.to_path_buf(),
        commands: Vec::new(),
        emaint_says: None,
    };
    switch(&config, r, &plan, &mut runner, &opts()).unwrap();
    // Gentoo's list isn't downloaded: bunny knows where its own lives.
    assert!(
        !runner.commands.iter().any(|c| c.starts_with("wget")),
        "{:#?}",
        runner.commands
    );
    assert!(runner
        .commands
        .iter()
        .any(|c| c == "emaint sync --repo bunny"));
    let conf = std::fs::read_to_string(portage::repo_conf_path(r, "bunny")).unwrap();
    assert!(
        conf.contains("sync-uri = https://github.com/bunnyos-dev/bunny-overlay.git"),
        "{conf}"
    );

    // With another repository too, the list is fetched for that one, and
    // the warning is about it.
    let both = Config::parse("repositories = [\"bunny\", \"guru\"]\n").unwrap();
    let plan = plan::make(&both, r).unwrap();
    assert_eq!(plan.add_repositories, ["guru"]);
    assert!(plan.render().contains("aren't checked"));
}

#[test]
fn says_so_when_a_repository_isnt_in_gentoos_list() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/bin/git", "");
    let config = Config::parse("repositories = [\"nosuchrepo\"]\n").unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = WithRepos {
        root: r.to_path_buf(),
        commands: Vec::new(),
        emaint_says: None,
    };
    let err = switch(&config, r, &plan, &mut runner, &opts())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("no repository called \"nosuchrepo\" in Gentoo's list"),
        "{err}"
    );
    assert!(!portage::repo_conf_path(r, "nosuchrepo").exists());
}

#[test]
fn keywords_are_put_back_when_the_switch_is_cancelled() {
    let root = fake_system();
    let r = root.path();
    let config = Config::parse(
        "packages = [\"app-editors/neovim\", \"app-shells/zsh\"]\n[pkg.\"app-shells/zsh\"]\nkeywords = [\"~amd64\"]\n",
    )
    .unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = Scripted {
        replies: vec![("--pretend --update", true, PRETEND.into())],
        ..Scripted::default()
    };
    let err = switch(&config, r, &plan, &mut runner, &quiet(false, false)).unwrap_err();
    assert!(err.is::<Cancelled>());
    assert!(
        !r.join(portage::KEYWORDS_DIR).exists(),
        "no keywords file was left"
    );
}

#[test]
fn errors_say_whether_anything_changed() {
    let early = nothing_changed(anyhow::anyhow!("the snapshot failed"));
    assert!(changed_nothing(&early));
    assert_eq!(early.to_string(), "the snapshot failed");
    assert!(changed_nothing(&anyhow::Error::new(Cancelled)));
    assert!(!changed_nothing(&anyhow::anyhow!("emerge stopped")));
    // Marking twice keeps the original kind.
    assert!(nothing_changed(anyhow::Error::new(Cancelled)).is::<Cancelled>());
}

#[test]
fn a_failed_snapshot_counts_as_nothing_changed() {
    let root = fake_system();
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, root.path()).unwrap();
    let mut runner = Recorder {
        fail: Some("snapshot"),
        ..Recorder::default()
    };
    let err = switch(&config, root.path(), &plan, &mut runner, &opts()).unwrap_err();
    assert!(changed_nothing(&err), "{err:#}");
}

#[test]
fn a_quiet_switch_keeps_openrcs_lines_in_the_services_log() {
    let root = fake_system();
    let r = root.path();
    write(r, "usr/share/consolefonts/eurlatgr.psfu.gz", "font");
    write(r, "etc/init.d/consolefont", "");
    write(r, "etc/conf.d/consolefont", "consolefont=\"default8x16\"\n");
    let config = Config::parse(CONFIG).unwrap();
    let plan = plan::make(&config, r).unwrap();
    let mut runner = Scripted {
        replies: vec![
            ("--pretend --update", true, PRETEND.into()),
            ("--quiet-build=y", true, INSTALL.into()),
            (
                "rc-update add",
                true,
                " * service sshd added to runlevel default\n".into(),
            ),
            (
                "rc-service consolefont",
                true,
                " * WARNING: you are stopping a boot service\n \
                 * Caching service dependencies ... [ ok ]\n"
                    .into(),
            ),
            // sshd doesn't start: its note points to the log.
            (
                "rc-service sshd",
                false,
                " * ERROR: sshd failed to start\n".into(),
            ),
        ],
        ..Scripted::default()
    };
    let outcome = switch(&config, r, &plan, &mut runner, &quiet(true, true)).unwrap();
    let c = &runner.commands;
    for wanted in [
        "rc-update add sshd default",
        "rc-service sshd start",
        "rc-update add consolefont boot",
        "rc-service consolefont restart",
    ] {
        assert!(c.iter().any(|l| l == wanted), "{wanted}: {c:#?}");
    }
    let log = r.join("var/log/bunny/20261116-120000-services.log");
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("$ rc-update add sshd default\n * service sshd added"));
    assert!(text.contains("WARNING: you are stopping a boot service"));
    assert!(text.contains("$ rc-service sshd start\n * ERROR: sshd failed to start\n(failed)"));
    let notes = outcome.notes.join("\n");
    assert!(
        notes.contains("\"sshd\" will start at boot, but didn't start now"),
        "{notes}"
    );
    assert!(
        notes.contains("What OpenRC printed is in /var/log/bunny/20261116-120000-services.log."),
        "{notes}"
    );
}
