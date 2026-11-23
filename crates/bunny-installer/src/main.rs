//! Bunny Linux's graphical installer. A thin window over `bunny install`:
//! the screens collect answers, and bunny checks them, shows the preview and
//! does the work. Nothing is decided here.
//!
//! Usage:
//!   bunny-installer              on the live USB (runs bunny install for real)
//!   bunny-installer --demo       a window with pretend disks; only dry runs
//!   bunny-installer --screenshots DIR   draw every screen to PNG files
//!   --bunny PATH                 which bunny to run (default: `bunny`)

mod answers;
mod screenshots;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::include_modules!();

/// The same spinner frames `bunny install`'s quiet progress line uses in the
/// terminal (`crates/bunny/src/progress.rs`), so a step that's still running
/// looks alive here too, not frozen.
const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Two pretend disks for `--demo` (lsblk's JSON, read by bunny itself).
const DEMO_DISKS: &str = r#"{"blockdevices":[
 {"name":"nvme0n1","path":"/dev/nvme0n1","size":"1000204886016","type":"disk","rm":"0","ro":"0","model":"Samsung 990","tran":"nvme","mountpoint":null,"fstype":null,
  "children":[{"name":"nvme0n1p1","path":"/dev/nvme0n1p1","size":"104857600","type":"part","fstype":"vfat","label":"EFI","mountpoint":null},
              {"name":"nvme0n1p2","path":"/dev/nvme0n1p2","size":"999000000000","type":"part","fstype":"ntfs","label":"Windows","mountpoint":null}]},
 {"name":"sda","path":"/dev/sda","size":"2000398934016","type":"disk","rm":"0","ro":"0","model":"WD Blue","tran":"sata","mountpoint":null,"fstype":null}]}"#;

struct Options {
    demo: bool,
    screenshots: Option<String>,
    bunny: String,
}

fn options() -> Result<Options> {
    let mut o = Options {
        demo: false,
        screenshots: None,
        bunny: "bunny".into(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => o.demo = true,
            "--screenshots" => {
                o.screenshots = Some(args.next().context("--screenshots needs a folder")?)
            }
            "--bunny" => o.bunny = args.next().context("--bunny needs a path")?,
            "--help" | "-h" => {
                println!(
                    "{}",
                    include_str!("main.rs")
                        .lines()
                        .take(10)
                        .map(|l| l.trim_start_matches("//!").trim_start())
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("Unknown option: {other} (try --help)"),
        }
    }
    Ok(o)
}

fn main() -> Result<()> {
    let opts = options()?;
    if let Some(dir) = &opts.screenshots {
        return screenshots::draw_all(dir);
    }
    // The demo's pretend disks, as a file bunny can read.
    let lsblk = if opts.demo {
        let path = std::env::temp_dir().join("bunny-installer-demo-disks.json");
        std::fs::write(&path, DEMO_DISKS)?;
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    };

    let ui = Installer::new()?;
    ui.set_demo(opts.demo);
    load_disks(&ui, &opts.bunny, lsblk.as_deref())?;

    // Keeps the current install step's marker spinning so it doesn't look
    // frozen; harmless to run even when nothing's installing, since it's
    // only ever drawn on the install screen.
    let spinner_timer = Timer::default();
    {
        let weak = ui.as_weak();
        let mut frame = 0usize;
        spinner_timer.start(
            TimerMode::Repeated,
            std::time::Duration::from_millis(90),
            move || {
                if let Some(ui) = weak.upgrade() {
                    frame = (frame + 1) % SPINNER_FRAMES.len();
                    ui.set_spinner_frame(SPINNER_FRAMES[frame].to_string().into());
                }
            },
        );
    }

    ui.on_quit_to_terminal(|| std::process::exit(0));
    {
        let weak = ui.as_weak();
        let demo = opts.demo;
        ui.on_restart(move || {
            if !demo {
                let _ = Command::new("reboot").status();
            }
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
        });
    }
    {
        let weak = ui.as_weak();
        let bunny = opts.bunny.clone();
        let lsblk = lsblk.clone();
        ui.on_check_answers(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_checking(true);
            ui.set_problem("".into());
            let args = answers::install_args(&collect(&ui), true, lsblk.as_deref());
            let weak = weak.clone();
            let bunny = bunny.clone();
            std::thread::spawn(move || {
                let result = run_captured(&bunny, &args);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_checking(false);
                    match result {
                        Ok((true, out)) => {
                            ui.set_preview(answers::preview(&out).into());
                            ui.set_page(5);
                        }
                        Ok((false, out)) => ui.set_problem(answers::problem(&out).into()),
                        Err(e) => ui.set_problem(format!("Couldn't run bunny: {e:#}").into()),
                    }
                });
            });
        });
    }
    {
        let weak = ui.as_weak();
        let bunny = opts.bunny.clone();
        let demo = opts.demo;
        let lsblk = lsblk.clone();
        ui.on_start_install(move || {
            let Some(ui) = weak.upgrade() else { return };
            // The demo goes through the steps as a dry run, at a readable
            // pace; the real one installs.
            let args = answers::install_args(&collect(&ui), demo, lsblk.as_deref());
            let password = ui.get_password().to_string();
            ui.set_steps(ModelRc::new(VecModel::from(Vec::<StepItem>::new())));
            ui.set_log("".into());
            ui.set_page(6);
            let weak = weak.clone();
            let bunny = bunny.clone();
            std::thread::spawn(move || install(&bunny, &args, &password, demo, weak));
        });
    }
    ui.run()?;
    Ok(())
}

