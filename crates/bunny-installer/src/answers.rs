//! The installer's answers, turned into a `bunny install` command line, and
//! bunny's output turned back into what the screens show. Everything here
//! is plain data, so it's tested without a window.

/// What the screens collected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answers {
    /// The disk's path, like `/dev/nvme0n1`.
    pub disk: String,
    pub recommended: bool,
    pub encrypt: bool,
    /// 0 zram · 1 zram and a swap file · 2 hibernate · 3 none
    pub swap_mode: i32,
    pub swap_size: String,
    pub user: String,
    pub hostname: String,
    pub timezone: String,
    pub keymap: String,
}

/// The `bunny install` arguments for these answers. `--yes` means bunny asks
/// nothing; the password goes on standard input (`--password-stdin`),
/// except on a dry run, which doesn't need one.
pub fn install_args(a: &Answers, dry_run: bool, lsblk: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["install", "--yes", "--disk", &a.disk]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut push = |flag: &str, value: &str| {
        args.push(flag.to_string());
        if !value.is_empty() {
            args.push(value.to_string());
        }
    };
    push("--user", &a.user);
    push("--hostname", &a.hostname);
    push("--timezone", &a.timezone);
    push("--keymap", &a.keymap);
    if a.recommended {
        push("--recommended", "");
    } else {
        if !a.encrypt {
            push("--no-encrypt", "");
        }
        match a.swap_mode {
            1 => push("--swap", a.swap_size.trim()),
            2 => push("--hibernate", ""),
            3 => push("--no-zram", ""),
            _ => {}
        }
    }
    if dry_run {
        push("--dry-run", "");
    } else {
        push("--password-stdin", "");
    }
    if let Some(file) = lsblk {
        push("--lsblk", file);
    }
    args
}

/// One disk from `bunny install --list-disks`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Disk {
    pub name: String,
    pub path: String,
    pub describe: String,
    pub contents: Vec<String>,
    pub unusable: Option<String>,
}

pub fn parse_disks(json: &str) -> anyhow::Result<Vec<Disk>> {
    Ok(serde_json::from_str(json)?)
}

/// The preview from a dry run's output: from "This ERASES" up to the dry
/// run's own note.
pub fn preview(output: &str) -> String {
    let start = output.find("This ERASES").unwrap_or(0);
    let rest = &output[start..];
    let end = ["This is a dry run", "--yes: not asking"]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    rest[..end].trim_end().to_string()
}

/// What went wrong, from bunny's output: its `✗` lines and what follows
/// them, or the last lines if there are none.
pub fn problem(output: &str) -> String {
    if let Some(i) = output.find('✗') {
        return output[i..].trim().to_string();
    }
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join("\n")
}

/// A step's title, if this output line starts one (`==> Encrypting the
/// disk`).
pub fn step(line: &str) -> Option<&str> {
    line.strip_prefix("==> ").map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers() -> Answers {
        Answers {
            disk: "/dev/nvme0n1".into(),
            recommended: true,
            encrypt: true,
            swap_mode: 0,
            swap_size: "8G".into(),
            user: "you".into(),
            hostname: "laptop".into(),
            timezone: "America/Toronto".into(),
            keymap: "us".into(),
        }
    }

    #[test]
    fn the_recommended_setup_is_one_flag() {
        assert_eq!(
            install_args(&answers(), false, None).join(" "),
            "install --yes --disk /dev/nvme0n1 --user you --hostname laptop \
             --timezone America/Toronto --keymap us --recommended --password-stdin"
        );
    }

    #[test]
    fn choices_become_bunnys_own_options() {
        let mut a = answers();
        a.recommended = false;
        a.encrypt = false;
        a.swap_mode = 1;
        a.swap_size = " 4G ".into();
        let args = install_args(&a, true, Some("/tmp/disks.json")).join(" ");
        assert!(
            args.ends_with("--no-encrypt --swap 4G --dry-run --lsblk /tmp/disks.json"),
            "{args}"
        );
        a.swap_mode = 2;
        assert!(install_args(&a, false, None).contains(&"--hibernate".to_string()));
        a.swap_mode = 3;
        assert!(install_args(&a, false, None).contains(&"--no-zram".to_string()));
        // Choosing yourself but keeping the recommended answers adds nothing.
        a.encrypt = true;
        a.swap_mode = 0;
        let args = install_args(&a, false, None).join(" ");
        assert!(
            !args.contains("--recommended") && !args.contains("--no-"),
            "{args}"
        );
    }

    #[test]
    fn disks_come_from_bunnys_list() {
        let json = r#"[{"name":"sda","path":"/dev/sda","describe":"sda  2.0 TB","size":2,
            "contents":["nothing (it's empty)"],"unusable":null},
            {"name":"sdb","path":"/dev/sdb","describe":"sdb  16 GB","size":1,
            "contents":[],"unusable":"it's the USB stick you started from"}]"#;
        let disks = parse_disks(json).unwrap();
        assert_eq!(disks.len(), 2);
        assert_eq!(
            disks[1].unusable.as_deref(),
            Some("it's the USB stick you started from")
        );
    }

    #[test]
    fn the_preview_is_the_part_about_the_disk() {
        let out = "\nThis ERASES everything on vda:\n  - nothing\n\nWhat you'll get:\n  x\n\n\
                   This is a dry run: nothing will be erased.\n==> Partitioning\n";
        assert_eq!(
            preview(out),
            "This ERASES everything on vda:\n  - nothing\n\nWhat you'll get:\n  x"
        );
    }

    #[test]
    fn a_problem_is_bunnys_own_words() {
        assert_eq!(
            problem("This ERASES…\n✗ --user \"Root\": Use lowercase letters.\n"),
            "✗ --user \"Root\": Use lowercase letters."
        );
        assert_eq!(problem("a\nb\n\nc\nd\ne\n"), "b\nc\nd\ne");
    }

    #[test]
    fn steps_are_bunnys_headings() {
        assert_eq!(
            step("==> Encrypting the disk (LUKS2)"),
            Some("Encrypting the disk (LUKS2)")
        );
        assert_eq!(step("    Encrypts the partition"), None);
    }
}
