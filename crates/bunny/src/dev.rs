//! Developer mode: `[dev]` in bunny.toml.
//!
//! With `report` on, every bunny command is recorded exactly as it appeared
//! in the terminal (prompts, progress, answers), using util-linux's `script`
//! so nothing about how bunny behaves changes. `bunny report` tidies the
//! recordings, swaps personal details for placeholders, and files them as an
//! issue in a private GitHub repository with the user's own `gh` login.
//! With `report = "auto"`, a command that ends with a problem is reported
//! by itself.

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config_repo::Person;

/// Where reports go unless `[dev] reports` says otherwise.
pub const DEFAULT_REPORTS: &str = "bunnyos-dev/bunny-reports";

/// Set while a command is being recorded, so it isn't recorded twice.
pub const SESSION_ENV: &str = "BUNNY_SESSION";

/// On a preview live USB (one built for testing), this file turns recording
/// on for every bunny command, since the USB has no `bunny.toml`. Release
/// images don't have it.
pub const PREVIEW: &str = "/etc/bunny-preview.toml";

/// The `[dev]` section.
#[derive(Debug, Default, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Dev {
    /// Record commands: false (default), true, or "auto" (also report
    /// problems by themselves).
    #[serde(default)]
    pub report: ReportMode,
    /// The GitHub repository reports are filed in, as "owner/name".
    #[serde(default)]
    pub reports: Option<String>,
    /// What to call this computer in reports ("laptop", "desktop").
    #[serde(default)]
    pub name: Option<String>,
}

impl Dev {
    pub fn repo(&self) -> &str {
        self.reports.as_deref().unwrap_or(DEFAULT_REPORTS)
    }

    pub fn machine(&self) -> &str {
        self.name.as_deref().unwrap_or("this computer")
    }
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum ReportMode {
    #[default]
    Off,
    /// Record; send with `bunny report`.
    Record,
    /// Record, and send problems by themselves.
    Auto,
}

impl<'de> Deserialize<'de> for ReportMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Word(String),
        }
        match Raw::deserialize(d)? {
            Raw::Flag(false) => Ok(ReportMode::Off),
            Raw::Flag(true) => Ok(ReportMode::Record),
            Raw::Word(w) if w == "auto" => Ok(ReportMode::Auto),
            Raw::Word(w) => Err(serde::de::Error::custom(format!(
                "report = \"{w}\" isn't a choice; use true, false or \"auto\""
            ))),
        }
    }
}

/// Reads `[dev]` straight from the config file, before anything else runs.
/// Anything unreadable means developer mode is off.
pub fn settings(config: &Path) -> Dev {
    #[derive(Deserialize)]
    struct Only {
        #[serde(default)]
        dev: Dev,
    }
    std::fs::read_to_string(config)
        .ok()
        .and_then(|text| {
            let table: toml::Table = toml::from_str(&text).ok()?;
            let dev = table.get("dev")?.clone();
            let mut only = toml::Table::new();
            only.insert("dev".into(), dev);
            toml::Value::Table(only).try_into::<Only>().ok()
        })
        .map(|o| o.dev)
        .unwrap_or_default()
}

/// `[dev]` from the config, or from `preview` (a preview live USB's file)
/// when the config doesn't turn recording on.
pub fn settings_or_preview(config: &Path, preview: &Path) -> Dev {
    let dev = settings(config);
    if dev.report != ReportMode::Off {
        return dev;
    }
    let preview = settings(preview);
    if preview.report != ReportMode::Off {
        preview
    } else {
        dev
    }
}

/// The recording of the command running now, if it's being recorded.
pub fn current_recording() -> Option<PathBuf> {
    std::env::var_os(SESSION_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
}

/// Copies the recording of a command still running (as far as it got) into
/// `to` (another system's sessions folder), with its notes, so `bunny
/// report` there sends it. `bunny install` uses it just before it
/// unmounts the new system, so the last few lines aren't in the copy.
/// Returns the copied log.
pub fn copy_recording(log: &Path, to: &Path, args: &[String]) -> Result<PathBuf> {
    let name = log
        .file_name()
        .context("The recording has no file name")?
        .to_owned();
    std::fs::create_dir_all(to).with_context(|| format!("Couldn't create {}", to.display()))?;
    let copied = to.join(name);
    std::fs::copy(log, &copied).with_context(|| format!("Couldn't copy {}", log.display()))?;
    let args: Vec<String> = args
        .iter()
        .filter(|a| *a != "--in-session")
        .cloned()
        .collect();
    let stem = log
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let started = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .and_then(|sub| stem.strip_suffix(&format!("-{sub}")))
        .unwrap_or(&stem)
        .to_string();
    // From the time in the recording's name: the file's creation time
    // isn't available on the live USB's filesystem, which made it "0 s".
    let seconds = seconds_since(&started, unix_now())
        .or_else(|| {
            std::fs::metadata(log)
                .and_then(|m| m.created())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs())
        })
        .unwrap_or(0);
    let meta = Meta {
        args,
        version: env!("CARGO_PKG_VERSION").into(),
        started,
        seconds,
        // It got this far without failing.
        exit_code: Some(0),
        reported: None,
    };
    write_meta(&copied.with_extension("toml"), &meta)?;
    Ok(copied)
}

