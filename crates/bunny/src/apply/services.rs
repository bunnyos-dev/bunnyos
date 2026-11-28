//! Running OpenRC's service commands (`rc-update`, `rc-service`) during a
//! switch without their raw output on the screen ("service sshd added to
//! runlevel default", "WARNING: you are stopping a boot service",
//! "Caching service dependencies … [ ok ]"). bunny says what happened in its
//! own words instead, and keeps what the commands printed in
//! `/var/log/bunny/<time>-services.log`. With `--verbose` they run as before,
//! their output on the screen.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::Runner;

/// The service commands of one switch, and what they printed.
#[derive(Debug, Default)]
pub struct ServiceCommands {
    /// Show their output live instead (`--verbose`).
    pub verbose: bool,
    text: String,
    failed: bool,
}

impl ServiceCommands {
    pub fn new(verbose: bool) -> Self {
        ServiceCommands {
            verbose,
            text: String::new(),
            failed: false,
        }
    }

    /// Runs `cmd` quietly (live with `verbose`), keeping its output for the
    /// log. Returns whether it worked.
    pub fn run(&mut self, runner: &mut dyn Runner, cmd: &[String]) -> Result<bool> {
        if self.verbose {
            return runner.run(cmd);
        }
        let (ok, said) = runner.run_quiet(cmd, &mut |_| {})?;
        let said = crate::explain::strip_ansi(&said);
        self.text
            .push_str(&format!("$ {}\n{}", cmd.join(" "), said));
        if !said.is_empty() && !said.ends_with('\n') {
            self.text.push('\n');
        }
        if !ok {
            self.text.push_str("(failed)\n");
            self.failed = true;
        }
        Ok(ok)
    }

    /// Whether anything was run quietly (so there's a log to keep).
    pub fn ran(&self) -> bool {
        !self.text.is_empty()
    }

    /// Whether a command run quietly failed (its notes then point to the log).
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Saves what the commands printed as `<stamp>-services.log` in
    /// `/var/log/bunny`. Nothing when nothing ran quietly.
    pub fn save(&self, root: &Path, stamp: &str) -> Option<PathBuf> {
        if !self.ran() {
            return None;
        }
        super::install::save_log(root, stamp, "services", &self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers each command with what OpenRC prints, and records it.
    struct OpenRc {
        ran: Vec<String>,
        shown_live: Vec<String>,
        fail: &'static str,
    }

    impl Runner for OpenRc {
        fn run(&mut self, cmd: &[String]) -> Result<bool> {
            self.shown_live.push(cmd.join(" "));
            Ok(true)
        }
        fn run_quiet(
            &mut self,
            cmd: &[String],
            _on_line: &mut dyn FnMut(&str),
        ) -> Result<(bool, String)> {
            let line = cmd.join(" ");
            self.ran.push(line.clone());
            let said = match cmd.first().map(String::as_str) {
                Some("rc-update") => format!(" * service {} added to runlevel {}\n", cmd[2], cmd[3]),
                _ => " * WARNING: you are stopping a boot service\n * Caching service dependencies ... [ ok ]\n".to_string(),
            };
            Ok((line != self.fail, said))
        }
    }

    fn cmd(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn openrcs_lines_go_to_the_log_not_the_screen() {
        let root = tempfile::tempdir().unwrap();
        let mut runner = OpenRc {
            ran: Vec::new(),
            shown_live: Vec::new(),
            fail: "rc-service sshd start",
        };
        let mut services = ServiceCommands::new(false);
        assert!(services
            .run(
                &mut runner,
                &cmd(&["rc-update", "add", "consolefont", "boot"])
            )
            .unwrap());
        assert!(!services
            .run(&mut runner, &cmd(&["rc-service", "sshd", "start"]))
            .unwrap());
        assert!(runner.shown_live.is_empty(), "nothing shown live");
        let log = services.save(root.path(), "20261125-120000").unwrap();
        assert_eq!(
            log,
            root.path()
                .join("var/log/bunny/20261125-120000-services.log")
        );
        let text = std::fs::read_to_string(log).unwrap();
        assert!(text.starts_with(
            "$ rc-update add consolefont boot\n * service consolefont added to runlevel boot\n"
        ));
        assert!(
            text.contains("$ rc-service sshd start\n * WARNING: you are stopping a boot service\n")
        );
        assert!(text.ends_with("[ ok ]\n(failed)\n"), "{text}");
        assert!(services.failed());
    }

    #[test]
    fn verbose_shows_them_live_and_keeps_no_log() {
        let root = tempfile::tempdir().unwrap();
        let mut runner = OpenRc {
            ran: Vec::new(),
            shown_live: Vec::new(),
            fail: "",
        };
        let mut services = ServiceCommands::new(true);
        services
            .run(&mut runner, &cmd(&["rc-service", "consolefont", "restart"]))
            .unwrap();
        assert_eq!(runner.shown_live, ["rc-service consolefont restart"]);
        assert!(runner.ran.is_empty());
        assert_eq!(services.save(root.path(), "x"), None);
    }
}
