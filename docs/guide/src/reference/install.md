# bunny install, in detail

`bunny install` puts Bunny Linux on a computer: it erases a disk, makes
[the Bunny Linux disk layout](disk-layout.md) on it, and installs a
minimal system. It runs from a live USB.

> **Planned:** the Bunny Linux USB image, and a step-by-step "Install Bunny
> Linux" chapter. Until then, `bunny install` runs from Gentoo's minimal
> install USB, with the `bunny` file copied onto it.

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
- No desktop. Adding one with bunny is the first thing to do afterwards.

Time zone is UTC and the language is US English for now; asking for them is
planned.

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
| `--dry-run` | Show everything it would do (each command, each file), without erasing or changing anything. Doesn't need root. |

Sizes are written like `512M`, `2G` or `1.5G`.

## Step by step

1. **Checks.** It must run as root, and the USB must have started in UEFI
   mode (bunny says how to fix that if not).
2. **The disk.** bunny lists the disks with their size and model. A disk
   is left out, with the reason, if it's in use: anything on it mounted (the
   USB stick you started from always is), used as swap, or unlocked. Disks
   under 20 GB and read-only ones are left out too.
3. **The preview.** Everything on the disk that will be erased (each
   partition, its size, what it holds, like "NTFS (Windows)"), then exactly
   what you'll get.
4. **Confirming.** You type the disk's name (like `nvme0n1`). Anything else
   cancels, and nothing is changed.
5. **Your account.** A user name, a name for the computer, and a password
   (at least 8 characters, typed twice, not shown). With encryption, the same
   password unlocks the disk.
6. **The disk.** The old partition table is cleared, and a new one written:
   the EFI partition and one for Bunny Linux. The second is encrypted
   (LUKS2) and opened as `bunny-root`, formatted as btrfs, and the
   subvolumes are created and mounted at `/mnt/bunny`.
7. **The base system.** Gentoo's newest OpenRC base system (stage3) is
   downloaded, its SHA512 checksum is checked, and it's unpacked.
8. **Settings.** bunny writes `/etc/fstab`, Portage settings (prebuilt
   packages with signatures checked), the startup image's settings (btrfs,
   encryption, hibernation), the sudo file, and copies itself in.
9. **Inside the new system** (chroot): the package list, language and time
   zone, the packages, services (`dhcpcd`, `sysklogd`, and `sshd` with
   `--ssh`), the boot menu (GRUB, named "Bunny Linux", also at the EFI
   fallback path), the startup image, and your account.
10. **Done.** Everything is unmounted and the disk locked again. Restart,
    take out the USB stick, type your password to unlock the disk, and log in.

If a step fails, bunny says which one and why, unmounts everything and locks
the disk again. The disk is only partly set up at that point; run
`bunny install` again and it starts over from the beginning.

## The files it writes

| File | What it is |
|---|---|
| `/etc/fstab` | Where each subvolume, the EFI partition and the swapfile are mounted. |
| `/etc/portage/make.conf` | Adds `FEATURES="getbinpkg binpkg-request-signature"`, the licences of prebuilt packages, and `GRUB_PLATFORMS="efi-64"`. |
| `/etc/portage/package.use/bunny-install` | The kernel installer uses dracut and GRUB; sudo is built without a mail server. |
| `/etc/dracut.conf.d/bunny.conf` | The startup image can mount btrfs, unlock the disk and resume from hibernation. |
| `/etc/sudoers.d/bunny` | Admin rights for the `wheel` group, the same file [`[security]`](config.md#security) manages. |
| `/etc/default/grub` | Adds `GRUB_DISTRIBUTOR="Bunny Linux"` and the kernel settings (`rd.luks.uuid=…`, `rootflags=subvol=@`, and `resume=…` with `--hibernate`). |
| `/etc/conf.d/dmcrypt`, `/etc/bunny/keys/home.key` | Only with `--home-disk` and encryption: unlocks the `/home` disk at boot. The key is readable only by root. |

Passwords are never written to a file: they're passed straight to
`cryptsetup` and `chpasswd`.

## Planned

- Asking for the time zone, keyboard and language.
- Writing a starter `bunny.toml` for the new system, and turning on undo from
  the boot menu.
- Installing alongside another system, and installing onto partitions you
  made yourself (`--mounted`).
