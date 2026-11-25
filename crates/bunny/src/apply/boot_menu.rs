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

/// How to start `generation` from the boot menu, or why it can't be.
fn bootable(
    root: &Path,
    generation: &str,
    cmdline: &str,
    running: &str,
    boot_device: Option<&str>,
) -> std::result::Result<crate::boot::Bootable, &'static str> {
    use crate::boot;
    let generation_dir = root.join(".snapshots").join(generation);
    let undo_cmdline = boot::undo_cmdline(cmdline, generation);
    let Some(device) = boot_device else {
        // /boot is part of the system: the kernel inside the snapshot.
        let Some(uuid) = boot::root_uuid(cmdline) else {
            return Err("the system isn't started with root=UUID=…");
        };
        let Some(version) = boot::generation_kernel(&generation_dir, running) else {
            return Err("couldn't find a kernel in the saved generation");
        };
        if !boot::generation_has_module(&generation_dir, &version) {
            return Err("the saved generation's startup image doesn't have the undo step yet");
        }
        return Ok(boot::Bootable::in_snapshot(
            generation,
            &version,
            uuid,
            &undo_cmdline,
        ));
    };
    // /boot is its own filesystem: boot the copies bunny kept there.
    let Some(saved) = crate::kernels::load(root, generation) else {
        return Err("there's no copy of this generation's kernel on /boot");
    };
    let versions = crate::kernels::versions(saved.keys());
    let Some(version) = versions
        .iter()
        .find(|v| *v == running)
        .or_else(|| versions.last())
    else {
        return Err("couldn't find a kernel in the saved generation");
    };
    let (Some(kernel), Some(image)) = (
        saved.get(&format!("vmlinuz-{version}")),
        saved.get(&format!("initramfs-{version}.img")),
    ) else {
        return Err("the saved generation has no startup image for its kernel");
    };
    let store = root.join(crate::kernels::STORE).join("files");
    if !boot::image_has_module(&generation_dir, &store.join(image)) {
        return Err("the saved generation's startup image doesn't have the undo step yet");
    }
    let Some(boot_uuid) = crate::kernels::device_uuid(root, device) else {
        return Err("couldn't find the UUID of the /boot partition");
    };
    let mut initrds: Vec<String> = saved
        .iter()
        .filter(|(name, _)| name.ends_with("-uc.img"))
        .map(|(_, stored)| crate::kernels::grub_path(stored))
        .collect();
    initrds.push(crate::kernels::grub_path(image));
    Ok(boot::Bootable::on_boot(
        &boot_uuid,
        &crate::kernels::grub_path(kernel),
        &initrds,
        &undo_cmdline,
    ))
}

/// What came right after a generation, for its menu entry.
fn summary(root: &Path, generation: &crate::rollback::Generation) -> String {
    use crate::notes::Kind;
    match crate::notes::read_note(root, &generation.name) {
        Some(note) if note.kind == Kind::Switch => note.summary,
        Some(note) => note
            .to
            .map(|to| {
                format!(
                    "going back to {}",
                    crate::rollback::Generation { name: to }.when()
                )
            })
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// Everything the boot menu needs to know about this computer.
struct MenuContext {
    cmdline: String,
    running: String,
    boot_device: Option<String>,
}

impl MenuContext {
    fn read(root: &Path) -> MenuContext {
        MenuContext {
            cmdline: std::fs::read_to_string(root.join("proc/cmdline")).unwrap_or_default(),
            running: std::fs::read_to_string(root.join("proc/sys/kernel/osrelease"))
                .unwrap_or_default()
                .trim()
                .to_string(),
            boot_device: crate::kernels::boot_device(&crate::kernels::mountinfo(root)),
        }
    }

    fn version(
        &self,
        root: &Path,
        generation: &crate::rollback::Generation,
    ) -> std::result::Result<crate::boot::MenuVersion, &'static str> {
        Ok(crate::boot::MenuVersion {
            when: generation.when(),
            summary: summary(root, generation),
            bootable: bootable(
                root,
                &generation.name,
                &self.cmdline,
                &self.running,
                self.boot_device.as_deref(),
            )?,
        })
    }
}

/// The Earlier versions submenu: the newest generations that can be started
/// from the boot menu, newest first.
fn earlier_block(root: &Path, ctx: &MenuContext) -> String {
    let all = crate::rollback::list(root).unwrap_or_default();
    let versions: Vec<_> = all
        .iter()
        .rev()
        .filter_map(|g| ctx.version(root, g).ok())
        .take(crate::boot::EARLIER_SHOWN)
        .collect();
    crate::boot::render_earlier(&versions, all.len())
}

/// Rebuilds `/boot/grub/grub.cfg`. Returns a note if that failed.
pub(super) fn rebuild_menu(root: &Path, runner: &mut dyn Runner) -> Result<Option<String>> {
    let cfg = root.join("boot/grub/grub.cfg");
    let (ok, _) = runner.run_quiet(
        &cmd(&["grub-mkconfig", "-o", &cfg.to_string_lossy()]),
        &mut |_| {},
    )?;
    if ok || runner.dry() {
        Ok(None)
    } else {
        Ok(Some(format!(
            "The boot menu wasn't rebuilt with its new entries. \
             Run `{} grub-mkconfig -o /boot/grub/grub.cfg` to fix it.",
            crate::security::admin()
        )))
    }
}

/// Makes sure GRUB reads bunny.cfg right after the system's own entries:
/// installs `/etc/grub.d/11_bunny` and runs grub-mkconfig once (or again,
/// when `rebuild` says the menu's settings changed). Returns a note if that
/// couldn't be done.
fn install_menu_script(
    root: &Path,
    runner: &mut dyn Runner,
    rebuild: bool,
) -> Result<Option<String>> {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join(crate::boot::GRUBD_SCRIPT);
    if std::fs::read_to_string(&script).ok().as_deref() != Some(crate::boot::GRUBD_TEXT) {
        if let Some(dir) = script.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&script, crate::boot::GRUBD_TEXT)
            .with_context(|| format!("Couldn't write {}", script.display()))?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    } else if !rebuild {
        return Ok(None);
    }
    rebuild_menu(root, runner)
}

