//! The config `bunny install` writes into the new system: a starter
//! `bunny.toml` that works on any computer, and `hosts/<host name>.toml`
//! with what belongs to this one (what it needs to start, and a record of
//! how its disk was set up).

use std::path::Path;

use super::layout::{self, Choices, Uuids};
use super::system::Account;

const GUIDE: &str = "https://bunnyos-dev.github.io/bunnyos/";

/// What this computer needs to start: the kernel and its tools, the boot
/// menu, and the disk's filesystems. They go in the host file, so a config
/// written on another computer still installs them here.
pub fn boot_packages(choices: &Choices) -> Vec<&'static str> {
    let mut out = vec![
        "sys-kernel/installkernel",
        "sys-kernel/gentoo-kernel-bin",
        "sys-kernel/linux-firmware",
        "sys-boot/grub",
        "sys-boot/efibootmgr",
        "sys-fs/btrfs-progs",
        "sys-fs/dosfstools",
    ];
    if choices.encrypt {
        out.push("sys-fs/cryptsetup");
    }
    out
}

/// The rest of the minimal system, in the main config.
pub const BASE_PACKAGES: [&str; 6] = [
    "net-misc/dhcpcd",
    "net-wireless/iwd",
    "app-admin/sysklogd",
    "app-admin/sudo",
    "dev-vcs/git",
    // So nano, less and the like work over SSH from kitty (its terminal
    // type, xterm-kitty, isn't in the base system's list).
    "x11-terms/kitty-terminfo",
];

/// Packages from Bunny Linux's own repository, in the main config after
/// [`BASE_PACKAGES`]. `bunny-release` makes the system say Bunny Linux
/// (`/etc/os-release`, and the logo for fastfetch). They're installed by the
/// first `bunny switch`, which sets up the repository first; the install
/// script's own emerge (Gentoo's packages only) leaves them out.
pub const OWN_PACKAGES: [&str; 1] = ["sys-apps/bunny-release"];

/// Services in the main config.
pub fn services(account: &Account) -> Vec<&'static str> {
    let mut out = vec!["dhcpcd", "sysklogd"];
    if account.ssh {
        out.push("sshd");
    }
    out
}

/// Services for this computer only, in its host file: iwd (Wi-Fi) on a
/// computer that has Wi-Fi. Without it started, an install done over Wi-Fi
/// came up offline; in the main config it would fail on a desktop without
/// Wi-Fi.
pub fn host_services(wifi: bool) -> Vec<&'static str> {
    if wifi {
        vec!["iwd"]
    } else {
        Vec::new()
    }
}

/// Whether this computer has a Wi-Fi card (a network device with a
/// `wireless` folder in /sys), looked at from the live USB during install.
pub fn has_wifi(root: &Path) -> bool {
    std::fs::read_dir(root.join("sys/class/net"))
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().join("wireless").is_dir())
        })
        .unwrap_or(false)
}

/// Services the base system (stage3) already starts, listed in the config
/// so `bunny status` doesn't call them "changed outside bunny".
pub const BASE_SERVICES: [&str; 2] = ["local", "netmount"];

fn list(items: &[&str]) -> String {
    list_with_examples(items, &[])
}

/// A list, with commented-out example lines before its closing bracket.
fn list_with_examples(items: &[&str], examples: &[&str]) -> String {
    let mut out = String::from("[\n");
    for item in items {
        out.push_str(&format!("    \"{item}\",\n"));
    }
    for line in examples {
        out.push_str(&format!("    # {line}\n"));
    }
    out.push(']');
    out
}

/// The starter config's examples, commented out: how each kind of setting
/// is written. Each is a line of the file without its `# `, so a test can
/// uncomment exactly these and check the result. Only structural examples
/// (nothing that goes stale as Gentoo changes): a package, a service with
/// the package it needs, and a package's USE flags and keywords.
pub const PACKAGE_EXAMPLES: [&str; 2] = ["\"app-misc/tmux\",", "\"net-misc/chrony\","];
pub const SERVICE_EXAMPLES: [&str; 1] = ["\"chronyd\","];
pub const PKG_EXAMPLES: [&str; 3] = [
    "[pkg.\"app-misc/tmux\"]",
    "use = [\"vim-syntax\"]",
    "keywords = [\"~amd64\"]",
];

