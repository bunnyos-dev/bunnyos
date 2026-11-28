//! `[hardware]`: this computer's graphics cards and CPU features.
//!
//! Gentoo calls them `VIDEO_CARDS` and `CPU_FLAGS_X86`. They're USE flags
//! for every package at once, so bunny writes them into its own package.use
//! file as `*/*` lines (Gentoo's documented package.use syntax), first in
//! the file so a package's own `[pkg]` flags still win:
//!
//! ```text
//! */* VIDEO_CARDS: -* intel nvidia
//! */* CPU_FLAGS_X86: aes avx avx2 …
//! ```
//!
//! `video_cards` is the whole list (`-*` first, as Gentoo's NVIDIA guide
//! writes it); `cpu_flags` adds to Gentoo's defaults, as `cpuid2cpuflags`
//! output does. bunny never changes make.conf.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

use crate::apply::Runner;
use crate::config::Problem;

/// The `[hardware]` section. A setting that's left out isn't managed:
/// bunny writes no line for it.
#[derive(Debug, Default, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Hardware {
    /// Gentoo's `VIDEO_CARDS`, e.g. `["intel", "nvidia"]`.
    #[serde(default)]
    pub video_cards: Option<Vec<String>>,
    /// Gentoo's `CPU_FLAGS_X86`, e.g. `["aes", "avx", "avx2"]`.
    #[serde(default)]
    pub cpu_flags: Option<Vec<String>>,
}

/// One of the settings `[hardware]` manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    VideoCards,
    CpuFlags,
}

impl Group {
    pub const ALL: [Group; 2] = [Group::VideoCards, Group::CpuFlags];

    /// Portage's name, as written in package.use.
    pub fn portage(self) -> &'static str {
        match self {
            Group::VideoCards => "VIDEO_CARDS",
            Group::CpuFlags => "CPU_FLAGS_X86",
        }
    }

    /// The key in `[hardware]`.
    pub fn key(self) -> &'static str {
        match self {
            Group::VideoCards => "video_cards",
            Group::CpuFlags => "cpu_flags",
        }
    }

    /// In plain words.
    pub fn plain(self) -> &'static str {
        match self {
            Group::VideoCards => "graphics cards",
            Group::CpuFlags => "CPU features",
        }
    }

    fn from_portage(name: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.portage() == name)
    }

    /// The words bunny writes for `values`: the whole list for graphics
    /// cards (`-*` first), additions to Gentoo's defaults for CPU features.
    fn words(self, values: &[String]) -> Vec<String> {
        let mut words = Vec::new();
        if self == Group::VideoCards {
            words.push("-*".to_string());
        }
        words.extend(values.iter().cloned());
        words
    }
}

impl Hardware {
    pub fn get(&self, group: Group) -> Option<&Vec<String>> {
        match group {
            Group::VideoCards => self.video_cards.as_ref(),
            Group::CpuFlags => self.cpu_flags.as_ref(),
        }
    }

    pub fn set(&mut self, group: Group, values: Option<Vec<String>>) {
        match group {
            Group::VideoCards => self.video_cards = values,
            Group::CpuFlags => self.cpu_flags = values,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.video_cards.is_none() && self.cpu_flags.is_none()
    }

    /// A host file's `[hardware]` on top: each setting it has wins.
    pub fn add_host(&mut self, host: Hardware) {
        for group in Group::ALL {
            if let Some(values) = host.get(group) {
                self.set(group, Some(values.clone()));
            }
        }
    }

    /// The `*/*` lines for bunny's package.use file.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for group in Group::ALL {
            let Some(values) = self.get(group) else {
                continue;
            };
            let words = group.words(values);
            if words.is_empty() {
                continue;
            }
            let _ = writeln!(out, "*/* {}: {}", group.portage(), words.join(" "));
        }
        out
    }

    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for group in Group::ALL {
            let Some(values) = self.get(group) else {
                continue;
            };
            let mut seen = std::collections::BTreeSet::new();
            for value in values {
                if !is_value(value) {
                    problems.push(Problem {
                        what: format!(
                            "\"{value}\" in `{}` under [hardware] isn't a {} name.",
                            group.key(),
                            group.portage()
                        ),
                        fix: match group {
                            Group::VideoCards => "Use Gentoo's names in lowercase, for example \
                                                  \"intel\", \"nvidia\", \"amdgpu\" or \"radeonsi\" \
                                                  (`bunny hardware` suggests them)."
                                .into(),
                            Group::CpuFlags => "Use Gentoo's names in lowercase, for example \
                                                \"aes\" or \"sse4_2\" (`bunny hardware` suggests \
                                                them)."
                                .into(),
                        },
                    });
                } else if !seen.insert(value) {
                    problems.push(Problem {
                        what: format!(
                            "\"{value}\" is listed twice in `{}` under [hardware].",
                            group.key()
                        ),
                        fix: "Remove one of them.".into(),
                    });
                }
            }
        }
        problems
    }
}