/// Seconds since 1970, now.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// How long ago a `YYYYMMDD-HHMMSS` (UTC) stamp was, at `now`. `None` for
/// a stamp that can't be read or is in the future (the clock moved back).
fn seconds_since(stamp: &str, now: i64) -> Option<u64> {
    let then = crate::status::parse_stamp(stamp)?;
    u64::try_from(now - then).ok()
}

/// Where recordings are kept: the user's own state folder, so no root needed.
pub fn sessions_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/var/tmp"));
    base.join("bunny/sessions")
}

/// What's known about one recorded command, next to its recording.
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
pub struct Meta {
    pub args: Vec<String>,
    pub version: String,
    pub started: String,
    #[serde(default)]
    pub seconds: u64,
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// The issue it was sent in, once sent.
    #[serde(default)]
    pub reported: Option<String>,
}

/// Commands that are never recorded: reporting itself, and bunny's internal
/// helper.
fn recordable(subcommand: Option<&str>) -> bool {
    !matches!(subcommand, None | Some("report" | "helper" | "help"))
}

/// Records this command if developer mode is on: runs bunny again inside
/// `script` and returns its exit code. Returns None when nothing is recorded
/// (mode off, already recording, not a terminal, or `script` missing); the
/// caller then just runs the command.
pub fn record_if_on(config: &Path, in_session: bool) -> Option<ExitCode> {
    if in_session || std::env::var_os(SESSION_ENV).is_some() {
        return None;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let subcommand = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str);
    if !recordable(subcommand)
        || !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
    {
        return None;
    }
    let dev = settings_or_preview(config, Path::new(PREVIEW));
    if dev.report == ReportMode::Off {
        return None;
    }
    match record(&args, subcommand.unwrap_or("bunny"), &dev) {
        Ok(code) => Some(code),
        Err(err) => {
            // Recording is a convenience: never let it stop the command.
            eprintln!("(developer mode: couldn't record this command: {err:#})");
            None
        }
    }
}

fn record(args: &[String], subcommand: &str, dev: &Dev) -> Result<ExitCode> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    let stamp = crate::commands::timestamp();
    let base = dir.join(format!("{stamp}-{subcommand}"));
    let log = base.with_extension("log");
    let exe = std::env::current_exe()?;
    let line = std::iter::once(exe.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|a| shell_quote(&a))
        .collect::<Vec<_>>()
        .join(" ");
    let started = std::time::Instant::now();
    // The recorded bunny handles Ctrl+C; this one just waits for it.
    let _careful = crate::interrupt::Careful::start();
    let status = Command::new("script")
        .args(["--quiet", "--return", "--flush", "--command", &line])
        .arg(&log)
        .env("SHELL", "/bin/sh")
        .env(SESSION_ENV, &log)
        .status()
        .context("Couldn't start `script` (from util-linux)")?;
    let meta = Meta {
        args: args.to_vec(),
        version: env!("CARGO_PKG_VERSION").into(),
        started: stamp,
        seconds: started.elapsed().as_secs(),
        exit_code: status.code(),
        reported: None,
    };
    write_meta(&base.with_extension("toml"), &meta)?;

    if dev.report == ReportMode::Auto {
        let text = clean(&std::fs::read_to_string(&log).unwrap_or_default());
        if had_problem(&meta, &text) {
            match send_sessions(dev, std::slice::from_ref(&base), true) {
                Ok(url) => println!("  (developer mode) Sent a report about this: {url}"),
                Err(err) => println!(
                    "  (developer mode) Couldn't send a report ({err:#}).\n  \
                     `bunny report` sends it later."
                ),
            }
        }
    }
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(n) => ExitCode::from(u8::try_from(n).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
}

fn write_meta(path: &Path, meta: &Meta) -> Result<()> {
    std::fs::write(path, toml::to_string(meta)?)
        .with_context(|| format!("Couldn't write {}", path.display()))
}

/// Single-quotes an argument for /bin/sh.
pub fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@+,".contains(c))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// A command "had a problem" if it failed or printed a ⚠ or ✗, unless it
/// stopped because of what you typed (a name that doesn't exist, a config
/// mistake, no password at sudo's question): that's yours, not bunny's, even
/// though it prints a ✗ to tell you.
pub fn had_problem(meta: &Meta, text: &str) -> bool {
    if meta.exit_code == Some(i32::from(crate::commands::YOUR_INPUT)) {
        return false;
    }
    meta.exit_code != Some(0) || text.contains('✗') || text.contains('⚠')
}

