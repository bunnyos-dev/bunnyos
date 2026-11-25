//! A real install without the tools' own output: each step's `==> ` heading,
//! then one `✓` line when it's done. What the tools printed (wipefs,
//! mkfs, btrfs, cryptsetup, tar, grub-mkconfig…) goes to the install's log
//! instead, and only the end of it is shown when one of them fails.
//! `--verbose` shows everything as it happens, and still keeps the log.
//!
//! A few commands keep the screen, because their output *is* the progress:
//! the stage3 download's progress bar, the part inside the new system
//! (`inside.sh`, with its own `==> ` headings and emerge's quiet output) and
//! the first `bunny switch`, which is already quiet in its own way.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;

use crate::apply::Runner;

/// How many lines of a failed command's output are shown.
const TAIL: usize = 12;

/// Runs the install's commands quietly, keeping their output in a log.
pub struct QuietRunner {
    inner: Box<dyn Runner>,
    verbose: bool,
    log: Option<PathBuf>,
    step: RefCell<Option<Step>>,
}

/// The step being worked on: its `==> ` line has been printed, its `✓`
/// hasn't yet.
struct Step {
    started: Instant,
    /// A command with its own output ran (inside the new system, the first
    /// switch): that output already says how the step went.
    spoke: bool,
}

/// Commands whose output stays on the screen.
pub(crate) fn shows_live(cmd: &[String]) -> bool {
    let has = |word: &str| cmd.iter().any(|c| c == word);
    match cmd.first().map(String::as_str) {
        // The stage3's progress bar.
        Some("wget") => has("--show-progress"),
        // inside.sh and the first bunny switch / lock.
        Some("chroot") => has("/root/bunny-inside.sh") || has("/usr/local/bin/bunny"),
        _ => false,
    }
}

/// `/var/log/bunny/<time>-install.log`, the same name for the whole run.
pub fn log_path() -> PathBuf {
    static STAMP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let stamp = STAMP.get_or_init(crate::commands::timestamp);
    PathBuf::from(format!("/var/log/bunny/{stamp}-install.log"))
}

impl QuietRunner {
    /// `log`: where the tools' output goes (`None` keeps none).
    pub fn new(inner: Box<dyn Runner>, verbose: bool, log: Option<PathBuf>) -> Self {
        if let Some(dir) = log.as_deref().and_then(Path::parent) {
            let _ = std::fs::create_dir_all(dir);
        }
        QuietRunner {
            inner,
            verbose,
            log,
            step: RefCell::new(None),
        }
    }

    /// Adds a command and what it printed to the log. Never fails the
    /// install: a log that can't be written only loses the log.
    fn keep(&self, cmd: &[String], ok: bool, text: &str) {
        let Some(path) = &self.log else {
            return;
        };
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        else {
            return;
        };
        let _ = writeln!(file, "$ {}", cmd.join(" "));
        let _ = file.write_all(text.as_bytes());
        if !text.is_empty() && !text.ends_with('\n') {
            let _ = writeln!(file);
        }
        if !ok {
            let _ = writeln!(file, "(failed)");
        }
        let _ = writeln!(file);
    }

    /// After a quiet command failed: the end of what it said, in plain
    /// text, and where the rest is.
    fn show_failure(&self, cmd: &[String], text: &str) {
        print!("{}", failure_text(cmd, text, self.log.as_deref()));
        let _ = std::io::stdout().flush();
    }

    fn spoke(&self) {
        if let Some(step) = self.step.borrow_mut().as_mut() {
            step.spoke = true;
        }
    }

    /// Prints the open step's `✓` line (if it has one) and closes it.
    fn close_step(&self) {
        if let Some(step) = self.step.borrow_mut().take() {
            if !step.spoke {
                println!("{}", done_line(step.started.elapsed().as_secs()));
            }
        }
    }
}

/// `  ✓ Done`, with the time when it took a while.
fn done_line(secs: u64) -> String {
    if secs >= 2 {
        format!("  ✓ Done ({})", crate::progress::duration(secs))
    } else {
        "  ✓ Done".to_string()
    }
}

/// What's shown when a quiet command fails: its last lines, colour codes
/// removed, then where the full output is. The caller explains what it
/// means and what to do next.
fn failure_text(cmd: &[String], text: &str, log: Option<&Path>) -> String {
    let clean = crate::explain::strip_ansi(text);
    let lines: Vec<&str> = clean.lines().filter(|l| !l.trim().is_empty()).collect();
    let tool = cmd.first().map(String::as_str).unwrap_or("A command");
    let mut out = if lines.is_empty() {
        format!("  ✗ {tool} failed without saying why.\n")
    } else {
        let mut out = format!("  ✗ {tool} failed. What it said:\n");
        for line in &lines[lines.len().saturating_sub(TAIL)..] {
            out.push_str(&format!("    {}\n", line.trim_end()));
        }
        out
    };
    if let Some(log) = log {
        out.push_str(&format!(
            "  Everything it printed is in {}.\n",
            log.display()
        ));
    }
    out
}