/// Tidies Gentoo's leftovers in the boot menu (its theme, "GNU/Linux",
/// "(EFI BootNext)"), backing up each file into this switch's backup folder.
/// Returns whether anything changed (so grub-mkconfig needs to run).
pub(super) fn tidy_menu(
    root: &Path,
    plan: &crate::plan::Plan,
    outcome: &mut Outcome,
) -> Result<bool> {
    if plan.boot.tidy.is_empty() {
        return Ok(false);
    }
    crate::boot::tidy::apply(root, &plan.boot.tidy, &outcome.backup_dir)
        .context("Couldn't tidy the boot menu")?;
    outcome.notes.push(format!(
        "Tidied the boot menu. The old files are in {}.",
        outcome.backup_dir.display()
    ));
    Ok(true)
}

/// After a switch: the boot menu's "Undo the last change" (pointing at this
/// switch's snapshot) and "Earlier versions". Returns notes for the summary.
pub(super) fn write_menu(
    root: &Path,
    opts: &Options,
    outcome: &Outcome,
    runner: &mut dyn Runner,
    tidied: bool,
) -> Result<Vec<String>> {
    use crate::boot;
    if outcome.snapshot.is_none() {
        if tidied {
            return Ok(rebuild_menu(root, runner)?.into_iter().collect());
        }
        return Ok(Vec::new());
    }
    let mut notes = Vec::new();
    let ctx = MenuContext::read(root);
    let newest = crate::rollback::Generation {
        name: format!("bunny-{}", opts.stamp),
    };
    let undo = match ctx.version(root, &newest) {
        Ok(v) => boot::render_undo(&v),
        Err(why) => {
            notes.push(format!("The boot menu's undo entry wasn't updated: {why}."));
            String::new()
        }
    };
    let path = root.join(boot::BUNNY_CFG);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let text = boot::merge_custom_cfg(&text, &undo);
    let text = boot::merge_earlier(&text, &earlier_block(root, &ctx));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, text).with_context(|| format!("Couldn't write {}", path.display()))?;
    // Older versions of bunny put the undo entry in custom.cfg, at the
    // bottom of the menu: it's in bunny.cfg now.
    let custom = root.join(boot::CUSTOM_CFG);
    if let Ok(old) = std::fs::read_to_string(&custom) {
        let without = boot::merge_custom_cfg(&old, "");
        if without != old {
            std::fs::write(&custom, without)
                .with_context(|| format!("Couldn't write {}", custom.display()))?;
        }
    }
    notes.extend(install_menu_script(root, runner, tidied)?);
    if !undo.is_empty() {
        notes.push(
            "If this change stops the system from starting, pick \"Undo the last change\" \
             in the boot menu (hold Shift or Esc while starting if the menu is hidden)."
                .to_string(),
        );
    }
    Ok(notes)
}

/// Rewrites the Earlier versions submenu (after `bunny clean` deleted some
/// generations). Leaves the undo entry alone. Does nothing if bunny hasn't
/// written a boot menu.
pub fn refresh_earlier(root: &Path) -> Result<()> {
    let path = root.join(crate::boot::BUNNY_CFG);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let ctx = MenuContext::read(root);
    let updated = crate::boot::merge_earlier(&text, &earlier_block(root, &ctx));
    if updated != text {
        std::fs::write(&path, updated)
            .with_context(|| format!("Couldn't write {}", path.display()))?;
    }
    Ok(())
}