/// A VIDEO_CARDS or CPU_FLAGS_X86 value: lowercase letters, digits and `_`.
fn is_value(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// What a `*/*` package.use line sets, when it only sets `[hardware]`
/// settings: `*/* VIDEO_CARDS: -* nvidia` → `[(VideoCards, ["-*", "nvidia"])]`.
/// `None` for anything else (other `*/*` lines stay "kept" lines).
pub fn parse_line(line: &str) -> Option<Vec<(Group, Vec<String>)>> {
    let mut words = line.split_whitespace();
    if words.next()? != "*/*" {
        return None;
    }
    let mut groups: Vec<(Group, Vec<String>)> = Vec::new();
    for word in words {
        if let Some(name) = word.strip_suffix(':') {
            groups.push((Group::from_portage(name)?, Vec::new()));
        } else {
            // A plain flag before any group: not a [hardware] line.
            groups.last_mut()?.1.push(word.to_string());
        }
    }
    (!groups.is_empty()).then_some(groups)
}

/// What the `*/*` lines say, in Portage's order (later lines add to or
/// replace earlier ones). A list that starts with `-*` replaces Gentoo's
/// defaults; one that doesn't adds to them.
pub fn evaluate<'a>(
    lines: impl IntoIterator<Item = &'a (Group, Vec<String>)>,
) -> BTreeMap<Group, Vec<String>> {
    let mut state: BTreeMap<Group, Vec<String>> = BTreeMap::new();
    for (group, words) in lines {
        let values = state.entry(*group).or_default();
        for word in words {
            if word == "-*" {
                values.clear();
                values.push("-*".into());
            } else if let Some(off) = word.strip_prefix('-') {
                values.retain(|v| v != off);
            } else if !values.contains(word) {
                values.push(word.clone());
            }
        }
    }
    state
}

/// A `[hardware]` setting a switch would change.
#[derive(Debug, PartialEq, Clone)]
pub struct Change {
    pub group: Group,
    /// What package.use says now (`-*` first when it replaces Gentoo's
    /// defaults; empty when no line sets it).
    pub from: Vec<String>,
    /// What bunny would write, in the same form. `None`: the setting was
    /// taken out of `[hardware]`, so bunny's line goes and Gentoo's
    /// defaults (or your own lines) apply again.
    pub to: Option<Vec<String>>,
}

/// Shows a setting in plain words: `intel nvidia`, `Gentoo's defaults + aes`.
pub fn show(words: &[String]) -> String {
    match words.split_first() {
        None => "not set (Gentoo's defaults)".to_string(),
        Some((first, rest)) if first == "-*" => {
            if rest.is_empty() {
                "none".to_string()
            } else {
                rest.join(" ")
            }
        }
        _ => format!("Gentoo's defaults + {}", words.join(" ")),
    }
}

impl Change {
    pub fn render_to(&self) -> String {
        match &self.to {
            Some(words) => show(words),
            None => "not managed by bunny any more".to_string(),
        }
    }
}

