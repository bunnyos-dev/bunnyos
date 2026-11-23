//! Running external commands, for real or only printed (tests and `--root`).

use anyhow::{Context, Result};

/// Runs external commands.
pub trait Runner {
    /// Runs a command, showing its output to the user. Returns whether it succeeded.
    fn run(&mut self, cmd: &[String]) -> Result<bool>;

    /// Runs a command and returns what it printed, or `None` if it failed.
    /// Only for commands that just read information.
    fn output(&mut self, _cmd: &[String]) -> Result<Option<String>> {
        Ok(None)
    }

    /// Runs a command, showing its output live (including questions it asks)
    /// while keeping a copy. Returns whether it succeeded and what it printed.
    fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
        Ok((self.run(cmd)?, String::new()))
    }

    /// Runs a command without showing its output, calling `on_line` for each
    /// line (for a progress display). Returns whether it succeeded and
    /// everything it printed.
    fn run_quiet(
        &mut self,
        cmd: &[String],
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<(bool, String)> {
        Ok((self.run(cmd)?, String::new()))
    }

    /// Runs a command with `input` on its standard input (a partition
    /// table, a password), showing its output. `secret` input is never
    /// printed or logged. Returns whether it succeeded.
    fn run_with_input(&mut self, cmd: &[String], input: &str, secret: bool) -> Result<bool> {
        let _ = (input, secret);
        self.run(cmd)
    }

    /// Like [`Runner::run_with_input`], without showing the output:
    /// returns whether it succeeded and what it printed (never the input).
    fn run_with_input_quiet(
        &mut self,
        cmd: &[String],
        input: &str,
        secret: bool,
    ) -> Result<(bool, String)> {
        Ok((self.run_with_input(cmd, input, secret)?, String::new()))
    }

    /// True if commands are only printed, not run (a test root).
    fn dry(&self) -> bool {
        false
    }

    /// `bunny install` started a step (its `==> ` line comes next). A
    /// runner that hides the tools' output closes the step before with a
    /// `✓` line.
    fn step_started(&self, _title: &str) {}

    /// `bunny install` has no more steps (`ok`: the last one worked).
    fn steps_finished(&self, _ok: bool) {}

    /// Where the tools' output was kept, if anywhere.
    fn log_file(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// True for a dry run that describes each step in plain words instead
    /// of listing its commands and files (`bunny install --dry-run`
    /// without `--verbose`).
    fn describes_steps(&self) -> bool {
        false
    }
}

/// Runs commands for real.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        let status = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .status()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        Ok(status.success())
    }

    fn run_captured(&mut self, cmd: &[String]) -> Result<(bool, String)> {
        use std::io::{Read, Write};
        use std::process::Stdio;
        use std::sync::{Arc, Mutex};

        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        let copy = Arc::new(Mutex::new(Vec::new()));

        // Pass output through as it arrives, byte by byte rather than line by
        // line, so questions without a newline ("[Yes/No]") still show up.
        fn pump<R: Read + Send + 'static>(
            mut from: R,
            to_stderr: bool,
            copy: Arc<Mutex<Vec<u8>>>,
        ) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = from.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let _ = if to_stderr {
                        let mut e = std::io::stderr();
                        e.write_all(&buf[..n]).and_then(|_| e.flush())
                    } else {
                        let mut o = std::io::stdout();
                        o.write_all(&buf[..n]).and_then(|_| o.flush())
                    };
                    if let Ok(mut c) = copy.lock() {
                        c.extend_from_slice(&buf[..n]);
                    }
                }
            })
        }
        let out = pump(child.stdout.take().expect("piped"), false, copy.clone());
        let err = pump(child.stderr.take().expect("piped"), true, copy.clone());
        let status = child.wait()?;
        let _ = out.join();
        let _ = err.join();
        let text = String::from_utf8_lossy(&copy.lock().map(|c| c.clone()).unwrap_or_default())
            .into_owned();
        Ok((status.success(), text))
    }

    fn run_quiet(
        &mut self,
        cmd: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(bool, String)> {
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;

        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        // Read raw bytes per line, so output that isn't valid UTF-8 can't
        // stop the reader (which would leave the command blocked).
        fn reader<R: std::io::Read + Send + 'static>(
            from: R,
            tx: std::sync::mpsc::Sender<String>,
        ) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                let mut from = BufReader::new(from);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match from.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf);
                            let line = line.trim_end_matches(['\n', '\r']).to_string();
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
        }
        let out = reader(child.stdout.take().expect("piped"), tx.clone());
        let err = reader(child.stderr.take().expect("piped"), tx);
        let mut all = String::new();
        for line in rx {
            on_line(&line);
            all.push_str(&line);
            all.push('\n');
        }
        let status = child.wait()?;
        let _ = out.join();
        let _ = err.join();
        Ok((status.success(), all))
    }

    fn run_with_input(&mut self, cmd: &[String], input: &str, _secret: bool) -> Result<bool> {
        use std::io::Write;
        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A command that exits early closes its input; its exit status
            // says what went wrong, not the write.
            let _ = stdin.write_all(input.as_bytes());
        }
        Ok(child.wait()?.success())
    }

    fn run_with_input_quiet(
        &mut self,
        cmd: &[String],
        input: &str,
        _secret: bool,
    ) -> Result<(bool, String)> {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        if let Some(mut stdin) = child.stdin.take() {
            // As in run_with_input: the exit status says what went wrong.
            let _ = stdin.write_all(input.as_bytes());
        }
        // Both streams are read until the command ends, so neither fills up.
        let out = child.wait_with_output()?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok((out.status.success(), text))
    }

    fn output(&mut self, cmd: &[String]) -> Result<Option<String>> {
        let out = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .output()
            .with_context(|| format!("Couldn't start `{}`. Is it installed?", cmd[0]))?;
        Ok(out
            .status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned()))
    }
}

/// Prints commands instead of running them (for `--root` on a fake system).
pub struct PrintRunner;

impl Runner for PrintRunner {
    fn run(&mut self, cmd: &[String]) -> Result<bool> {
        println!("  (would run) {}", cmd.join(" "));
        Ok(true)
    }

    fn run_with_input(&mut self, cmd: &[String], input: &str, secret: bool) -> Result<bool> {
        println!("  (would run) {}", cmd.join(" "));
        if secret {
            println!("      with a password on its input");
        } else {
            for line in input.lines() {
                println!("      | {line}");
            }
        }
        Ok(true)
    }

    fn dry(&self) -> bool {
        true
    }
}

/// A dry run that runs and prints no commands: the steps say what they do
/// instead (see [`Runner::describes_steps`]).
pub struct StepsRunner;

impl Runner for StepsRunner {
    fn run(&mut self, _cmd: &[String]) -> Result<bool> {
        Ok(true)
    }

    fn run_with_input(&mut self, _cmd: &[String], _input: &str, _secret: bool) -> Result<bool> {
        Ok(true)
    }

    fn dry(&self) -> bool {
        true
    }

    fn describes_steps(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_runner_changes_nothing() {
        let mut r = StepsRunner;
        assert!(r.dry() && r.describes_steps());
        let cmd = vec!["wipefs".to_string(), "--all".into(), "/dev/null".into()];
        assert!(r.run(&cmd).unwrap());
        assert!(r.run_with_input(&cmd, "secret", true).unwrap());
        assert_eq!(r.output(&cmd).unwrap(), None);
        assert!(!PrintRunner.describes_steps());
        assert!(!SystemRunner.describes_steps());
    }
}