impl Runner for QuietRunner {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        if shows_live(cmd) {
            if cmd[0] == "chroot" {
                self.spoke();
            }
            self.keep(cmd, true, "(shown on the screen)\n");
            return self.inner.run(cmd);
        }
        if self.verbose {
            let (ok, text) = self.inner.run_captured(cmd)?;
            self.keep(cmd, ok, &text);
            return Ok(ok);
        }
        let (ok, text) = self.inner.run_quiet(cmd, &mut |_| {})?;
        self.keep(cmd, ok, &text);
        if !ok {
            self.show_failure(cmd, &text);
        }
        Ok(ok)
    }

    fn run_with_input(&mut self, cmd: &[String], input: &str, secret: bool) -> Result<bool> {
        if self.verbose {
            self.keep(cmd, true, "(shown on the screen)\n");
            return self.inner.run_with_input(cmd, input, secret);
        }
        let (ok, text) = self.inner.run_with_input_quiet(cmd, input, secret)?;
        self.keep(cmd, ok, &text);
        if !ok {
            self.show_failure(cmd, &text);
        }
        Ok(ok)
    }

    fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
        self.inner.output(cmd)
    }

    fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
        let (ok, text) = self.inner.run_captured(cmd)?;
        self.keep(cmd, ok, &text);
        Ok((ok, text))
    }

    fn run_quiet(
        &mut self,
        cmd: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(bool, String)> {
        let (ok, text) = self.inner.run_quiet(cmd, on_line)?;
        self.keep(cmd, ok, &text);
        Ok((ok, text))
    }

    fn dry(&self) -> bool {
        self.inner.dry()
    }

    fn describes_steps(&self) -> bool {
        self.inner.describes_steps()
    }

    fn step_started(&self, _title: &str) {
        self.close_step();
        *self.step.borrow_mut() = Some(Step {
            started: Instant::now(),
            spoke: false,
        });
    }

    fn steps_finished(&self, ok: bool) {
        if ok {
            self.close_step();
        } else {
            // The step that failed gets no ✓: its ✗ and the explanation
            // are already on the screen.
            self.step.borrow_mut().take();
        }
    }

    fn log_file(&self) -> Option<PathBuf> {
        self.log.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    /// Pretends to run commands: each one "prints" its name, and the ones
    /// named in `fail` fail. Records how each was run.
    #[derive(Default)]
    struct Fake {
        fail: Vec<&'static str>,
        how: std::rc::Rc<RefCell<Vec<String>>>,
    }
    impl Fake {
        fn said(cmd: &[String]) -> String {
            format!("\u{1b}[31m{} said this\u{1b}[0m\n", cmd[0])
        }
        fn ok(&self, cmd: &[String]) -> bool {
            !self.fail.contains(&cmd[0].as_str())
        }
    }
    impl Runner for Fake {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.how.borrow_mut().push(format!("live {}", cmd[0]));
            Ok(self.ok(cmd))
        }
        fn run_quiet(&mut self, cmd: &[String], _: &mut dyn FnMut(&str)) -> Result<(bool, String)> {
            self.how.borrow_mut().push(format!("quiet {}", cmd[0]));
            Ok((self.ok(cmd), Self::said(cmd)))
        }
        fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
            self.how.borrow_mut().push(format!("shown {}", cmd[0]));
            Ok((self.ok(cmd), Self::said(cmd)))
        }
        fn run_with_input_quiet(
            &mut self,
            cmd: &[String],
            _: &str,
            _: bool,
        ) -> Result<(bool, String)> {
            self.how
                .borrow_mut()
                .push(format!("quiet+input {}", cmd[0]));
            Ok((self.ok(cmd), Self::said(cmd)))
        }
    }

    #[test]
    fn the_tools_output_goes_to_the_log_not_the_screen() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("var/log/bunny/x-install.log");
        let fake = Fake::default();
        let how = fake.how.clone();
        let mut r = QuietRunner::new(Box::new(fake), false, Some(log.clone()));
        assert!(r.run(&s(&["wipefs", "--all", "/dev/vda"])).unwrap());
        assert!(r
            .run_with_input(
                &s(&["cryptsetup", "luksFormat", "--key-file=-"]),
                "pw",
                true
            )
            .unwrap());
        assert_eq!(*how.borrow(), ["quiet wipefs", "quiet+input cryptsetup"]);
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("$ wipefs --all /dev/vda\n"), "{text}");
        assert!(text.contains("wipefs said this"), "{text}");
        assert!(
            text.contains("$ cryptsetup luksFormat --key-file=-\n"),
            "{text}"
        );
        // The password went to the tool's input, never to the log.
        assert!(!text.contains("pw"), "{text}");
        assert_eq!(r.log_file(), Some(log));
    }

    #[test]
    fn progress_and_the_inside_part_stay_on_the_screen() {
        assert!(shows_live(&s(&[
            "wget",
            "-q",
            "--show-progress",
            "-O",
            "x",
            "u"
        ])));
        assert!(!shows_live(&s(&["wget", "-qO-", "u"])));
        assert!(shows_live(&s(&[
            "chroot",
            "/mnt/bunny",
            "/bin/bash",
            "/root/bunny-inside.sh"
        ])));
        assert!(shows_live(&s(&[
            "chroot",
            "/mnt/bunny",
            "/usr/local/bin/bunny",
            "switch"
        ])));
        assert!(!shows_live(&s(&[
            "chroot",
            "/mnt/bunny",
            "grub-mkconfig",
            "-o",
            "x"
        ])));
        assert!(!shows_live(&s(&["mkfs.btrfs", "--force", "/dev/vda2"])));

        let fake = Fake::default();
        let how = fake.how.clone();
        let mut r = QuietRunner::new(Box::new(fake), false, None);
        r.run(&s(&[
            "chroot",
            "/mnt/bunny",
            "/bin/bash",
            "/root/bunny-inside.sh",
        ]))
        .unwrap();
        assert_eq!(*how.borrow(), ["live chroot"]);
    }

    #[test]
    fn verbose_shows_everything_and_still_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("install.log");
        let fake = Fake::default();
        let how = fake.how.clone();
        let mut r = QuietRunner::new(Box::new(fake), true, Some(log.clone()));
        r.run(&s(&["mkfs.vfat", "-F", "32"])).unwrap();
        assert_eq!(*how.borrow(), ["shown mkfs.vfat"]);
        assert!(std::fs::read_to_string(&log)
            .unwrap()
            .contains("mkfs.vfat said this"));
    }

    #[test]
    fn a_failure_shows_the_end_of_what_the_tool_said() {
        let fake = Fake {
            fail: vec!["wipefs"],
            ..Default::default()
        };
        let mut r = QuietRunner::new(Box::new(fake), false, None);
        assert!(!r.run(&s(&["wipefs", "--all", "/dev/vda"])).unwrap());

        let many: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        let text = failure_text(
            &s(&["mkfs.btrfs"]),
            &format!("\u{1b}[31m{many}\u{1b}[0m"),
            Some(Path::new("/var/log/bunny/x-install.log")),
        );
        assert!(
            text.starts_with("  ✗ mkfs.btrfs failed. What it said:\n"),
            "{text}"
        );
        assert!(
            !text.contains("line 8\n") && text.contains("    line 9\n"),
            "{text}"
        );
        assert!(
            text.contains("    line 20\n") && !text.contains('\u{1b}'),
            "{text}"
        );
        assert!(text.ends_with("Everything it printed is in /var/log/bunny/x-install.log.\n"));
        assert_eq!(
            failure_text(&s(&["sfdisk"]), "\n", None),
            "  ✗ sfdisk failed without saying why.\n"
        );
    }

    #[test]
    fn real_commands_are_kept_with_their_input_unseen() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("install.log");
        let mut r = QuietRunner::new(
            Box::new(crate::apply::SystemRunner),
            false,
            Some(log.clone()),
        );
        let ok = r
            .run_with_input(
                &s(&["sh", "-c", "read x; echo out; echo err >&2; exit 3"]),
                "secret\n",
                true,
            )
            .unwrap();
        assert!(!ok);
        assert!(r.run(&s(&["sh", "-c", "echo fine"])).unwrap());
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("out\nerr\n(failed)"), "{text}");
        assert!(
            text.contains("fine\n") && !text.contains("secret"),
            "{text}"
        );
    }

    #[test]
    fn each_step_ends_with_one_line() {
        assert_eq!(done_line(0), "  ✓ Done");
        assert_eq!(done_line(1), "  ✓ Done");
        assert!(done_line(75).starts_with("  ✓ Done ("), "{}", done_line(75));
        // A step whose own output already said how it went gets no ✓, and
        // neither does one that failed.
        let mut r = QuietRunner::new(Box::new(Fake::default()), false, None);
        r.step_started("Installing inside the new system");
        r.run(&s(&[
            "chroot",
            "/mnt/bunny",
            "/bin/bash",
            "/root/bunny-inside.sh",
        ]))
        .unwrap();
        assert!(r.step.borrow().as_ref().is_some_and(|s| s.spoke));
        r.step_started("Tidying the boot menu");
        assert!(r.step.borrow().as_ref().is_some_and(|s| !s.spoke));
        r.steps_finished(false);
        assert!(r.step.borrow().is_none());
    }
}
