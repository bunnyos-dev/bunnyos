//! The config `bunny install` writes into the new system: a starter
//! `bunny.toml` that works on any computer, and `hosts/<host name>.toml`
//! with what belongs to this one (what it needs to start, and a record of
//! how its disk was set up).

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
pub const BASE_PACKAGES: [&str; 5] = [
    "net-misc/dhcpcd",
    "net-wireless/iwd",
    "app-admin/sysklogd",
    "app-admin/sudo",
    "dev-vcs/git",
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

fn list(items: &[&str]) -> String {
    let mut out = String::from("[\n");
    for item in items {
        out.push_str(&format!("    \"{item}\",\n"));
    }
    out.push(']');
    out
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
packages = {packages}

# Package repositories besides Gentoo's own. \"bunny\" is Bunny Linux's own
# (bunny-release, which makes the system say Bunny Linux).
repositories = [\"{own}\"]

# Services that start when the computer does.
services = {services}

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
        packages = list(&[&BASE_PACKAGES[..], &OWN_PACKAGES[..]].concat()),
        own = crate::overlays::OWN,
        services = list(&services(account)),
        user = account.username,
    )
}

/// `hosts/<host name>.toml`.
pub fn host_file(choices: &Choices, account: &Account, uuids: &Uuids) -> String {
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
    fn the_starter_config_is_valid_and_complete() {
        let text = config(&account());
        let c = Config::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        assert_eq!(c.packages, [&BASE_PACKAGES[..], &OWN_PACKAGES[..]].concat());
        assert_eq!(c.repositories, ["bunny"]);
        assert!(c.packages.contains(&"sys-apps/bunny-release".to_string()));
        assert_eq!(c.services, ["dhcpcd", "sysklogd", "sshd"]);
        assert_eq!(c.security.unwrap().admins, ["you"]);
        assert!(c.boot.undo_entry);
        assert!(text.contains("hosts/laptop.toml"));
        assert!(text.contains(GUIDE));
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
