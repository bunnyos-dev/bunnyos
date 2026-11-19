//! `bunny install`: put Bunny Linux on a computer, from the live USB.
//!
//! Part 1, guided: pick a disk, see exactly what will be erased, type the
//! disk's name to confirm, and get the standard layout (encrypted by
//! default) with a minimal system on it: OpenRC, a kernel, networking, one
//! user with sudo, and bunny. Packages come from Gentoo's binary host.
//!
//! - `disks`: which disks can be used, and what's on them
//! - `layout`: the partitions, encryption, subvolumes and fstab
//! - `system`: the base system, its settings, and the rest inside it

pub mod disks;
pub mod layout;
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
    runner: &mut dyn Runner,
) -> Result<()> {
    let home_key = system::home_key_path();
    let result = (|| -> Result<()> {
        if choices.home_disk.is_some() && choices.encrypt {
            make_key(&home_key, runner)?;
        }
        layout::prepare(choices, password, &home_key, runner)?;
        let uuids = layout::read_uuids(choices, runner)?;
        let offset = system::resume_offset(choices, runner)?;
        let cmdline = choices.cmdline(&uuids, offset.as_deref());
        system::unpack_stage3(choices, runner)?;
        system::configure(choices, account, &uuids, &cmdline, &home_key, runner)?;
        system::install_inside(choices, account, password, runner)
    })();
    let _ = system::finish(choices, runner);
    if !runner.dry() {
        let _ = std::fs::remove_file(&home_key);
    }
    result
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