/// Compares `[hardware]` with what package.use says now. `all` is every
/// `*/*` [hardware] line on the system in Portage's order; `bunnys` the ones
/// in bunny's own file (a setting taken out of `[hardware]` is only a change
/// when bunny wrote it).
pub fn changes(
    wanted: &Hardware,
    all: &[(Group, Vec<String>)],
    bunnys: &[(Group, Vec<String>)],
) -> Vec<Change> {
    let now = evaluate(all);
    let ours = evaluate(bunnys);
    let as_set = |w: &[String]| w.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    let mut changes = Vec::new();
    for group in Group::ALL {
        let from = now.get(&group).cloned().unwrap_or_default();
        match wanted.get(group) {
            Some(values) => {
                let to = group.words(values);
                if as_set(&from) != as_set(&to) {
                    changes.push(Change {
                        group,
                        from,
                        to: Some(to),
                    });
                }
            }
            None if ours.contains_key(&group) => changes.push(Change {
                group,
                from,
                to: None,
            }),
            None => {}
        }
    }
    changes
}

/// Which `[hardware]` settings Gentoo's make.conf sets (`VIDEO_CARDS="…"`).
/// Read only, for `bunny import` to suggest.
pub fn from_make_conf(root: &Path) -> Hardware {
    let mut found = Hardware::default();
    let text = std::fs::read_to_string(root.join("etc/portage/make.conf")).unwrap_or_default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let Some(group) = Group::from_portage(name.trim()) else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        let values: Vec<String> = value
            .split_whitespace()
            .filter(|w| is_value(w))
            .map(String::from)
            .collect();
        found.set(group, Some(values));
    }
    found
}

// ---------------------------------------------------------------------------
// `bunny hardware`: suggestions for this computer.

/// A graphics chip found on this computer.
#[derive(Debug, PartialEq)]
pub struct Gpu {
    /// PCI vendor id, e.g. `10de`.
    pub vendor: String,
    /// What lspci calls it, when lspci could say.
    pub description: Option<String>,
}

/// What `bunny hardware` found.
#[derive(Debug, Default, PartialEq)]
pub struct Suggestion {
    pub gpus: Vec<Gpu>,
    pub video_cards: Vec<String>,
    /// `None` when cpuid2cpuflags isn't installed.
    pub cpu_flags: Option<Vec<String>>,
    /// lspci couldn't be run (it's in sys-apps/pciutils).
    pub no_lspci: bool,
}

/// Gentoo's VIDEO_CARDS values for a PCI vendor id.
fn cards_for(vendor: &str) -> &'static [&'static str] {
    match vendor {
        "10de" => &["nvidia"],
        "8086" => &["intel"],
        "1002" => &["amdgpu", "radeonsi"],
        // Virtual machines: QEMU's virtio-gpu, VMware, QXL.
        "1af4" => &["virgl"],
        "15ad" => &["vmware"],
        "1b36" => &["qxl"],
        _ => &[],
    }
}

/// The vendor's name, for the list of what was found.
fn vendor_name(vendor: &str) -> &'static str {
    match vendor {
        "10de" => "NVIDIA",
        "8086" => "Intel",
        "1002" => "AMD",
        "1af4" => "Virtual (virtio)",
        "15ad" => "VMware",
        "1b36" => "Virtual (QXL)",
        "1234" => "Virtual (QEMU's standard screen)",
        _ => "Unknown",
    }
}

/// Graphics chips from `lspci -nn` output: lines for VGA, 3D and display
/// controllers, with the vendor from the `[vendor:device]` ids.
pub fn gpus_from_lspci(text: &str) -> Vec<Gpu> {
    text.lines()
        .filter(|l| {
            [
                "VGA compatible controller",
                "3D controller",
                "Display controller",
            ]
            .iter()
            .any(|kind| l.contains(kind))
        })
        .filter_map(|l| {
            // The last `[xxxx:yyyy]` on the line is the vendor and device.
            let ids = l.rsplit('[').find_map(|part| {
                let inside = part.split(']').next()?;
                let (vendor, device) = inside.split_once(':')?;
                (vendor.len() == 4
                    && device.len() == 4
                    && vendor.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| vendor.to_lowercase())
            })?;
            let description = l
                .split_once(": ")
                .map(|(_, d)| d.trim().to_string())
                .filter(|d| !d.is_empty());
            Some(Gpu {
                vendor: ids,
                description,
            })
        })
        .collect()
}