/// Turns a raw terminal recording into readable text: no colour codes, a
/// progress line that was redrawn shows only its last state, and script's
/// own header and footer are dropped.
pub fn clean(raw: &str) -> String {
    let no_ansi = strip_ansi(raw);
    let mut out = String::new();
    for line in no_ansi.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with("Script started on ") || line.starts_with("Script done on ") {
            continue;
        }
        // A carriage return redraws the line: keep what was drawn last.
        let shown = line
            .rsplit('\r')
            .find(|part| !part.trim().is_empty())
            .unwrap_or("");
        // Backspaces (corrections while typing) remove the character before.
        let mut fixed = String::new();
        for c in shown.chars() {
            if c == '\u{8}' {
                fixed.pop();
            } else if c != '\u{7}' {
                fixed.push(c);
            }
        }
        out.push_str(fixed.trim_end());
        out.push('\n');
    }
    out.trim_matches('\n').to_string()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: ESC [ ... final byte (@ to ~)
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL or ESC \
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Personal details to swap out before anything leaves the computer.
#[derive(Debug, Default)]
pub struct Redactions {
    /// (what to find, what to put instead), longest first.
    pub swaps: Vec<(String, String)>,
}

impl Redactions {
    /// User names and their home folders, and the computer's name.
    pub fn for_this_computer(root: &Path) -> Redactions {
        let mut swaps = Vec::new();
        if let Ok(passwd) = std::fs::read_to_string(root.join("etc/passwd")) {
            for line in passwd.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() < 6 {
                    continue;
                }
                let uid: u32 = fields[2].parse().unwrap_or(0);
                if (1000..60000).contains(&uid) {
                    if fields[5].len() > 1 {
                        swaps.push((fields[5].to_string(), "/home/you".to_string()));
                    }
                    swaps.push((fields[0].to_string(), "you".to_string()));
                }
            }
        }
        for var in ["USER", "SUDO_USER", "DOAS_USER", "LOGNAME"] {
            if let Ok(name) = std::env::var(var) {
                if !name.is_empty() && name != "root" {
                    swaps.push((name, "you".to_string()));
                }
            }
        }
        for file in ["proc/sys/kernel/hostname", "etc/hostname"] {
            if let Ok(host) = std::fs::read_to_string(root.join(file)) {
                let host = host.trim();
                if !host.is_empty() && host != "localhost" {
                    swaps.push((host.to_string(), "computer".to_string()));
                    swaps.push((host.to_lowercase(), "computer".to_string()));
                }
            }
        }
        Redactions::new(swaps)
    }

    pub fn new(mut swaps: Vec<(String, String)>) -> Redactions {
        swaps.retain(|(from, to)| from.len() > 1 && from != to);
        swaps.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        swaps.dedup_by(|a, b| a.0 == b.0);
        Redactions { swaps }
    }

    pub fn apply(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (from, to) in &self.swaps {
            text = if from.starts_with('/') {
                text.replace(from.as_str(), to)
            } else if SHARED_WORDS.contains(&from.to_lowercase().as_str()) {
                replace_as_name(&text, from, to)
            } else {
                replace_word(&text, from, to)
            };
        }
        mask_tokens(&text)
    }
}

/// Words bunny (and Gentoo) print themselves. A user or computer called
/// one of these is only swapped where it's clearly a name, so a user named
/// `bunny` doesn't turn `bunny update` into `you update`.
const SHARED_WORDS: [&str; 8] = [
    "bunny", "gentoo", "portage", "emerge", "linux", "root", "admin", "user",
];

/// Replaces `name` only where it's used as a user or computer name:
/// `name@host`, `user@name`, `"name"` (like `admins = ["name"]`), and after
/// `login: ` or `user `.
fn replace_as_name(text: &str, name: &str, with: &str) -> String {
    let mut out = text.to_string();
    for (find, put) in [
        (format!("{name}@"), format!("{with}@")),
        (format!("@{name}"), format!("@{with}")),
        (format!("\"{name}\""), format!("\"{with}\"")),
        (format!("login: {name}"), format!("login: {with}")),
        (format!("user {name}"), format!("user {with}")),
    ] {
        out = out.replace(&find, &put);
    }
    out
}

/// Replaces `word` where it isn't part of a longer name (letters, digits,
/// `_` or `-` on either side).
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let part_of_name =
        |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let before = if at > 0 {
            rest[..at].chars().last()
        } else {
            out.chars().last()
        };
        let after = rest[at + word.len()..].chars().next();
        out.push_str(&rest[..at]);
        if part_of_name(before) || part_of_name(after) {
            out.push_str(word);
        } else {
            out.push_str(with);
        }
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}