/// Fills the disk screen from `bunny install --list-disks`, first usable
/// disk chosen.
fn load_disks(ui: &Installer, bunny: &str, lsblk: Option<&str>) -> Result<()> {
    let mut args = vec!["install".to_string(), "--list-disks".to_string()];
    if let Some(file) = lsblk {
        args.extend(["--lsblk".to_string(), file.to_string()]);
    }
    let (ok, out) = run_captured(bunny, &args)?;
    if !ok {
        anyhow::bail!("bunny couldn't list the disks:\n{out}");
    }
    let disks = answers::parse_disks(&out)?;
    let items: Vec<DiskItem> = disks
        .iter()
        .map(|d| DiskItem {
            name: d.name.clone().into(),
            describe: d.describe.clone().into(),
            contents: d.contents.join(", ").into(),
            usable: d.unusable.is_none(),
            why: d.unusable.clone().unwrap_or_default().into(),
        })
        .collect();
    let first = disks.iter().position(|d| d.unusable.is_none()).unwrap_or(0);
    ui.set_disks(ModelRc::new(VecModel::from(items)));
    ui.set_disk(first as i32);
    ui.set_disk_paths(ModelRc::new(VecModel::from(
        disks
            .iter()
            .map(|d| SharedString::from(d.path.as_str()))
            .collect::<Vec<_>>(),
    )));
    Ok(())
}

/// The answers on the screens now.
fn collect(ui: &Installer) -> answers::Answers {
    let disk = usize::try_from(ui.get_disk()).unwrap_or(0);
    let paths = ui.get_disk_paths();
    use slint::Model;
    answers::Answers {
        disk: paths
            .row_data(disk)
            .map(|p| p.to_string())
            .unwrap_or_default(),
        recommended: ui.get_recommended(),
        encrypt: ui.get_encrypt(),
        swap_mode: ui.get_swap_mode(),
        swap_size: ui.get_swap_size().to_string(),
        user: ui.get_user().trim().to_string(),
        hostname: ui.get_hostname().trim().to_string(),
        timezone: ui.get_timezone().trim().to_string(),
        keymap: ui.get_keymap().trim().to_string(),
    }
}

/// Runs bunny and returns whether it worked and everything it printed.
fn run_captured(bunny: &str, args: &[String]) -> Result<(bool, String)> {
    let out = Command::new(bunny)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("couldn't start {bunny}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// Runs the install, showing each of bunny's steps as it starts, and the
/// full output under "Show the details".
fn install(bunny: &str, args: &[String], password: &str, demo: bool, weak: slint::Weak<Installer>) {
    let started = Command::new(bunny)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match started {
        Ok(c) => c,
        Err(e) => return finish(&weak, false, format!("Couldn't start {bunny}: {e}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if !demo {
            let _ = writeln!(stdin, "{password}");
        }
    }
    // bunny's messages on standard error go to the details too.
    let stderr = child.stderr.take().map(|err| {
        std::thread::spawn(move || {
            let mut text = String::new();
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                text.push_str(&line);
                text.push('\n');
            }
            text
        })
    });
    let mut steps: Vec<String> = Vec::new();
    let mut log = String::new();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            log.push_str(&line);
            log.push('\n');
            if let Some(title) = answers::step(&line) {
                steps.push(title.to_string());
                if demo {
                    std::thread::sleep(std::time::Duration::from_millis(450));
                }
            }
            show(&weak, &steps, false, &log);
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    if let Some(err) = stderr.and_then(|h| h.join().ok()) {
        log.push_str(&err);
    }
    show(&weak, &steps, ok, &log);
    finish(&weak, ok, log);
}

/// The step list: every step before the last is done; the last is done
/// only when bunny finished.
fn show(weak: &slint::Weak<Installer>, steps: &[String], all_done: bool, log: &str) {
    let items: Vec<(String, bool)> = steps
        .iter()
        .enumerate()
        .map(|(i, t)| (t.clone(), all_done || i + 1 < steps.len()))
        .collect();
    let log = log.to_string();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            let items: Vec<StepItem> = items
                .into_iter()
                .map(|(title, done)| StepItem {
                    title: title.into(),
                    done,
                })
                .collect();
            ui.set_steps(ModelRc::new(VecModel::from(items)));
            ui.set_log(log.into());
        }
    });
}

fn finish(weak: &slint::Weak<Installer>, ok: bool, log: String) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_log(log.into());
            ui.set_page(if ok { 7 } else { 8 });
        }
    });
}
