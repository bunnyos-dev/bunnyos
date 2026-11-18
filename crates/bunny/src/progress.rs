//! The progress line during installs and removals:
//!
//! `⠹ [101/102] Compiling www-client/firefox 153.7.0  ·  37%  ·  24m 10s`
//!
//! On a terminal it's redrawn several times a second by its own thread, so
//! the spinner keeps turning while a long build prints nothing. It shows how
//! long the current package has taken, a percentage when the build log has
//! counters, and otherwise how long the same package took last time. It's
//! cut to the terminal's width, because a wrapped line can't be redrawn in
//! place. When the output isn't a terminal, each step is one plain line.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const TICK: Duration = Duration::from_millis(120);
/// How often the build log is read for a percentage.
const LOG_EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Step {
    text: String,
    started: Option<Instant>,
    /// The build log to read a percentage from (packages being compiled).
    build_log: Option<PathBuf>,
    /// How long this package took last time, in seconds.
    usually: Option<u64>,
}

struct Shared {
    step: Step,
    stop: bool,
}

pub struct Ticker {
    shared: Arc<Mutex<Shared>>,
    thread: Option<std::thread::JoinHandle<()>>,
    terminal: bool,
}

impl Ticker {
    pub fn new() -> Ticker {
        let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
        let shared = Arc::new(Mutex::new(Shared {
            step: Step::default(),
            stop: false,
        }));
        let thread = terminal.then(|| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || draw_loop(&shared))
        });
        Ticker {
            shared,
            thread,
            terminal,
        }
    }

    /// Starts a new step. `build_log` and `usually` add a percentage or an
    /// estimate for packages being compiled.
    pub fn step(&mut self, text: &str, build_log: Option<PathBuf>, usually: Option<u64>) {
        if !self.terminal {
            println!("  {text}");
            return;
        }
        if let Ok(mut s) = self.shared.lock() {
            s.step = Step {
                text: text.to_string(),
                started: Some(Instant::now()),
                build_log,
                usually,
            };
        }
    }

    /// Stops the spinner and clears its line.
    pub fn finish(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.stop = true;
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.finish();
    }
}

fn draw_loop(shared: &Mutex<Shared>) {
    use std::io::Write;
    let mut frame = 0usize;
    let mut percent: Option<u8> = None;
    let mut last_read: Option<Instant> = None;
    let mut log_for: Option<PathBuf> = None;
    let mut drawn = false;
    loop {
        let line = {
            let Ok(s) = shared.lock() else { break };
            if s.stop {
                break;
            }
            let Some(started) = s.step.started else {
                drop(s);
                std::thread::sleep(TICK);
                continue;
            };
            if log_for != s.step.build_log {
                log_for = s.step.build_log.clone();
                percent = None;
                last_read = None;
            }
            if let Some(log) = &s.step.build_log {
                if last_read.is_none_or(|t| t.elapsed() >= LOG_EVERY) {
                    percent = read_tail(log).and_then(|t| crate::emerge::build_percent(&t));
                    last_read = Some(Instant::now());
                }
            }
            compose(
                FRAMES[frame % FRAMES.len()],
                &s.step.text,
                started.elapsed().as_secs(),
                percent,
                s.step.usually,
            )
        };
        let width = terminal_width().unwrap_or(80);
        print!("\r\x1b[2K{}", fit(&line, width.saturating_sub(1)));
        let _ = std::io::stdout().flush();
        drawn = true;
        frame += 1;
        std::thread::sleep(TICK);
    }
    if drawn {
        print!("\r\x1b[2K");
        let _ = std::io::stdout().flush();
    }
}

/// The line itself: spinner, step, then what's known about how far along it is.
pub fn compose(
    spinner: char,
    text: &str,
    elapsed: u64,
    percent: Option<u8>,
    usually: Option<u64>,
) -> String {
    let mut line = format!("  {spinner} {text}");
    if let Some(p) = percent {
        line.push_str(&format!("  ·  {p}%"));
    }
    // Times only once a step is slow enough to wonder about.
    if elapsed >= 3 {
        line.push_str(&format!("  ·  {}", duration(elapsed)));
        if percent.is_none() {
            if let Some(usual) = usually.filter(|u| *u >= 60) {
                line.push_str(&format!(" (last time: {})", duration(usual)));
            }
        }
    }
    line
}

/// `24m 10s`, `1h 05m`, `42s`.
pub fn duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// Cuts a line to `width` characters, so it never wraps.
pub fn fit(line: &str, width: usize) -> String {
    if line.chars().count() <= width {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The last few KB of a file (build logs can be hundreds of MB).
fn read_tail(path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)))
        .ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The terminal's width in columns.
fn terminal_width() -> Option<usize> {
    // SAFETY: TIOCGWINSZ only writes into the winsize struct we pass.
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        (libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0)
            .then_some(size.ws_col as usize)
    }
}

/// Where Portage keeps a package's build log while it compiles.
pub fn build_log(root: &std::path::Path, name: &str, version: &str) -> PathBuf {
    let (category, package) = name.split_once('/').unwrap_or(("", name));
    root.join("var/tmp/portage")
        .join(category)
        .join(format!("{package}-{version}"))
        .join("temp/build.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_the_line() {
        let text = "[101/102] Compiling www-client/firefox 153.7.0";
        assert_eq!(
            compose('⠹', text, 1, None, None),
            format!("  ⠹ {text}"),
            "nothing extra in the first seconds"
        );
        assert_eq!(
            compose('⠹', text, 1450, Some(37), Some(4800)),
            format!("  ⠹ {text}  ·  37%  ·  24m 10s"),
            "a real percentage beats last time's estimate"
        );
        assert_eq!(
            compose('⠹', text, 1450, None, Some(4800)),
            format!("  ⠹ {text}  ·  24m 10s (last time: 1h 20m)")
        );
        assert_eq!(
            compose('⠹', text, 30, None, Some(20)),
            format!("  ⠹ {text}  ·  30s"),
            "quick packages don't need last time"
        );
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(1450), "24m 10s");
        assert_eq!(duration(3900), "1h 05m");
    }

    #[test]
    fn never_wider_than_the_terminal() {
        assert_eq!(fit("short", 80), "short");
        let cut = fit(
            "⠹ [101/102] Compiling www-client/firefox 153.7.0  ·  24m 10s",
            30,
        );
        assert_eq!(cut.chars().count(), 30);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn knows_where_the_build_log_is() {
        assert_eq!(
            build_log(std::path::Path::new("/"), "www-client/firefox", "153.7.0"),
            PathBuf::from("/var/tmp/portage/www-client/firefox-153.7.0/temp/build.log")
        );
    }

    #[test]
    fn reads_the_end_of_a_long_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("build.log");
        let mut text = "[1/100] start\n".repeat(5000);
        text.push_str("[60/100] nearly\n");
        std::fs::write(&log, text).unwrap();
        let tail = read_tail(&log).unwrap();
        assert!(tail.len() <= 16 * 1024);
        assert_eq!(crate::emerge::build_percent(&tail), Some(60));
    }
}
