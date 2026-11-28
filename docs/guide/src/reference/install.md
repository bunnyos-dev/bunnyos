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
| `--swap <size>` | Also make a swap file on disk of this size, like `8G`, in its own subvolume (`/swap/swapfile`), used after zram. Default: none. |
| `--no-zram` | No compressed swap in memory. By default bunny sets up zram (a `zram` service in the boot runlevel): as big as memory, up to 8 GB, used before any swap file. |
| `--hibernate` | Set up hibernation: a swapfile as big as your memory (unless `--swap` says otherwise), and the kernel settings to resume from it. |
| `--home-disk <disk>` | Use a second disk, all of it, for `/home`. It's erased too, encrypted with the same password, and unlocked at boot with a key file kept on the encrypted system (so you only type the password once). |
| `--ssh` | Start the SSH server at boot. Off by default. |
| `--config <file or repo>` | Install from your own config instead of the starter one: a `bunny.toml` file, or a git repository with `bunny.toml` at its top (`github:you/my-system` is short for `https://github.com/you/my-system`). It's fetched and checked **before** anything is erased. |
| `--mounted <folder>` | Install onto partitions you made and mounted yourself, without erasing anything. See [Installing onto your own partitions](#installing-onto-your-own-partitions). |
| `--edit` | Stop before the config is applied, so you can change it first. Opens `$EDITOR` (or `nano`, or `vi`), checks the config when you close it, and asks you to edit again if something's wrong. |
| `--dry-run` | Go through the questions (not the password, since nothing is encrypted and no account is made), then show each step and what it does, without erasing or changing anything. Doesn't need root. With `--verbose` (`--dry-run --verbose`), every command and file instead, each as a `(would run)` or `(would write)` line. |
| `--recommended` | Take the recommended setup (encrypted, zram swap) without asking. |
| `--user <name>`, `--hostname <name>`, `--timezone <zone>`, `--keymap <layout>` | The account's answers, given instead of asked. Checked the same way; a wrong one stops with the reason. |
| `--password-stdin` | Read the password as one line on standard input instead of asking. Used by the graphical installer. |
| `--yes` | Ask nothing (needs `--disk`): no typing the disk's name, the recommended answer for anything not given, UTC and `us` unless given, and `--user`, `--hostname` and the password (`--password-stdin`, except on a dry run) must be given. |
| `--list-disks` | Print the disks as JSON (name, path, a description, what's on them, and why one can't be used) and stop. Used by the graphical installer. |

| `--verbose` | On a real install, show everything the tools print as they run (it's kept in the log either way). With `--dry-run`, see above. |

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
3. **The recommended setup, or your own.** "Use the recommended setup?
   [Y/n]": Enter takes it: the disk encrypted, and swap compressed in
   memory (zram, as big as the computer's memory up to 8 GB, like Fedora),
   with no swap file on disk. `n` asks, each with the recommended answer on
   Enter: encrypt ([Y/n]), then swap: Enter for zram; a size (zram plus a
   swap file on disk, used after zram); `hibernate` (zram plus a swap file
   as big as memory, and resuming from it); or `none`. An option answers
   its question, which isn't asked: `--no-encrypt`; `--swap`,
   `--hibernate` or `--no-zram`. Given both, nothing is asked. `/home` on
   a second disk isn't asked: use `--home-disk`.
4. **The preview.** Everything on the disk that will be erased (each
   partition, its size, what it holds, like "NTFS (Windows)"), then exactly
   what you'll get, and which other disks aren't touched. If any answer
   differs from the recommended setup, the options that repeat it follow
   (like `bunny install --no-encrypt --swap 8G`). The disk isn't in them,
   since it's named differently on each computer.
5. **Confirming.** You type the disk's name (like `nvme0n1`). Anything else
   cancels, and nothing is changed.
6. **Your account.** A user name, a name for the computer, the time zone
   (like `America/Toronto`; Enter for UTC), the keyboard layout (like `us`,
   `uk` or `de`; Enter for `us`), and a password (typed twice, not shown;
   it can't be empty). Under 8 characters gets a warning that it's easy to
   guess, then the second question as usual: type it again to keep it, or
   press Enter to choose another. With encryption, the same password unlocks the
   disk.
7. **The disk.** The old partition table is cleared, and a new one written:
   the EFI partition and one for Bunny Linux. The second is encrypted
   (LUKS2) and opened as `bunny-root`, formatted as btrfs, and the
   subvolumes are created and mounted at `/mnt/bunny`.
8. **The base system.** Gentoo's newest OpenRC base system (stage3) is
   downloaded, its SHA512 checksum is checked, and it's unpacked.
9. **Settings.** bunny writes `/etc/fstab`, Portage settings (prebuilt
   packages with signatures checked), the startup image's settings (btrfs,
   encryption, hibernation), the sudo file, and copies itself in.
10. **Inside the new system** (chroot): the package list (Gentoo's
   announcements so far are marked read, since they're older than the
   system), language and time zone, the packages (plus a few of the base
   system's own, swapped for prebuilt ones with Gentoo's usual settings:
   the base system is built with some features off, see below), services (`dhcpcd`, `sysklogd`, and `sshd` with
   `--ssh`), the boot menu (GRUB, named "Bunny Linux", also at the EFI
   fallback path), the startup image, and your account.
11. **Your config.** The starter `bunny.toml` (or yours) goes in
    `/etc/bunny/`, with this computer's host file. With `--edit`, you edit
    it now.
12. **The first switch.** `bunny switch` and `bunny lock` run inside the new
    system, so it starts already matching its config: `bunny status` shows ✓.
    If the switch fails, the install still finishes (the system starts
    either way) and bunny tells you to run `sudo bunny switch` after
    restarting.
13. **Your config's history.** `/etc/bunny` becomes a git repository
    (`git init -b main`) with its own name and email for commits: your user
    name and `<user>@<computer name>.local` (a placeholder, only written on
    commits). A `.gitignore` keeps out the `/home` disk's key (`keys/`) and
    leftover copies, then everything else is committed as "As installed".
    A config from a git repository (`--config github:…`) keeps that
    repository's history instead. If this fails, the install still
    finishes; `bunny publish` starts it later.
14. **Done.** Everything is unmounted and the disk locked again. Restart,
    take out the USB stick, type your password to unlock the disk, and log in.

**What you see.** Each step prints its `==> ` heading, then `✓ Done` (with
the time, when it took 2 seconds or more). What the tools themselves print
(wipefs, mkfs, btrfs, cryptsetup, tar, grub-mkconfig, git) isn't shown: it
goes to `/var/log/bunny/<time>-install.log` on the USB, which is copied to
the same place in the new system at the end. Three things keep the screen,
because their output is the progress: the base system's download bar, the
part inside the new system (its own `==> ` headings and emerge's short
lines), and the first `bunny switch`. If a tool fails, bunny shows `✗`, the
last 12 lines it printed, and where the rest is, then says what it means.
`--verbose` shows everything as it runs, and still keeps the log.

**Why a few base packages are replaced.** Gentoo builds its base system
(stage3) with some USE flags turned off that its normal settings turn on:
curl without HTTP/2 and HTTP/3, and libb2, gettext and portage-utils without
OpenMP (the compiler it starts from can't do that yet). Left alone, the
first `bunny switch` would show them as "(rebuild)". The install updates
them from Gentoo's prebuilt packages instead, so the first switch has
nothing to rebuild.

**When something compiles anyway.** Everything the install itself puts in
comes prebuilt from Gentoo's binary host; only Bunny Linux's own packages
(`bunny-release`) are built on your computer, during the first switch. The
exception is a package Gentoo marked stable in the last few hours: Gentoo
builds the prebuilt copy after the package list already has the new
version, so an install in that window compiles that one package (zlib 1.4's
prebuilt copies appeared on November 23, 2026, the same day an install
compiled it). Nothing is wrong when this happens, it only takes longer; an
install a day later gets the prebuilt one.

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
| `/etc/default/grub` | Adds `GRUB_DISTRIBUTOR="Bunny Linux"` and the kernel settings (`rd.luks.uuid=…`, `rootflags=subvol=@`, and `resume=…` with `--hibernate`). Comments out Gentoo's `GRUB_THEME`. |
| `/etc/grub.d/10_bunny` | Names the main boot menu entry "Bunny Linux" (not "Bunny Linux GNU/Linux"), with "Advanced options" under it. `10_linux` and `31_efi_bootnext` ("(EFI BootNext)" entries) are made non-executable; the originals are in `/var/lib/bunny/backups/as-installed/`. See [the boot menu](generations.md#undo-from-the-boot-menu). |
| `/etc/init.d/zram` | Unless `--no-zram`: compressed swap in memory, started at boot. |
| `/etc/init.d/swapfiles` | Only with a swapfile: turns it on once `/swap` is mounted (OpenRC's own swap service starts too early for a swapfile on its own subvolume). |
| `/etc/conf.d/dmcrypt`, `/etc/bunny/keys/home.key` | Only with `--home-disk` and encryption: unlocks the `/home` disk at boot. The key is readable only by root. |

| `/etc/bunny/bunny.toml` | Your config. |
| `/etc/bunny/.git`, `/etc/bunny/.gitignore` | The config's git history (first commit "As installed"), and what's kept out of it. |
| `/etc/gitconfig` | Adds `/etc/bunny` to git's `safe.directory`, so you can read its history (`git log`) as yourself, although the folder belongs to root. |
| `/etc/bunny/hosts/<computer name>.toml` | What belongs to this computer only. |
| `/etc/localtime`, `/etc/timezone`, `/etc/conf.d/keymaps` | Your time zone (the link in `/etc/localtime` is what's used) and keyboard layout. |
| `/usr/local/share/consolefonts/bunny.psfu`, `/etc/conf.d/consolefont` | A console font that can draw bunny's marks (✓ ✗ ⚠), set by OpenRC's `consolefont` service at boot. It's kbd's `eurlatgr`, with ⚠ drawn as `!` (no console font has a ⚠). Only the text console uses it; desktops have their own fonts. |

Passwords are never written to a file: they're passed straight to
`cryptsetup` and `chpasswd`.

## Your config

The new system's config is split in two, so the main file works on any
computer:

- **`/etc/bunny/bunny.toml`** is everything you choose: your programs,
  services, who can use sudo, and the boot menu's Undo entry. The starter
  one lists the minimal system's networking, log, sudo and git, plus
  `sys-apps/bunny-release` from Bunny Linux's own repository
  (`repositories = ["bunny"]`), which makes the system say Bunny Linux
  instead of Gentoo. It has comments pointing to this Guide, and a few
  commented-out examples (`app-misc/tmux` and `net-misc/chrony` in
  `packages`, `chronyd` in `services`, and a `[pkg."app-misc/tmux"]` entry
  with `use` and `keywords`): remove the `#` to use one. Put it in git and
  use it on every computer.
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