/// Masks things that identify people or networks wherever they appear:
/// email addresses and Tailscale machine names.
fn mask_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        let bare = token.trim_end_matches(['.', ',', ':', ';', ')']);
        let tail = &token[bare.len()..];
        if bare.contains(".ts.net") {
            out.push_str("<tailnet-name>");
            out.push_str(tail);
        } else if let Some((user, domain)) = bare.split_once('@') {
            if !user.is_empty() && domain.contains('.') && !domain.starts_with('.') {
                out.push_str("<email>");
                out.push_str(tail);
            } else {
                out.push_str(token);
            }
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() || "\"'<>()[]{}".contains(c) {
            flush(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// Recordings not sent yet, oldest first (each as its path without extension).
pub fn unsent(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .filter(|p| read_meta(p).is_some_and(|m| m.reported.is_none()))
                .map(|p| p.with_extension(""))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn read_meta(path: &Path) -> Option<Meta> {
    toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Longest recording kept whole; longer ones keep their start and end.
const MAX_SESSION: usize = 14_000;
/// GitHub's limit for an issue body is 65,536 characters.
const MAX_BODY: usize = 60_000;

/// Keeps the start and the end of a long text.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max / 5).collect();
    let tail: Vec<char> = text.chars().rev().take(max - max / 5).collect();
    let tail: String = tail.into_iter().rev().collect();
    format!("{head}\n\n[… the middle is left out to keep the report short …]\n\n{tail}")
}

/// One recorded command, ready to put in a report.
pub struct Session {
    pub meta: Meta,
    pub text: String,
}

/// Facts about the computer that help reproduce a problem.
pub struct About {
    pub machine: String,
    pub bunny: String,
    pub kernel: String,
    pub profile: String,
    pub portage: String,
}

impl About {
    pub fn gather(root: &Path, machine: &str) -> About {
        let read = |p: &str| {
            std::fs::read_to_string(root.join(p))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "unknown".into())
        };
        let profile = std::fs::read_link(root.join("etc/portage/make.profile"))
            .map(|p| {
                let p = p.to_string_lossy().into_owned();
                p.split_once("profiles/")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or(p)
            })
            .unwrap_or_else(|_| "unknown".into());
        let portage = std::fs::read_dir(root.join("var/db/pkg/sys-apps"))
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find_map(|n| n.strip_prefix("portage-").map(str::to_string))
            })
            .unwrap_or_else(|| "unknown".into());
        About {
            machine: machine.to_string(),
            bunny: env!("CARGO_PKG_VERSION").into(),
            kernel: read("proc/sys/kernel/osrelease"),
            profile,
            portage,
        }
    }
}

/// The issue title and body for a set of recordings.
pub fn compose(
    sessions: &[Session],
    status: Option<&str>,
    about: &About,
    automatic: bool,
    redact: &Redactions,
) -> (String, String) {
    let problems = sessions
        .iter()
        .filter(|s| had_problem(&s.meta, &s.text))
        .count();
    let title = match sessions {
        [one] => format!(
            "{}: `bunny {}`{}",
            about.machine,
            redact.apply(&one.meta.args.join(" ")),
            if problems > 0 { " had a problem" } else { "" }
        ),
        _ => format!(
            "{}: {} commands, {} with a problem",
            about.machine,
            sessions.len(),
            problems
        ),
    };

    let mut body = String::new();
    let _ = writeln!(
        body,
        "**{}** · bunny {} · kernel {} · profile `{}` · Portage {}\n",
        about.machine, about.bunny, about.kernel, about.profile, about.portage
    );
    let _ = writeln!(
        body,
        "Sent by `bunny report`{}. Personal details are replaced with `you`, \
         `/home/you`, `computer`, `<email>` and `<tailnet-name>`.\n",
        if automatic {
            " automatically, because a command had a problem"
        } else {
            ""
        }
    );
    let mut budget = MAX_BODY.saturating_sub(body.len() + 6_000);
    for (i, s) in sessions.iter().enumerate() {
        let problem = had_problem(&s.meta, &s.text);
        let exit = match s.meta.exit_code {
            Some(0) => "worked".to_string(),
            Some(n) => format!("exit {n}"),
            None => "stopped".to_string(),
        };
        let text = shorten(&redact.apply(&s.text), MAX_SESSION.min(budget.max(2_000)));
        budget = budget.saturating_sub(text.len());
        let _ = writeln!(
            body,
            "<details{}><summary><b>{}. <code>bunny {}</code></b> · {}{} · {} · {} s</summary>\n\n````text\n{}\n````\n</details>\n",
            if problem { " open" } else { "" },
            i + 1,
            html_escape(&redact.apply(&s.meta.args.join(" "))),
            exit,
            if problem { " · ⚠ problem" } else { "" },
            readable_time(&s.meta.started),
            s.meta.seconds,
            text.replace("````", "```")
        );
    }
    if let Some(status) = status {
        let _ = writeln!(
            body,
            "<details><summary><b><code>bunny status</code> when this was sent</b></summary>\n\n````text\n{}\n````\n</details>",
            shorten(redact.apply(status).trim_end(), 4_000)
        );
    }
    (title, body)
}

/// `20261118-031707` as `2026-11-18 03:17 UTC`.
fn readable_time(stamp: &str) -> String {
    let d: Vec<char> = stamp.chars().collect();
    if d.len() != 15 || d[8] != '-' {
        return stamp.to_string();
    }
    let s = |a: usize, b: usize| d[a..b].iter().collect::<String>();
    format!(
        "{}-{}-{} {}:{} UTC",
        s(0, 4),
        s(4, 6),
        s(6, 8),
        s(9, 11),
        s(11, 13)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Sends recordings as one issue. Returns its address, and marks them sent.
pub fn send_sessions(dev: &Dev, bases: &[PathBuf], automatic: bool) -> Result<String> {
    let (title, body) = prepare(dev, bases, automatic)?;
    let url = file_issue(dev.repo(), &title, &body)?;
    for base in bases {
        let path = base.with_extension("toml");
        if let Some(mut meta) = read_meta(&path) {
            meta.reported = Some(url.clone());
            write_meta(&path, &meta)?;
        }
    }
    Ok(url)
}

/// Builds the report from recordings on disk.
pub fn prepare(dev: &Dev, bases: &[PathBuf], automatic: bool) -> Result<(String, String)> {
    let sessions: Vec<Session> = bases
        .iter()
        .filter_map(|base| {
            Some(Session {
                meta: read_meta(&base.with_extension("toml"))?,
                text: clean(&std::fs::read_to_string(base.with_extension("log")).ok()?),
            })
        })
        .collect();
    if sessions.is_empty() {
        bail!("there are no recordings to send");
    }
    let root = Path::new("/");
    if !automatic {
        // It can take a while (glsa-check reads every advisory), and
        // without a word it looked like `bunny report` had hung.
        println!("Adding `bunny status` to the report (this can take a moment the first time)…");
    }
    let mut status_cmd = Command::new(std::env::current_exe()?);
    status_cmd
        .arg("status")
        .env(SESSION_ENV, "status-for-report");
    let status = match output_within(status_cmd, STATUS_TIME_LIMIT) {
        Finished::Output(text) => Some(text),
        Finished::TooSlow => Some(format!(
            "(bunny status didn't finish within {} seconds, so it's left out.)",
            STATUS_TIME_LIMIT.as_secs()
        )),
        Finished::Failed => None,
    };
    Ok(compose(
        &sessions,
        status.as_deref(),
        &About::gather(root, dev.machine()),
        automatic,
        &Redactions::for_this_computer(root),
    ))
}

/// Who `gh` runs as: under `sudo bunny` or `doas bunny`, the person who
/// ran it (root has no GitHub sign-in), the way `bunny publish` does.
/// `None` means run `gh` as we are.
fn gh_person() -> Option<Person> {
    if crate::commands::is_root() {
        Person::invoking(Path::new("/"))
    } else {
        None
    }
}

/// The command that files an issue in `repo` with `gh` (through the REST
/// API, which works with every kind of GitHub token), run as `person` when
/// there is one. The body comes on standard input.
fn issue_command(repo: &str, title: &str, person: Option<&Person>) -> Vec<String> {
    let path = format!("repos/{repo}/issues");
    let title = format!("title={title}");
    let gh = [
        "gh",
        "api",
        &path,
        "--method",
        "POST",
        "-f",
        &title,
        "-F",
        "body=@-",
        "--jq",
        ".html_url",
    ];
    match person {
        Some(person) => person.run_as(&gh),
        None => gh.iter().map(|a| a.to_string()).collect(),
    }
}

/// How long `bunny report` waits for `bunny status`.
const STATUS_TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// How a command run with a time limit ended.
#[derive(Debug, PartialEq)]
enum Finished {
    /// What it printed.
    Output(String),
    /// Still running at the limit, so it was stopped.
    TooSlow,
    /// It couldn't be started.
    Failed,
}

/// Runs `cmd`, keeping what it prints, and stops it after `limit`.
fn output_within(mut cmd: Command, limit: std::time::Duration) -> Finished {
    use std::io::Read;
    let Ok(mut child) = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return Finished::Failed;
    };
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_string(&mut text);
        }
        text
    });
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Finished::TooSlow;
            }
        }
    }
    Finished::Output(reader.join().unwrap_or_default())
}

