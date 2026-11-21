//! `--screenshots DIR`: draws every screen with example answers to PNG
//! files, with Slint's software renderer and no window or display. For the
//! Guide, for reviews, and to look at the screens anywhere.

use std::rc::Rc;

use anyhow::{Context, Result};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

use crate::{DiskItem, Installer, StepItem};

/// One screenshot: the page, the file name, and what to set first.
type Shot = (i32, &'static str, fn(&Installer));

const WIDTH: u32 = 1024;
const HEIGHT: u32 = 768;

struct Offscreen {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

pub fn draw_all(dir: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    slint::platform::set_platform(Box::new(Offscreen {
        window: window.clone(),
    }))
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let ui = Installer::new()?;
    example(&ui);
    ui.show()?;
    let shots: &[Shot] = &[
        (0, "1-welcome", |_| {}),
        (1, "2-disk", |_| {}),
        (2, "3-setup", |ui| ui.set_recommended(true)),
        (2, "3b-setup-choose", |ui| {
            ui.set_recommended(false);
            ui.set_swap_mode(1);
        }),
        (3, "4-account", |ui| ui.set_recommended(true)),
        (4, "5-password", |ui| {
            ui.set_password("bunny".into());
            ui.set_password_again("bun".into());
        }),
        (5, "6-review", |ui| ui.set_confirm_name("nvme0n1".into())),
        (6, "7-installing", |_| {}),
        (7, "8-done", |ui| ui.set_steps(steps(6, 6))),
    ];
    for (page, name, setup) in shots {
        ui.set_page(*page);
        setup(&ui);
        slint::platform::update_timers_and_animations();
        let mut pixels = vec![PremultipliedRgbaColor::default(); (WIDTH * HEIGHT) as usize];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, WIDTH as usize);
        });
        let path = format!("{dir}/{name}.png");
        save(&path, &pixels).with_context(|| format!("couldn't write {path}"))?;
        println!("{path}");
    }
    Ok(())
}

/// Example answers, the same as `--demo` shows.
fn example(ui: &Installer) {
    ui.set_demo(true);
    ui.set_disks(ModelRc::new(VecModel::from(vec![
        DiskItem {
            name: "nvme0n1".into(),
            describe: "nvme0n1  1.0 TB  Samsung 990 (NVMe)".into(),
            contents: "nvme0n1p1  105 MB  FAT \"EFI\", nvme0n1p2  999 GB  NTFS (Windows)".into(),
            usable: true,
            why: "".into(),
        },
        DiskItem {
            name: "sda".into(),
            describe: "sda  2.0 TB  WD Blue (SATA)".into(),
            contents: "nothing (it's empty)".into(),
            usable: true,
            why: "".into(),
        },
        DiskItem {
            name: "sdb".into(),
            describe: "sdb  16 GB  USB stick".into(),
            contents: "".into(),
            usable: false,
            why: "it's the USB stick you started from".into(),
        },
    ])));
    ui.set_user("you".into());
    ui.set_hostname("laptop".into());
    ui.set_timezone("America/Toronto".into());
    ui.set_preview(
        "This ERASES everything on nvme0n1  1.0 TB  Samsung 990 (NVMe):\n  \
         - nvme0n1p1  105 MB  FAT \"EFI\"\n  - nvme0n1p2  999 GB  NTFS (Windows)\n\n\
         What you'll get:\n  /dev/nvme0n1p1  2 GB  EFI partition, mounted at /boot\n  \
         /dev/nvme0n1p2  the rest  encrypted (LUKS2)\n     btrfs, compressed, with subvolumes\n     \
         Swap: compressed, in memory (zram), as big as your memory up to 8 GB\n  \
         A minimal system: OpenRC, a kernel, networking, your account with sudo, and bunny.\n  \
         Not touched: sda.\n  A starter config in /etc/bunny/bunny.toml, applied before you restart."
            .into(),
    );
    ui.set_steps(steps(6, 3));
    ui.set_log(
        "==> Partitioning /dev/nvme0n1\n==> Creating the EFI partition's filesystem\n\
         ==> Encrypting the disk (LUKS2)\n==> Downloading Gentoo's base system\n"
            .into(),
    );
}

/// The install's first `count` steps, `done` of them finished.
fn steps(count: usize, done: usize) -> ModelRc<StepItem> {
    let titles = [
        "Partitioning /dev/nvme0n1",
        "Creating the EFI partition's filesystem",
        "Encrypting the disk (LUKS2)",
        "Downloading Gentoo's base system",
        "Installing packages",
        "Setting up the boot menu",
    ];
    ModelRc::new(VecModel::from(
        titles
            .iter()
            .take(count)
            .enumerate()
            .map(|(i, t)| StepItem {
                title: (*t).into(),
                done: i < done,
            })
            .collect::<Vec<_>>(),
    ))
}

fn save(path: &str, pixels: &[PremultipliedRgbaColor]) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let data: Vec<u8> = pixels
        .iter()
        .flat_map(|p| [p.red, p.green, p.blue, 255])
        .collect();
    writer.write_image_data(&data)?;
    Ok(())
}
