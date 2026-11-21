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

    let early = interrupted_message(None, &[], &Outcome::default());
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
                       [binary   N     ] app-misc/tmux-3.6a::gentoo\n";
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
    assert!(!emerges[1].contains("--ask"), "{}", emerges[1]);
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
        .any(|c| c == "emerge --depclean --color=n"));
    assert!(outcome.notes.iter().any(|n| n.contains("weren't removed")));

    let mut runner = Scripted {
        replies: vec![("--depclean --pretend", true, depclean.into())],
        ..Scripted::default()
    };
    switch(&config, root.path(), &plan, &mut runner, &quiet(true, true)).unwrap();
    assert!(runner
        .commands
        .iter()
        .any(|c| c == "emerge --depclean --color=n"));
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

    let cfg = std::fs::read_to_string(r.join(crate::boot::CUSTOM_CFG)).unwrap();
    assert!(cfg.contains("linux /@snapshots/bunny-20261116-120000/boot/vmlinuz-6.12.58 root=UUID=1234-abcd ro rootflags=subvol=@ bunny.rollback=bunny-20261116-120000"), "{cfg}");
    assert!(
        outcome
            .notes
            .iter()
            .any(|n| n.contains("undo the last change")),
        "{:?}",
        outcome.notes
    );

    // Next time the module is already there: nothing to plan for [boot].
    assert!(plan::make(&config, r).unwrap().boot.is_empty());
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

    let cfg = std::fs::read_to_string(r.join(crate::boot::CUSTOM_CFG)).unwrap();
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
            .any(|n| n.contains("undo the last change")),
        "{:?}",
        outcome.notes
    );
}

/// Records commands, answers `wget` with Gentoo's list of repositories, and
/// "downloads" a repository by making its folder with one package in it.
struct WithRepos {
    root: PathBuf,
    commands: Vec<String>,
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
    };
    let err = switch(&config, r, &plan, &mut runner, &opts())
        .unwrap_err()
        .to_string();
    assert!(err.contains("gui-wm/mangowmm"), "{err}");
    assert!(err.contains("nothing was installed"), "{err}");
    assert!(!runner.commands.iter().any(|l| l.starts_with("emerge")));
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