/// Files an issue with the GitHub CLI and the user's own login (through the
/// REST API, which works with every kind of GitHub token).
fn file_issue(repo: &str, title: &str, body: &str) -> Result<String> {
    let person = gh_person();
    let cmd = issue_command(repo, title, person.as_ref());
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| {
            anyhow::anyhow!(
                "the GitHub CLI (gh) isn't installed. Install it with `bunny add dev-util/github-cli`, \
                 then sign in with `gh auth login`"
            )
        })?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().context("Couldn't talk to gh")?;
        stdin.write_all(body.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        bail!("{}", explain_gh_failure(repo, &said, person.as_ref()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// gh's error, as something to do about it.
fn explain_gh_failure(repo: &str, said: &str, person: Option<&Person>) -> String {
    if said.contains("auth login") || said.contains("HTTP 401") || said.contains("not logged") {
        match person {
            Some(p) => format!(
                "{} isn't signed in to GitHub. Run `gh auth login` once as yourself \
                 (not with sudo or doas), then try again",
                p.name
            ),
            None => "gh isn't signed in to GitHub. Run `gh auth login` once, then try again".into(),
        }
    } else if said.contains("HTTP 404") || said.contains("Not Found") {
        format!("couldn't find {repo}, or this GitHub account can't open it")
    } else if said.contains("HTTP 410") || said.contains("disabled") {
        format!("{repo} has issues turned off")
    } else {
        format!("gh couldn't file the report: {}", said.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_mode_reads_true_false_and_auto() {
        let parse = |s: &str| -> Result<Dev, toml::de::Error> { toml::from_str(s) };
        assert_eq!(parse("").unwrap().report, ReportMode::Off);
        assert_eq!(parse("report = true").unwrap().report, ReportMode::Record);
        assert_eq!(parse("report = false").unwrap().report, ReportMode::Off);
        assert_eq!(parse("report = \"auto\"").unwrap().report, ReportMode::Auto);
        let err = parse("report = \"always\"").unwrap_err().to_string();
        assert!(err.contains("use true, false or \"auto\""), "{err}");
    }

    #[test]
    fn settings_come_from_the_config_file_and_default_to_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bunny.toml");
        assert_eq!(settings(&path).report, ReportMode::Off);
        std::fs::write(
            &path,
            "packages = [\"app-misc/tmux\"]\n[dev]\nreport = \"auto\"\nname = \"laptop\"\n",
        )
        .unwrap();
        let dev = settings(&path);
        assert_eq!(dev.report, ReportMode::Auto);
        assert_eq!(dev.machine(), "laptop");
        assert_eq!(dev.repo(), DEFAULT_REPORTS);
        std::fs::write(&path, "this isn't toml [").unwrap();
        assert_eq!(settings(&path).report, ReportMode::Off);
    }

    #[test]
    fn a_preview_usb_records_without_a_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bunny.toml");
        let preview = dir.path().join("bunny-preview.toml");
        // No config, no preview file: off.
        assert_eq!(
            settings_or_preview(&config, &preview).report,
            ReportMode::Off
        );
        std::fs::write(&preview, "[dev]\nreport = true\n").unwrap();
        assert_eq!(
            settings_or_preview(&config, &preview).report,
            ReportMode::Record
        );
        // The config's own [dev] wins when it turns recording on.
        std::fs::write(&config, "[dev]\nreport = \"auto\"\n").unwrap();
        assert_eq!(
            settings_or_preview(&config, &preview).report,
            ReportMode::Auto
        );
    }

    #[test]
    fn copies_a_recording_into_another_system() {
        let usb = tempfile::tempdir().unwrap();
        let log = usb.path().join("20261119-201500-install.log");
        std::fs::write(&log, "==> Starting\n").unwrap();
        let to = tempfile::tempdir().unwrap();
        let sessions = to.path().join("home/you/.local/state/bunny/sessions");
        let args = vec!["install".to_string(), "--ssh".to_string()];
        let copied = copy_recording(&log, &sessions, &args).unwrap();
        assert_eq!(copied, sessions.join("20261119-201500-install.log"));
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "==> Starting\n");
        // `bunny report` in the new system finds it as not sent yet.
        assert_eq!(
            unsent(&sessions),
            [sessions.join("20261119-201500-install")]
        );
        let meta = read_meta(&copied.with_extension("toml")).unwrap();
        assert_eq!(meta.args, args);
        assert_eq!(meta.started, "20261119-201500");
        assert_eq!(meta.exit_code, Some(0));
        assert_eq!(meta.reported, None);
        // How long it ran comes from the time in its name, not the file's
        // creation time (unavailable on the live USB, so it said "0 s").
        let since = seconds_since("20261119-201500", unix_now()).unwrap();
        assert!(
            meta.seconds >= since && meta.seconds <= since + 60,
            "{}",
            meta.seconds
        );
    }

    #[test]
    fn says_how_long_ago_a_stamp_was() {
        let then = crate::status::parse_stamp("20261122-100000").unwrap();
        assert_eq!(seconds_since("20261122-100000", then + 754), Some(754));
        assert_eq!(seconds_since("20261122-100000", then - 5), None);
        assert_eq!(seconds_since("garbage", then), None);
    }

    #[test]
    fn a_slow_command_is_stopped_at_the_limit() {
        let mut quick = Command::new("sh");
        quick.args(["-c", "echo ready"]);
        assert_eq!(
            output_within(quick, std::time::Duration::from_secs(10)),
            Finished::Output("ready\n".into())
        );
        let mut slow = Command::new("sleep");
        slow.arg("30");
        let started = std::time::Instant::now();
        assert_eq!(
            output_within(slow, std::time::Duration::from_millis(200)),
            Finished::TooSlow
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(
            output_within(
                Command::new("/nonexistent/command"),
                std::time::Duration::from_secs(1)
            ),
            Finished::Failed
        );
    }

    #[test]
    fn some_commands_are_never_recorded() {
        assert!(recordable(Some("add")));
        assert!(!recordable(Some("report")));
        assert!(!recordable(Some("helper")));
        assert!(!recordable(None));
    }

    #[test]
    fn quotes_arguments_for_the_shell() {
        assert_eq!(shell_quote("app-misc/tmux"), "app-misc/tmux");
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn cleans_a_terminal_recording() {
        let raw = "Script started on 2026-11-18 10:00:00+00:00 [COMMAND=\"bunny add x\"]\r\n\
                   \u{1b}[1m✓\u{1b}[0m looks good.\r\n\
                   Apply these changes? [Y/n] \r\n\
                   \r  [1/2] Compiling a\r  [2/2] Compiling b\r\u{1b}[K  ✓ Installed 2 packages.\r\n\
                   typo\u{8}\u{8}pe\r\n\
                   \u{1b}]0;title\u{7}done\r\n\
                   Script done on 2026-11-18 10:00:05+00:00 [COMMAND_EXIT_CODE=\"0\"]\r\n";
        assert_eq!(
            clean(raw),
            "✓ looks good.\nApply these changes? [Y/n]\n  ✓ Installed 2 packages.\ntype\ndone"
        );
    }

    #[test]
    fn your_own_mistakes_are_not_reported_by_themselves() {
        let yours = Meta {
            exit_code: Some(i32::from(crate::commands::YOUR_INPUT)),
            ..Meta::default()
        };
        // Ctrl+C at sudo's password question.
        assert!(!had_problem(
            &yours,
            "[sudo] password for you: \n✗ bunny needs admin rights to change the system, and sudo didn't give them,\n  so nothing was changed."
        ));
        // A package name that doesn't exist.
        assert!(!had_problem(
            &yours,
            "✗ \"mpvv\" isn't a package. Did you mean media-video/mpv?\nNothing was changed."
        ));
    }

    #[test]
    fn a_problem_is_a_failure_or_a_warning() {
        let ok = Meta {
            exit_code: Some(0),
            ..Meta::default()
        };
        let failed = Meta {
            exit_code: Some(1),
            ..Meta::default()
        };
        assert!(!had_problem(&ok, "✓ all good"));
        assert!(had_problem(&ok, "⚠ Versions  1 difference"));
        assert!(had_problem(&failed, "✓ all good"));
    }

    #[test]
    fn swaps_personal_details() {
        let r = Redactions::new(vec![
            ("/home/sam".into(), "/home/you".into()),
            ("sam".into(), "you".into()),
            ("Thinkbox".into(), "computer".into()),
        ]);
        let text = "sam@Thinkbox ~ $ ls /home/sam/docs\n\
                    samba and sams stay; user sam, mount //nas.cat-dog.ts.net/files,\n\
                    mail sam.smith@example.org.";
        assert_eq!(
            r.apply(text),
            "you@computer ~ $ ls /home/you/docs\n\
             samba and sams stay; user you, mount <tailnet-name>,\n\
             mail <email>."
        );
    }

    #[test]
    fn a_user_called_bunny_keeps_bunnys_own_words() {
        // From the dry run: the user name was `bunny`.
        let r = Redactions::new(vec![
            ("/home/bunny".into(), "/home/you".into()),
            ("bunny".into(), "you".into()),
            ("vm-bunny".into(), "computer".into()),
        ]);
        let text = "bunny@vm-bunny ~ $ bunny status\n\
                    ✓ /etc/bunny/bunny.toml looks good. Use `bunny update`.\n\
                    admins = [\"bunny\"]\nvm-bunny login: bunny\nls /home/bunny/x";
        assert_eq!(
            r.apply(text),
            "you@computer ~ $ bunny status\n\
             ✓ /etc/bunny/bunny.toml looks good. Use `bunny update`.\n\
             admins = [\"you\"]\ncomputer login: you\nls /home/you/x"
        );
    }

    #[test]
    fn reads_passwd_and_hostname_from_a_root() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("etc")).unwrap();
        std::fs::write(
            r.join("etc/passwd"),
            "root:x:0:0::/root:/bin/bash\nportage:x:250:250::/var/lib/portage:/sbin/nologin\n\
             robin:x:1000:1000::/home/robin:/bin/zsh\n",
        )
        .unwrap();
        std::fs::write(r.join("etc/hostname"), "Workbench\n").unwrap();
        let red = Redactions::for_this_computer(r);
        let out = red.apply("robin on Workbench (workbench) in /home/robin; portage stays");
        assert_eq!(
            out,
            "you on computer (computer) in /home/you; portage stays"
        );
    }

    #[test]
    fn shortens_long_recordings_from_the_middle() {
        let text: String = (0..2000).map(|i| format!("line {i}\n")).collect();
        let short = shorten(&text, 1000);
        assert!(short.starts_with("line 0\n"));
        assert!(short.trim_end().ends_with("line 1999"));
        assert!(short.contains("the middle is left out"));
        assert!(short.chars().count() < 1100);
    }

    #[test]
    fn composes_a_report() {
        let sessions = vec![
            Session {
                meta: Meta {
                    args: vec!["status".into()],
                    exit_code: Some(0),
                    started: "20261118-100000".into(),
                    seconds: 1,
                    ..Meta::default()
                },
                text: "✓ Config  the system matches your config".into(),
            },
            Session {
                meta: Meta {
                    args: vec!["add".into(), "mpv".into()],
                    exit_code: Some(1),
                    started: "20261118-100100".into(),
                    seconds: 9,
                    ..Meta::default()
                },
                text: "✗ emerge found a problem\n````\nnested fence".into(),
            },
        ];
        let about = About {
            machine: "laptop".into(),
            bunny: "0.1.0".into(),
            kernel: "7.3.4-gentoo-dist-bin".into(),
            profile: "default/linux/amd64/23.0".into(),
            portage: "3.0.83".into(),
        };
        let (title, body) = compose(
            &sessions,
            Some("✓ Config"),
            &about,
            true,
            &Redactions::default(),
        );
        assert_eq!(title, "laptop: 2 commands, 1 with a problem");
        assert!(body.contains("bunny 0.1.0 · kernel 7.3.4-gentoo-dist-bin"));
        assert!(body.contains("automatically"));
        assert!(body.contains("2026-11-18 10:01 UTC · 9 s"), "{body}");
        assert!(body.contains("<details><summary><b>1. <code>bunny status</code></b> · worked"));
        assert!(body.contains(
            "<details open><summary><b>2. <code>bunny add mpv</code></b> · exit 1 · ⚠ problem"
        ));
        assert!(
            !body.contains("````\nnested"),
            "a fence inside the output would end the block"
        );
        let (one, _) = compose(&sessions[1..], None, &about, false, &Redactions::default());
        assert_eq!(one, "laptop: `bunny add mpv` had a problem");
    }

    #[test]
    fn explains_why_sending_failed() {
        let repo = "o/r";
        let said = "To get started with GitHub CLI, please run:  gh auth login";
        assert!(explain_gh_failure(repo, said, None).contains("gh isn't signed in"));
        let you = Person::named("you", Path::new("/nonexistent"));
        let as_you = explain_gh_failure(repo, said, Some(&you));
        assert!(as_you.starts_with("you isn't signed in"), "{as_you}");
        assert!(as_you.contains("not with sudo or doas"), "{as_you}");
        assert!(explain_gh_failure(repo, "gh: Not Found (HTTP 404)", None)
            .contains("couldn't find o/r"));
        assert!(explain_gh_failure(repo, "something else", None).contains("something else"));
    }

    #[test]
    fn files_the_issue_as_whoever_ran_sudo() {
        // As yourself: gh straight away.
        let plain = issue_command("o/r", "t", None);
        assert_eq!(plain[..3], ["gh", "api", "repos/o/r/issues"]);
        // Under sudo or doas: gh runs as you, with your own settings, so it
        // finds your sign-in instead of root's (the same as bunny publish).
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc")).unwrap();
        std::fs::write(
            root.path().join("etc/passwd"),
            "root:x:0:0::/root:/bin/bash\nyou:x:1000:1000::/home/you:/bin/bash\n",
        )
        .unwrap();
        let you = Person::named("you", root.path());
        let cmd = issue_command("o/r", "t", Some(&you));
        assert_eq!(cmd[..4], ["runuser", "-u", "you", "--"]);
        assert!(cmd.contains(&"HOME=/home/you".to_string()), "{cmd:?}");
        assert!(cmd.contains(&"XDG_CONFIG_HOME=/home/you/.config".to_string()));
        let gh = cmd.iter().position(|a| a == "gh").unwrap();
        assert_eq!(cmd[gh..], plain[..]);
    }

    #[test]
    fn finds_unsent_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let sent = Meta {
            reported: Some("https://x/1".into()),
            ..Meta::default()
        };
        write_meta(&d.join("20261118-100000-status.toml"), &Meta::default()).unwrap();
        write_meta(&d.join("20261118-090000-add.toml"), &Meta::default()).unwrap();
        write_meta(&d.join("20261117-090000-add.toml"), &sent).unwrap();
        std::fs::write(d.join("20261118-100000-status.log"), "x").unwrap();
        assert_eq!(
            unsent(d),
            vec![
                d.join("20261118-090000-add"),
                d.join("20261118-100000-status")
            ]
        );
        assert!(unsent(&d.join("missing")).is_empty());
    }
}
