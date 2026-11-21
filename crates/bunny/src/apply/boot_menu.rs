//! The boot menu's undo entry: the dracut module, the GRUB entry, and the
//! kernel copies kept on a separate `/boot`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{cmd, Options, Outcome, Runner};

/// Installs the dracut module and rebuilds the startup image for each kernel.
/// Each new image is built as a separate file and only swapped in once it's
/// complete; the old one is backed up. If anything fails, the module is
/// removed again and the old images stay.
pub(super) fn install_boot_module(
    root: &Path,
    versions: &[String],
    runner: &mut dyn Runner,
    backup: &Path,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = root.join(crate::boot::MODULE_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    for (name, text) in [
        ("module-setup.sh", crate::boot::MODULE_SETUP),
        ("bunny-rollback.sh", crate::boot::HOOK),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text)
            .with_context(|| format!("Couldn't write {}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    // The start-up service that finishes an undo when /boot is its own
    // partition (the startup image can't reach it). Harmless otherwise.
    crate::init::for_config(crate::config::Init::default(), root)
        .add_boot_service(
            crate::finish_undo::SERVICE,
            crate::finish_undo::SERVICE_SCRIPT,
        )
        .with_context(|| format!("Couldn't add the {} service", crate::finish_undo::SERVICE))?;
    let undo = |why: String| -> anyhow::Error {
        let _ = std::fs::remove_dir_all(&dir);
        anyhow::anyhow!(
            "{why}\n  The startup image wasn't changed, and nothing else was either.\n  \
             To go ahead without the boot menu entry, set `undo_entry = false` under [boot]."
        )
    };

    println!("\nAdding the undo step to the startup image…");
    for version in versions {
        let image = crate::boot::initramfs(root, version);
        let mut staged = image.clone().into_os_string();
        staged.push(".bunny-new");
        let staged = PathBuf::from(staged);
        let (ok, said) = runner.run_quiet(
            &cmd(&["dracut", "--force", &staged.to_string_lossy(), version]),
            &mut |_| {},
        )?;
        if runner.dry() {
            continue;
        }
        if !ok || !staged.is_file() {
            let _ = std::fs::remove_file(&staged);
            return Err(undo(format!(
                "Couldn't rebuild the startup image for kernel {version}. dracut said:\n{}",
                crate::emerge::tail(&said, 10)
            )));
        }
        std::fs::create_dir_all(backup)?;
        if image.exists() {
            std::fs::copy(&image, backup.join(image.file_name().unwrap_or_default()))
                .with_context(|| format!("Couldn't back up {}", image.display()))?;
        }
        std::fs::rename(&staged, &image)
            .with_context(|| format!("Couldn't put the new {} in place", image.display()))?;
    }
    Ok(())
}

/// Keeps a copy of /boot's kernel files for `generation`. Returns a note if
/// that couldn't be done (the switch goes ahead either way).
pub fn save_kernels(root: &Path, generation: &str) -> Option<String> {
    use crate::kernels::Saved;
    let mb = |b: u64| b.div_ceil(1024 * 1024);
    match crate::kernels::save(root, generation) {
        Ok(Saved::Done { .. }) | Ok(Saved::NothingToSave) => None,
        Ok(Saved::NoRoom { needed, free }) => Some(format!(
            "There wasn't room on /boot to keep a copy of the kernel ({} MB needed, {} MB free), \
             so undoing this change can't put the kernel back. `bunny clean` frees space.",
            mb(needed),
            mb(free)
        )),
        Err(e) => Some(format!(
            "Couldn't keep a copy of the kernel on /boot ({e:#}), \
             so undoing this change can't put the kernel back."
        )),
    }
}

/// Points the boot menu's undo entry at this switch's snapshot. Returns a
/// note for the summary.
pub(super) fn write_undo_entry(
    root: &Path,
    opts: &Options,
    outcome: &Outcome,
) -> Result<Option<String>> {
    use crate::boot;
    if outcome.snapshot.is_none() {
        return Ok(None);
    }
    let generation = format!("bunny-{}", opts.stamp);
    let generation_dir = root.join(".snapshots").join(&generation);
    let cmdline = std::fs::read_to_string(root.join("proc/cmdline")).unwrap_or_default();
    let running =
        std::fs::read_to_string(root.join("proc/sys/kernel/osrelease")).unwrap_or_default();
    let not_added = |why: &str| {
        Ok(Some(format!(
            "The boot menu's undo entry wasn't updated: {why}."
        )))
    };

    let when = crate::rollback::Generation {
        name: generation.clone(),
    }
    .when();
    let undo_cmdline = boot::undo_cmdline(&cmdline, &generation);

    let mountinfo = crate::kernels::mountinfo(root);
    let block = if let Some(device) = crate::kernels::boot_device(&mountinfo) {
        // /boot is its own filesystem: boot the copies bunny kept there.
        let Some(saved) = crate::kernels::load(root, &generation) else {
            return not_added("there's no copy of this generation's kernel on /boot");
        };
        let versions = crate::kernels::versions(saved.keys());
        let running = running.trim();
        let Some(version) = versions
            .iter()
            .find(|v| *v == running)
            .or_else(|| versions.last())
        else {
            return not_added("couldn't find a kernel in the saved generation");
        };
        let (Some(kernel), Some(image)) = (
            saved.get(&format!("vmlinuz-{version}")),
            saved.get(&format!("initramfs-{version}.img")),
        ) else {
            return not_added("the saved generation has no startup image for its kernel");
        };
        let store = root.join(crate::kernels::STORE).join("files");
        if !boot::image_has_module(&generation_dir, &store.join(image)) {
            return not_added(
                "the saved generation's startup image doesn't have the undo step yet",
            );
        }
        let Some(boot_uuid) = crate::kernels::device_uuid(root, &device) else {
            return not_added("couldn't find the UUID of the /boot partition");
        };
        let mut initrds: Vec<String> = saved
            .iter()
            .filter(|(name, _)| name.ends_with("-uc.img"))
            .map(|(_, stored)| crate::kernels::grub_path(stored))
            .collect();
        initrds.push(crate::kernels::grub_path(image));
        boot::render_entry_on_boot(
            &when,
            &boot_uuid,
            &crate::kernels::grub_path(kernel),
            &initrds,
            &undo_cmdline,
        )
    } else {
        match bootable_from_snapshot(
            &generation,
            &generation_dir,
            &cmdline,
            running.trim(),
            &when,
        ) {
            Ok(block) => block,
            Err(why) => return not_added(why),
        }
    };
    let path = root.join(boot::CUSTOM_CFG);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, boot::merge_custom_cfg(&text, &block))
        .with_context(|| format!("Couldn't write {}", path.display()))?;
    Ok(Some(
        "If this change stops the system from starting, pick \"Bunny Linux: undo the last change\" \
         in the boot menu (hold Shift or Esc while starting if the menu is hidden)."
            .to_string(),
    ))
}

/// The undo entry when `/boot` is part of the system: the kernel inside the
/// generation's snapshot. Returns why not, if it can't be made.
pub(super) fn bootable_from_snapshot(
    generation: &str,
    generation_dir: &Path,
    cmdline: &str,
    running: &str,
    when: &str,
) -> std::result::Result<String, &'static str> {
    use crate::boot;
    let Some(uuid) = boot::root_uuid(cmdline) else {
        return Err("the system isn't started with root=UUID=…");
    };
    let Some(version) = boot::generation_kernel(generation_dir, running) else {
        return Err("couldn't find a kernel in the saved generation");
    };
    if !boot::generation_has_module(generation_dir, &version) {
        return Err("the saved generation's startup image doesn't have the undo step yet");
    }
    Ok(boot::render_entry(
        generation,
        when,
        &version,
        uuid,
        &boot::undo_cmdline(cmdline, generation),
    ))
}