/// Graphics chips from /sys/class/drm (cardN/device/vendor), for when
/// lspci isn't installed.
pub fn gpus_from_sys(root: &Path) -> Vec<Gpu> {
    let mut cards: Vec<(u32, String)> = std::fs::read_dir(root.join("sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let number: u32 = name.strip_prefix("card")?.parse().ok()?;
            let vendor = std::fs::read_to_string(e.path().join("device/vendor")).ok()?;
            let vendor = vendor.trim().trim_start_matches("0x").to_lowercase();
            Some((number, vendor))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .map(|(_, vendor)| Gpu {
            vendor,
            description: None,
        })
        .collect()
}

/// `cpuid2cpuflags` prints `CPU_FLAGS_X86: aes avx …` (older versions put
/// `*/* ` in front).
pub fn parse_cpuid2cpuflags(text: &str) -> Option<Vec<String>> {
    let (_, flags) = text.split_once("CPU_FLAGS_X86:")?;
    let flags: Vec<String> = flags
        .split_whitespace()
        .filter(|w| is_value(w))
        .map(String::from)
        .collect();
    (!flags.is_empty()).then_some(flags)
}

/// Looks at this computer. Read only: runs `lspci -nn` and `cpuid2cpuflags`.
pub fn suggest(root: &Path, runner: &mut dyn Runner) -> Suggestion {
    let lspci = runner
        .output(&["lspci".to_string(), "-nn".to_string()])
        .ok()
        .flatten();
    let mut suggestion = Suggestion::default();
    match lspci {
        Some(text) => suggestion.gpus = gpus_from_lspci(&text),
        None => suggestion.no_lspci = true,
    }
    if suggestion.gpus.is_empty() {
        suggestion.gpus = gpus_from_sys(root);
    }
    // Integrated graphics first, then the rest, each name once.
    let order = [
        "intel", "amdgpu", "radeonsi", "nvidia", "virgl", "vmware", "qxl",
    ];
    for card in order {
        if suggestion
            .gpus
            .iter()
            .any(|g| cards_for(&g.vendor).contains(&card))
        {
            suggestion.video_cards.push(card.to_string());
        }
    }
    suggestion.cpu_flags = runner
        .output(&["cpuid2cpuflags".to_string()])
        .ok()
        .flatten()
        .and_then(|t| parse_cpuid2cpuflags(&t));
    suggestion
}

fn quoted(values: &[String]) -> String {
    values
        .iter()
        .map(|v| format!("\"{v}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Suggestion {
    /// What `bunny hardware` prints. `host_file`: where this computer's
    /// host file is, to say where the block goes.
    pub fn render(&self, host_file: &str) -> String {
        let mut out = String::new();
        if self.gpus.is_empty() {
            out.push_str("No graphics chips found");
            if self.no_lspci {
                out.push_str(
                    " (lspci isn't installed: bunny add sys-apps/pciutils,\n\
                     then run bunny hardware again)",
                );
            }
            out.push_str(".\n\n");
        } else {
            out.push_str("Graphics found on this computer:\n");
            for gpu in &self.gpus {
                let what = gpu
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("PCI vendor {}", gpu.vendor));
                let _ = writeln!(out, "  · {}: {what}", vendor_name(&gpu.vendor));
            }
            out.push('\n');
        }

        let _ = writeln!(out, "Ready to paste into {host_file}:\n");
        out.push_str("[hardware]\n");
        if self.video_cards.is_empty() {
            out.push_str("# video_cards = []   # nothing bunny recognises; see the Guide\n");
        } else {
            let _ = writeln!(out, "video_cards = [{}]", quoted(&self.video_cards));
        }
        match &self.cpu_flags {
            Some(flags) => {
                let _ = writeln!(out, "cpu_flags = [{}]", quoted(flags));
            }
            None => out.push_str("# cpu_flags: see below\n"),
        }
        out.push('\n');

        let has = |card: &str| self.video_cards.iter().any(|c| c == card);
        if has("nvidia") {
            out.push_str(
                "NVIDIA: add the driver too, x11-drivers/nvidia-drivers (`packages` in the\n\
                 same file). Newer cards use the open kernel module: under\n\
                 [pkg.\"x11-drivers/nvidia-drivers\"], use = [\"kernel-open\"]. The Guide's\n\
                 \"Graphics cards and CPU features\" page has the whole example.\n",
            );
            if has("intel") || has("amdgpu") {
                out.push_str(
                    "Two graphics chips (a laptop, usually): keep both. The built-in one keeps\n\
                     the screen and power saving working.\n",
                );
            }
            out.push('\n');
        }
        match &self.cpu_flags {
            Some(_) => out.push_str(
                "CPU features are a trade-off: packages that use them (video codecs, crypto\n\
                 libraries…) no longer match Gentoo's prebuilt copies, so they compile on\n\
                 this computer instead of downloading. Leave cpu_flags out to keep downloading.\n\n",
            ),
            None => out.push_str(
                "CPU features: cpuid2cpuflags isn't installed, so they can't be read.\n\
                 To get them: bunny add app-portage/cpuid2cpuflags, then run bunny hardware again.\n\n",
            ),
        }
        out.push_str("Then check what would change: bunny plan\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn renders_lines_and_checks_values() {
        let hw = Hardware {
            video_cards: Some(s(&["intel", "nvidia"])),
            cpu_flags: Some(s(&["aes", "sse4_2"])),
        };
        assert_eq!(
            hw.render(),
            "*/* VIDEO_CARDS: -* intel nvidia\n*/* CPU_FLAGS_X86: aes sse4_2\n"
        );
        assert!(hw.problems().is_empty());
        // No graphics at all is still a line; no CPU flags is nothing.
        let none = Hardware {
            video_cards: Some(vec![]),
            cpu_flags: Some(vec![]),
        };
        assert_eq!(none.render(), "*/* VIDEO_CARDS: -*\n");
        assert_eq!(Hardware::default().render(), "");

        let bad = Hardware {
            video_cards: Some(s(&["NVIDIA", "nvidia", "nvidia"])),
            cpu_flags: Some(s(&["-*"])),
        };
        let text: Vec<String> = bad.problems().into_iter().map(|p| p.what).collect();
        assert_eq!(text.len(), 3, "{text:?}");
        assert!(text[0].contains("\"NVIDIA\""));
        assert!(text[1].contains("listed twice"));
    }

    #[test]
    fn the_host_wins_one_setting_at_a_time() {
        let mut hw = Hardware {
            video_cards: Some(s(&["amdgpu"])),
            cpu_flags: Some(s(&["aes"])),
        };
        hw.add_host(Hardware {
            video_cards: Some(s(&["intel", "nvidia"])),
            cpu_flags: None,
        });
        assert_eq!(hw.video_cards, Some(s(&["intel", "nvidia"])));
        assert_eq!(hw.cpu_flags, Some(s(&["aes"])));
    }

    #[test]
    fn parses_only_hardware_lines() {
        assert_eq!(
            parse_line("*/* VIDEO_CARDS: -* nvidia"),
            Some(vec![(Group::VideoCards, s(&["-*", "nvidia"]))])
        );
        assert_eq!(
            parse_line("*/*  CPU_FLAGS_X86: aes avx VIDEO_CARDS: intel"),
            Some(vec![
                (Group::CpuFlags, s(&["aes", "avx"])),
                (Group::VideoCards, s(&["intel"]))
            ])
        );
        assert_eq!(parse_line("*/* PYTHON_TARGETS: python3_13"), None);
        assert_eq!(parse_line("*/* wayland VIDEO_CARDS: intel"), None);
        assert_eq!(
            parse_line("x11-drivers/nvidia-drivers VIDEO_CARDS: nvidia"),
            None
        );
        assert_eq!(parse_line("*/*"), None);
    }

    #[test]
    fn evaluates_in_order_and_compares() {
        let lines = vec![
            (Group::VideoCards, s(&["nvidia"])),
            (Group::CpuFlags, s(&["aes", "avx"])),
            (Group::CpuFlags, s(&["-avx", "sse4_2"])),
        ];
        let now = evaluate(&lines);
        assert_eq!(now[&Group::VideoCards], ["nvidia"]);
        assert_eq!(now[&Group::CpuFlags], ["aes", "sse4_2"]);
        assert_eq!(show(&now[&Group::VideoCards]), "Gentoo's defaults + nvidia");

        // `-* nvidia` isn't the same as adding nvidia to Gentoo's defaults.
        let wanted = Hardware {
            video_cards: Some(s(&["nvidia"])),
            cpu_flags: Some(s(&["sse4_2", "aes"])),
        };
        let c = changes(&wanted, &lines, &[]);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].group, Group::VideoCards);
        assert_eq!(c[0].render_to(), "nvidia");

        let ours = vec![(Group::VideoCards, s(&["-*", "nvidia"]))];
        assert!(
            changes(&wanted, &[ours[0].clone(), lines[1].clone()], &ours)
                .iter()
                .all(|c| c.group != Group::VideoCards)
        );
        // Taken out of [hardware]: bunny's line goes.
        let c = changes(&Hardware::default(), &ours, &ours);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].to, None);
        // Someone else's line and no [hardware]: nothing to do.
        assert!(changes(&Hardware::default(), &lines, &[]).is_empty());
    }

    #[test]
    fn reads_make_conf_as_a_suggestion() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc/portage")).unwrap();
        std::fs::write(
            root.path().join("etc/portage/make.conf"),
            "COMMON_FLAGS=\"-O2\"\nVIDEO_CARDS=\"intel nvidia\" # laptop\nCPU_FLAGS_X86='aes avx'\n",
        )
        .unwrap();
        let hw = from_make_conf(root.path());
        assert_eq!(hw.video_cards, Some(s(&["intel", "nvidia"])));
        assert_eq!(hw.cpu_flags, Some(s(&["aes", "avx"])));
        assert!(from_make_conf(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn finds_graphics_chips() {
        let lspci = "\
00:02.0 VGA compatible controller [0300]: Intel Corporation Arrow Lake-S [Intel Graphics] [8086:7d67] (rev 06)
00:14.0 USB controller [0c03]: Intel Corporation Device [8086:7f6e]
01:00.0 VGA compatible controller [0300]: NVIDIA Corporation GB203M [GeForce RTX 5080 Laptop GPU] [10de:2c59] (rev a1)
";
        let gpus = gpus_from_lspci(lspci);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].vendor, "8086");
        assert_eq!(gpus[1].vendor, "10de");
        assert!(gpus[1].description.as_ref().unwrap().contains("RTX 5080"));

        let root = tempfile::tempdir().unwrap();
        for (card, vendor) in [("card1", "0x1002\n"), ("card0", "0x8086\n")] {
            let dir = root.path().join("sys/class/drm").join(card).join("device");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("vendor"), vendor).unwrap();
        }
        // A connector, not a card.
        std::fs::create_dir_all(root.path().join("sys/class/drm/card0-eDP-1")).unwrap();
        let gpus = gpus_from_sys(root.path());
        assert_eq!(
            gpus.iter().map(|g| g.vendor.as_str()).collect::<Vec<_>>(),
            ["8086", "1002"]
        );
    }

    #[test]
    fn reads_cpuid2cpuflags() {
        assert_eq!(
            parse_cpuid2cpuflags("CPU_FLAGS_X86: aes avx avx2 sse4_2\n"),
            Some(s(&["aes", "avx", "avx2", "sse4_2"]))
        );
        assert_eq!(
            parse_cpuid2cpuflags("*/* CPU_FLAGS_X86: mmx sse\n"),
            Some(s(&["mmx", "sse"]))
        );
        assert_eq!(parse_cpuid2cpuflags("nothing"), None);
    }

    /// Answers `lspci -nn` and `cpuid2cpuflags` from fixed text, and records
    /// what was asked.
    struct Fake {
        lspci: Option<&'static str>,
        cpuid: Option<&'static str>,
        asked: Vec<Vec<String>>,
    }

    impl Runner for Fake {
        fn run(&mut self, cmd: &[String]) -> anyhow::Result<bool> {
            panic!("bunny hardware must only read: {cmd:?}");
        }
        fn output(&mut self, cmd: &[String]) -> anyhow::Result<Option<String>> {
            self.asked.push(cmd.to_vec());
            let answer = match cmd[0].as_str() {
                "lspci" => self.lspci,
                "cpuid2cpuflags" => self.cpuid,
                _ => None,
            };
            match answer {
                Some(text) => Ok(Some(text.to_string())),
                None => anyhow::bail!("Couldn't start `{}`. Is it installed?", cmd[0]),
            }
        }
    }

    #[test]
    fn suggests_a_block_for_an_nvidia_laptop() {
        let root = tempfile::tempdir().unwrap();
        let mut fake = Fake {
            lspci: Some(
                "00:02.0 VGA compatible controller [0300]: Intel Corporation Device [8086:7d67]\n\
                 01:00.0 VGA compatible controller [0300]: NVIDIA Corporation Device [10de:2c59]\n",
            ),
            cpuid: Some("CPU_FLAGS_X86: aes avx avx2\n"),
            asked: vec![],
        };
        let got = suggest(root.path(), &mut fake);
        assert_eq!(
            fake.asked,
            [
                vec!["lspci".to_string(), "-nn".to_string()],
                vec!["cpuid2cpuflags".to_string()]
            ]
        );
        assert_eq!(got.video_cards, ["intel", "nvidia"]);
        let text = got.render("/etc/bunny/hosts/laptop.toml");
        assert!(
            text.contains(
                "[hardware]\nvideo_cards = [\"intel\", \"nvidia\"]\ncpu_flags = [\"aes\", \"avx\", \"avx2\"]\n"
            ),
            "{text}"
        );
        assert!(text.contains("x11-drivers/nvidia-drivers"), "{text}");
        assert!(text.contains("keep both"), "{text}");
        // The pasted block is valid bunny.toml.
        let block: String = text
            .split("[hardware]")
            .nth(1)
            .unwrap()
            .split("\n\n")
            .next()
            .unwrap()
            .to_string();
        let parsed: toml::Value = toml::from_str(&format!("[hardware]{block}")).unwrap();
        assert_eq!(
            parsed["hardware"]["video_cards"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn says_what_to_install_when_tools_are_missing() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("sys/class/drm/card0/device");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vendor"), "0x1002\n").unwrap();
        let mut fake = Fake {
            lspci: None,
            cpuid: None,
            asked: vec![],
        };
        let got = suggest(root.path(), &mut fake);
        assert_eq!(got.video_cards, ["amdgpu", "radeonsi"]);
        assert_eq!(got.cpu_flags, None);
        let text = got.render("your host file");
        assert!(
            text.contains("bunny add app-portage/cpuid2cpuflags"),
            "{text}"
        );
        assert!(!text.contains("nvidia-drivers"), "{text}");

        // Nothing found at all.
        let empty = tempfile::tempdir().unwrap();
        let text = suggest(empty.path(), &mut fake).render("your host file");
        assert!(text.contains("bunny add sys-apps/pciutils"), "{text}");
    }
}