/// Every example line, as it appears in the file (commented out).
#[cfg(test)]
pub fn example_lines() -> Vec<&'static str> {
    PACKAGE_EXAMPLES
        .iter()
        .chain(SERVICE_EXAMPLES.iter())
        .chain(PKG_EXAMPLES.iter())
        .copied()
        .collect()
}

/// The starter `bunny.toml`.
pub fn config(account: &Account) -> String {
    format!(
        "# bunny.toml: your whole system in one file.
#
# Change something here, then run `bunny switch` to apply it (it shows what
# will change first, and saves a copy of the system so you can undo).
# The Guide explains every part: {GUIDE}
#
# Written by bunny install. Settings that belong to this computer only (what
# it needs to start, and how its disk was set up) are in hosts/{host}.toml,
# so this file works on any computer: keep it in git and use it everywhere.

# What you want installed. Add things with `bunny add <name>`, or here.
# The lines starting with # at the end are examples: remove the # to use one.
# More: {GUIDE}tasks/install.html
packages = {packages}

# Package repositories besides Gentoo's own. \"bunny\" is Bunny Linux's own
# (bunny-release, which makes the system say Bunny Linux).
repositories = [\"{own}\"]

# Services that start when the computer does. The example, chronyd, keeps
# the clock right; it needs net-misc/chrony in `packages` too.
# More: {GUIDE}reference/config.html#services
services = {services}

# Settings for one package: USE flags turn its features on, or off with a
# - in front, and keywords = [\"~amd64\"] accepts its testing versions. An
# example (remove the # from all three lines to use it):
{pkg_examples}
# More: {GUIDE}tasks/use-flags.html

# Who can use sudo.
[security]
privilege = \"sudo\"
admins = [\"{user}\"]

# An \"Undo\" entry in the boot menu after every change, for when a change
# stops the computer from starting.
[boot]
undo_entry = true
",
        host = account.hostname,
        packages = list_with_examples(
            &[&BASE_PACKAGES[..], &OWN_PACKAGES[..]].concat(),
            &PACKAGE_EXAMPLES
        ),
        own = crate::overlays::OWN,
        services = list_with_examples(
            &[services(account), BASE_SERVICES.to_vec()].concat(),
            &SERVICE_EXAMPLES
        ),
        pkg_examples = PKG_EXAMPLES
            .iter()
            .map(|l| format!("# {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
        user = account.username,
    )
}

/// `hosts/<host name>.toml`, for the computer bunny install runs on.
pub fn host_file(choices: &Choices, account: &Account, uuids: &Uuids) -> String {
    host_file_with(choices, account, uuids, has_wifi(Path::new("/")))
}

fn host_file_with(choices: &Choices, account: &Account, uuids: &Uuids, wifi: bool) -> String {
    let mut machine = format!(
        "disk = \"{}\"\nencrypted = {}\n",
        choices.disk, choices.encrypt
    );
    machine.push_str(&format!("efi_uuid = \"{}\"\n", uuids.efi));
    machine.push_str(&format!("root_uuid = \"{}\"\n", uuids.root_fs));
    if let Some(luks) = &uuids.root_luks {
        machine.push_str(&format!("luks_uuid = \"{luks}\"\n"));
    }
    if let Some(home) = &choices.home_disk {
        machine.push_str(&format!("home_disk = \"{home}\"\n"));
    }
    if choices.swap_mib > 0 {
        machine.push_str(&format!("swap = \"{}\"\n", layout::mib(choices.swap_mib)));
    }
    if choices.hibernate {
        machine.push_str("hibernate = true\n");
    }
    machine.push_str(&format!(
        "timezone = \"{}\"\nkeymap = \"{}\"\n",
        account.timezone, account.keymap
    ));
    format!(
        "# Settings for {host} only. bunny reads this on top of ../bunny.toml\n\
         # when it runs on {host}. Written by bunny install.\n\
         \n\
         # What this computer needs to start. Leave these in unless you know you\n\
         # don't need one.\n\
         packages = {packages}\n\
         {services}\
         \n\
         [pkg.\"sys-kernel/installkernel\"]\n\
         use = [\"dracut\", \"grub\"]\n\
         \n\
         # How bunny install set up this computer. A record only: bunny never\n\
         # changes your disks from it.\n\
         [machine]\n\
         {machine}",
        host = account.hostname,
        packages = list(&boot_packages(choices)),
        services = if wifi {
            format!(
                "\n# Wi-Fi: this computer has it, so iwd starts at boot. `bunny wifi` or\n\
                 # `iwctl` connects.\nservices = {}\n",
                list(&host_services(true))
            )
        } else {
            String::new()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Host};

    fn account() -> Account {
        Account {
            username: "you".into(),
            hostname: "laptop".into(),
            ssh: true,
            timezone: "America/Toronto".into(),
            keymap: "us".into(),
        }
    }

    #[test]
    fn wifi_starts_on_a_computer_that_has_it_only() {
        let choices = Choices::default();
        let uuids = Uuids::default();
        let with = host_file_with(&choices, &account(), &uuids, true);
        let host: Host = toml::from_str(&with).unwrap_or_else(|e| panic!("{e}\n{with}"));
        assert_eq!(host.services, ["iwd"], "{with}");
        let without = host_file_with(&choices, &account(), &uuids, false);
        let host: Host = toml::from_str(&without).unwrap();
        assert!(host.services.is_empty(), "{without}");
        // The main config, shared by every computer, never starts it.
        assert!(!config(&account()).contains("\"iwd\",\n]"));
    }

    #[test]
    fn finds_a_wifi_card_in_sys() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("sys/class/net/enp3s0")).unwrap();
        assert!(!has_wifi(root.path()));
        std::fs::create_dir_all(root.path().join("sys/class/net/wlan0/wireless")).unwrap();
        assert!(has_wifi(root.path()));
    }

    #[test]
    fn the_starter_config_is_valid_and_complete() {
        let text = config(&account());
        let c = Config::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        assert_eq!(c.packages, [&BASE_PACKAGES[..], &OWN_PACKAGES[..]].concat());
        assert_eq!(c.repositories, ["bunny"]);
        assert!(c.packages.contains(&"sys-apps/bunny-release".to_string()));
        assert_eq!(
            c.services,
            ["dhcpcd", "sysklogd", "sshd", "local", "netmount"]
        );
        assert_eq!(c.security.unwrap().admins, ["you"]);
        assert!(c.boot.undo_entry);
        assert!(text.contains("hosts/laptop.toml"));
        assert!(text.contains(GUIDE));
    }

    /// The starter config with every example uncommented.
    fn with_examples_on(text: &str) -> String {
        let mut out = text.to_string();
        for line in example_lines() {
            let commented = format!("# {line}\n");
            assert_eq!(out.matches(&commented).count(), 1, "{line:?} once\n{text}");
            out = out.replace(&commented, &format!("{line}\n"));
        }
        out
    }

    #[test]
    fn the_examples_are_commented_out_and_point_to_the_guide() {
        let text = config(&account());
        let c = Config::parse(&text).unwrap();
        assert!(!c.packages.iter().any(|p| p == "app-misc/tmux"));
        assert!(!c.services.iter().any(|s| s == "chronyd"));
        assert!(c.pkg.is_empty());
        for page in [
            "tasks/install.html",
            "reference/config.html#services",
            "tasks/use-flags.html",
        ] {
            assert!(text.contains(&format!("# More: {GUIDE}{page}")), "{page}");
        }
    }

    #[test]
    fn uncommented_examples_pass_bunnys_check() {
        let text = with_examples_on(&config(&account()));
        // What `bunny check` does: read the file, then look for mistakes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bunny.toml");
        std::fs::write(&path, &text).unwrap();
        let c = Config::load(&path).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert!(c.problems().is_empty(), "{:?}\n{text}", c.problems());
        assert!(c.packages.contains(&"app-misc/tmux".to_string()));
        assert!(c.packages.contains(&"net-misc/chrony".to_string()));
        assert!(c.services.contains(&"chronyd".to_string()));
        let tmux = &c.pkg["app-misc/tmux"];
        assert_eq!(tmux.use_flags, ["vim-syntax"]);
        assert_eq!(tmux.keywords, ["~amd64"]);
        // The sections after them still belong where they did (the TOML
        // section trap).
        assert_eq!(c.security.unwrap().admins, ["you"]);
        assert!(c.boot.undo_entry);

        // And a plan against a fake root takes them in.
        let root = tempfile::tempdir().unwrap();
        let world = root.path().join("var/lib/portage/world");
        std::fs::create_dir_all(world.parent().unwrap()).unwrap();
        std::fs::write(&world, "net-misc/dhcpcd\n").unwrap();
        let c = Config::parse(&text).unwrap();
        let plan = crate::plan::make(&c, root.path()).unwrap();
        assert!(plan.add_packages.iter().any(|p| p == "app-misc/tmux"));
        assert!(
            plan.use_changes.iter().any(|u| u.atom == "app-misc/tmux"),
            "{:?}",
            plan.use_changes
        );
    }

    #[test]
    fn bunny_add_keeps_the_examples() {
        let text = config(&account());
        let edited = crate::edit::add_packages(&text, &["app-editors/vim".to_string()]).unwrap();
        let after = Config::parse(&edited.text).unwrap();
        assert!(after.packages.contains(&"app-editors/vim".to_string()));
        for line in example_lines() {
            assert!(edited.text.contains(&format!("# {line}")), "{line} kept");
        }
        // Still uncomments cleanly.
        Config::parse(&with_examples_on(&edited.text)).unwrap();
    }

    #[test]
    fn the_host_file_records_the_disk() {
        let choices = Choices {
            disk: "/dev/nvme0n1".into(),
            swap_mib: 8192,
            hibernate: true,
            ..Default::default()
        };
        let uuids = Uuids {
            efi: "ABCD-1234".into(),
            root_fs: "fs-uuid".into(),
            root_luks: Some("luks-uuid".into()),
            ..Default::default()
        };
        let text = host_file(&choices, &account(), &uuids);
        let host: Host = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert!(host.packages.contains(&"sys-fs/cryptsetup".to_string()));
        assert_eq!(
            host.pkg["sys-kernel/installkernel"].use_flags,
            ["dracut", "grub"]
        );
        let m = host.machine.unwrap();
        assert_eq!(m.disk.as_deref(), Some("/dev/nvme0n1"));
        assert!(m.encrypted && m.hibernate);
        assert_eq!(m.luks_uuid.as_deref(), Some("luks-uuid"));
        assert_eq!(m.timezone.as_deref(), Some("America/Toronto"));

        let plain = Choices {
            disk: "/dev/vda".into(),
            encrypt: false,
            ..Default::default()
        };
        let text = host_file(&plain, &account(), &Uuids::default());
        assert!(
            !text.contains("cryptsetup") && !text.contains("luks_uuid"),
            "{text}"
        );
    }

    #[test]
    fn together_they_cover_everything_the_install_puts_in() {
        let choices = Choices::default();
        let mut c = Config::parse(&config(&account())).unwrap();
        let host: Host =
            toml::from_str(&host_file(&choices, &account(), &Uuids::default())).unwrap();
        c.add_host("laptop", "hosts/laptop.toml".into(), host);
        for atom in super::super::system::packages(&choices) {
            assert!(
                c.packages.iter().any(|p| p == atom),
                "{atom} isn't in the config"
            );
        }
        // Plus Bunny Linux's own, which the first switch installs.
        assert_eq!(
            c.packages.len(),
            super::super::system::packages(&choices).len() + OWN_PACKAGES.len()
        );
    }
}
