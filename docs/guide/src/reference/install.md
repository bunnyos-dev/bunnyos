# bunny install, in detail

`bunny install` puts Bunny Linux on a computer: it erases a disk, makes
[the Bunny Linux disk layout](disk-layout.md) on it, and installs a
minimal system. It runs from a live USB.

> Step by step, for a first install: [Install Bunny Linux](../start/installing.md).
> This page has every option and exactly what happens. `bunny install` runs
> from the Bunny Linux live USB (it also works from Gentoo's minimal install
> USB, with the `bunny` file copied onto it).

## What you get

- The disk layout: a 2 GB EFI partition mounted at `/boot`, and the rest
  encrypted (LUKS2) with btrfs subvolumes for the system, your files, logs,
  caches, build space and generations.
- A minimal system: OpenRC, Gentoo's prebuilt kernel (`gentoo-kernel-bin`)
  and firmware, GRUB, networking (`dhcpcd` for Ethernet, `iwd` installed for
  Wi-Fi), a system log, `sudo`, `git`, and bunny itself in
  `/usr/local/bin/bunny`.
- Your account, in the `wheel` group, with admin rights through sudo. The
  `root` account has no password (use sudo).
- Your time zone and keyboard layout. The language is US English for now.
- Your config, already applied: `/etc/bunny/bunny.toml` (a starter one, or
  yours with `--config`) and `/etc/bunny/hosts/<computer name>.toml` (see
  [Your config](#your-config) below), with its `bunny.lock`. The first
  generation is the system as installed, and the boot menu has an "Undo"
  entry after every change.
- No desktop. Adding one with bunny is the first thing to do afterwards.

## Options

| Option | What it does |
|---|---|
| `--disk <disk>` | The disk to erase, like `/dev/nvme0n1` or `nvme0n1`. Without it, bunny lists the disks and asks. |
| `--no-encrypt` | Don't encrypt. Anyone with the computer can then read the disk. |
| `--efi-size <size>` | Size of the EFI partition. Default `2G`; at least `512M`. |
| `--swap <size>` | Make a swapfile of this size, like `8G`, in its own subvolume (`/swap/swapfile`). Default: no swapfile. |
| `--hibernate` | Set up hibernation: a swapfile as big as your memory (unless `--swap` says otherwise), and the kernel settings to resume from it. |
| `--home-disk <disk>` | Use a second disk, all of it, for `/home`. It's erased too, encrypted with the same password, and unlocked at boot with a key file kept on the encrypted system (so you only type the password once). |
| `--ssh` | Start the SSH server at boot. Off by default. |
| `--config <file or repo>` | Install from your own config instead of the starter one: a `bunny.toml` file, or a git repository with `bunny.toml` at its top (`github:you/my-system` is short for `https://github.com/you/my-system`). It's fetched and checked **before** anything is erased. |
| `--mounted <folder>` | Install onto partitions you made and mounted yourself, without erasing anything. See [Installing onto your own partitions](#installing-onto-your-own-partitions). |
| `--edit` | Stop before the config is applied, so you can change it first. Opens `$EDITOR` (or `nano`, or `vi`), checks the config when you close it, and asks you to edit again if something's wrong. |
| `--dry-run` | Go through the questions, then show each step and what it does, without erasing or changing anything. Doesn't need root. With `--verbose` (`--dry-run --verbose`), every command and file instead, each as a `(would run)` or `(would write)` line. |

Sizes are written like `512M`, `2G` or `1.5G`.

## Step by step

1. **Checks.** It must run as root, and the USB must have started in UEFI
   mode (bunny says how to fix that if not). With `--config`, your config
   is fetched and checked now; if it has a mistake, bunny says what and
   stops, before touching any disk.
2. **The disk.** bunny lists the disks with their size and model. A disk
   is left out, with the reason, if it's in use: anything on it mounted (the
   USB stick you started from always is), used as swap, or unlocked. Disks
   under 20 GB and read-only ones are left out too.
3. **The preview.** Everything on the disk that will be erased (each
   partition, its size, what it holds, like "NTFS (Windows)"), then exactly
   what you'll get.
4. **Confirming.** You type the disk's name (like `nvme0n1`). Anything else
   cancels, and nothing is changed.
5. **Your account.** A user name, a name for the computer, the time zone
   (like `America/Toronto`; Enter for UTC), the keyboard layout (like `us`,
   `uk` or `de`; Enter for `us`), and a password (typed twice, not shown;
   it can't be empty). Under 8 characters gets a warning that it's easy to
   guess, then the second question as usual: type it again to keep it, or
   press Enter to choose another. With encryption, the same password unlocks the
   disk.
6. **The disk.** The old partition table is cleared, and a new one written:
   the EFI partition and one for Bunny Linux. The second is encrypted
   (LUKS2) and opened as `bunny-root`, formatted as btrfs, and the
   subvolumes are created and mounted at `/mnt/bunny`.
7. **The base system.** Gentoo's newest OpenRC base system (stage3) is
   downloaded, its SHA512 checksum is checked, and it's unpacked.
8. **Settings.** bunny writes `/etc/fstab`, Portage settings (prebuilt
   packages with signatures checked), the startup image's settings (btrfs,
   encryption, hibernation), the sudo file, and copies itself in.
9. **Inside the new system** (chroot): the package list (Gentoo's
   announcements so far are marked read, since they're older than the
   system), language and time zone, the packages, services (`dhcpcd`, `sysklogd`, and `sshd` with
   `--ssh`), the boot menu (GRUB, named "Bunny Linux", also at the EFI
   fallback path), the startup image, and your account.
10. **Your config.** The starter `bunny.toml` (or yours) goes in
    `/etc/bunny/`, with this computer's host file. With `--edit`, you edit
    it now.
11. **The first switch.** `bunny switch` and `bunny lock` run inside the new
    system, so it starts already matching its config: `bunny status` shows ✓.
    If the switch fails, the install still finishes (the system starts
    either way) and bunny tells you to run `sudo bunny switch` after
    restarting.
12. **Your config's history.** `/etc/bunny` becomes a git repository
    (`git init -b main`) with its own name and email for commits: your user
    name and `<user>@<computer name>.local` (a placeholder, only written on
    commits). A `.gitignore` keeps out the `/home` disk's key (`keys/`) and
    leftover copies, then everything else is committed as "As installed".
    A config from a git repository (`--config github:…`) keeps that
    repository's history instead. If this fails, the install still
    finishes; `bunny publish` starts it later.
13. **Done.** Everything is unmounted and the disk locked again. Restart,
    take out the USB stick, type your password to unlock the disk, and log in.

If a step fails, bunny says which one and why, unmounts everything and locks
the disk again. The disk is only partly set up at that point; run
`bunny install` again and it starts over from the beginning.

On a **preview USB** (built for testing), the install is recorded, and just
before unmounting it copies the recording into your home folder in the new
system ("Keeping the recording of this install"), so `bunny report` can send
it after you restart. See
[Developer mode](developer-mode.md#recording-an-install-preview-usbs).

## The files it writes

| File | What it is |
|---|---|
| `/etc/fstab` | Where each subvolume, the EFI partition and the swapfile are mounted. |
| `/etc/portage/make.conf` | Adds `FEATURES="getbinpkg binpkg-request-signature"`, the licences of prebuilt packages, and `GRUB_PLATFORMS="efi-64"`. |
| `/etc/portage/package.use/bunny-install` | The kernel installer uses dracut and GRUB; sudo is built without a mail server. |
| `/etc/dracut.conf.d/bunny.conf` | The startup image can mount btrfs, unlock the disk and resume from hibernation. |
| `/etc/sudoers.d/bunny` | Admin rights for the `wheel` group, the same file [`[security]`](config.md#security) manages. |
| `/etc/default/grub` | Adds `GRUB_DISTRIBUTOR="Bunny Linux"` and the kernel settings (`rd.luks.uuid=…`, `rootflags=subvol=@`, and `resume=…` with `--hibernate`). |
| `/etc/init.d/swapfiles` | Only with a swapfile: turns it on once `/swap` is mounted (OpenRC's own swap service starts too early for a swapfile on its own subvolume). |
| `/etc/conf.d/dmcrypt`, `/etc/bunny/keys/home.key` | Only with `--home-disk` and encryption: unlocks the `/home` disk at boot. The key is readable only by root. |

| `/etc/bunny/bunny.toml` | Your config. |
| `/etc/bunny/.git`, `/etc/bunny/.gitignore` | The config's git history (first commit "As installed"), and what's kept out of it. |
| `/etc/bunny/hosts/<computer name>.toml` | What belongs to this computer only. |
| `/etc/localtime`, `/etc/timezone`, `/etc/conf.d/keymaps` | Your time zone (the link in `/etc/localtime` is what's used) and keyboard layout. |

Passwords are never written to a file: they're passed straight to
`cryptsetup` and `chpasswd`.

## Your config

The new system's config is split in two, so the main file works on any
computer:

- **`/etc/bunny/bunny.toml`** is everything you choose: your programs,
  services, who can use sudo, and the boot menu's Undo entry. The starter
  one lists the minimal system's networking, log, sudo and git, and has
  comments pointing to this Guide. Put it in git and use it on every
  computer.
- **`/etc/bunny/hosts/<computer name>.toml`** is what this computer needs to
  start (the kernel, firmware, GRUB, the filesystem tools, and `cryptsetup`
  when the disk is encrypted), plus a `[machine]` record of how its disk was
  set up. See [host files](config.md#host-files).

With `--config` and a repository that already has
`hosts/<computer name>.toml`, yours is kept, and the new record is written
next to it as `hosts/<computer name>.toml.from-install`.

The first switch only runs if the config (with the host file) lists
everything the computer needs to start. If yours doesn't, bunny says which
packages are missing and skips the switch, so nothing is removed: add them
to the host file, then run `sudo bunny switch` after restarting.

## Installing onto your own partitions

If you'd rather partition and format the disk yourself (or keep another
system on it), mount everything first, then point bunny at it:

```sh
mount /dev/nvme0n1p5 /mnt/gentoo
mount --mkdir /dev/nvme0n1p1 /mnt/gentoo/boot
bunny install --mounted /mnt/gentoo
```

bunny doesn't partition, format or erase anything. It reads what's mounted,
shows it, explains anything that limits it, and asks before installing. Then
it installs the same minimal system, and writes `/etc/fstab` and the kernel
settings from what's mounted. At the end, your partitions stay mounted.

What it needs:

- **Something mounted at the folder, and empty** (the folders other
  partitions are mounted on, and `lost+found`, are fine).
- **The EFI partition mounted at `<folder>/boot`.** Bunny Linux keeps the boot
  menu and kernels there.
- **A Linux filesystem for the system:** btrfs, ext4, xfs or f2fs.

What it explains:

| What's mounted | What you get |
|---|---|
| btrfs, the system in a subvolume named `@`, and an `@snapshots` subvolume at `/.snapshots` | Everything, including undo. This is [the Bunny Linux layout](disk-layout.md). |
| btrfs with `@`, but no `/.snapshots` | No undo until you add one. |
| btrfs mounted without subvolumes, still empty | bunny offers to make the standard subvolumes first (`@`, `@home`, `@log`, `@cache`, `@build`, `@snapshots`) and mounts everything again the Bunny Linux way, so you get undo. |
| btrfs with the system in a subvolume with another name | No undo (it needs `@`). |
| ext4, xfs or f2fs | No undo (it needs btrfs). Everything else works, and bunny keeps backups of the files it changes. |
| `/home` not its own subvolume or partition | Undo would also put your own files back. bunny suggests an `@home` subvolume. |
| Encrypted partitions (LUKS) | Each is unlocked when the computer starts, asking for its password. |

These layouts are allowed and checked, but only the standard one is tested
in every release. `--mounted` can't be combined with the options that
describe a disk (`--disk`, `--no-encrypt`, `--efi-size`, `--swap`,
`--hibernate`, `--home-disk`); a swapfile is yours to set up.

## Planned

- Choosing the language.
- Installing alongside another system on the same disk (today, use
  `--mounted`).
