//! `bunny install`: put Bunny Linux on a computer, from the live USB.
//!
//! Part 1, guided: pick a disk, see exactly what will be erased, type the
//! disk's name to confirm, and get the standard layout (encrypted by
//! default) with a minimal system on it: OpenRC, a kernel, networking, one
//! user with sudo, and bunny. Packages come from Gentoo's binary host.
//!
//! Part 2, the config: the new system gets a `bunny.toml` (a starter one from
//! your answers, or yours with `--config`) and `hosts/<host name>.toml`, and
//! its first `bunny switch` and `bunny lock` run before you restart.
//!
//! - `disks`: which disks can be used, and what's on them
//! - `layout`: the partitions, encryption, subvolumes and fstab
//! - `system`: the base system, its settings, and the rest inside it
//! - `starter`: the starter `bunny.toml` and this computer's host file
//! - `first_config`: your config in the new system, and its first switch

pub mod disks;
pub mod first_config;
pub mod layout;
pub mod starter;
pub mod system;

use anyhow::Result;

use crate::apply::Runner;
use layout::Choices;
use system::Account;

/// Does the whole install, in order. On a failure after the disk was
/// touched, everything is unmounted and locked again, so running it again
/// starts cleanly.
pub fn run(
    choices: &Choices,
    account: &Account,
    password: &str,
    config: &first_config::Source,
    edit: bool,
    runner: &mut dyn Runner,
) -> Result<Finished> {
    let home_key = system::home_key_path();
    let result = (|| -> Result<Finished> {
        if choices.home_disk.is_some() && choices.encrypt {
            make_key(&home_key, runner)?;
        }
        layout::prepare(choices, password, &home_key, runner)?;
        let uuids = layout::read_uuids(choices, runner)?;
        let offset = system::resume_offset(choices, runner)?;
        let cmdline = choices.cmdline(&uuids, offset.as_deref());
        system::unpack_stage3(choices, runner)?;
        system::configure(choices, account, &uuids, &cmdline, &home_key, runner)?;
        system::install_inside(choices, account, password, runner)?;
        let config_path = first_config::put_in_place(config, choices, account, &uuids, runner)?;
        if edit {
            // Problems left after editing: the starter settings already in
            // the new system still work, so this doesn't stop the install.
            if let Err(err) = first_config::edit(&config_path, &account.hostname, runner) {
                println!("\n⚠ {err:#}. bunny didn't apply it; fix it after restarting.");
                return Ok(Finished { switched: false });
            }
        }
        let switched = first_config::first_switch(choices, account, &config_path, runner)?;
        Ok(Finished { switched })
    })();
    let _ = system::finish(choices, runner);
    first_config::clean_up(config, runner);
    if !runner.dry() {
        let _ = std::fs::remove_file(&home_key);
    }
    result
}

/// How the install ended.
pub struct Finished {
    /// The first `bunny switch` ran and the lock was written.
    pub switched: bool,
}

/// A random key for the /home disk, readable only by root.
fn make_key(path: &std::path::Path, runner: &dyn Runner) -> Result<()> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    if runner.dry() {
        println!(
            "  (would make) a random key for the /home disk in {}",
            path.display()
        );
        return Ok(());
    }
    let mut key = [0u8; 64];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(path)?;
    use std::io::Write;
    file.write_all(&key)?;
    Ok(())
}
