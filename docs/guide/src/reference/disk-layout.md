# The disk layout

bunny's undo needs btrfs, laid out like this. [`bunny install`](install.md)
makes it:

```
Disk (GPT)
├─ EFI partition   2 GB   FAT32    mounted at /boot (boot menu and kernels)
└─ Bunny Linux     rest   encrypted (LUKS2), then btrfs
   ├─ @            /                   ← snapshotted as generations
   ├─ @home        /home               ← never rolled back
   ├─ @log         /var/log            ← never rolled back
   ├─ @cache       /var/cache          ← downloads and packages
   ├─ @build       /var/tmp/portage    ← compile scratch space
   ├─ @snapshots   /.snapshots         ← generations live here
   └─ @swap        /swap               ← only with a swapfile
```

## Why it's split this way

- **Rollback restores the system, not your stuff.** `/home`, logs and caches
  are separate subvolumes, so going back to yesterday's system doesn't take
  your files or logs with it.
- **Builds don't bloat snapshots.** Compiling a browser writes gigabytes to
  `/var/tmp/portage`. Keeping it separate keeps generations small.
- **The EFI partition is `/boot`**, so the boot menu can start the kernel
  before the disk is unlocked. Since `/boot` isn't inside `@`, bunny keeps a
  copy of each generation's kernel there, so undo still puts the kernel
  back (see [Generations](generations.md#when-boot-is-its-own-partition)).
- **The EFI partition is 2 GB**, more than the usual 512 MB, to leave room for
  those kernel copies (and a gaming kernel later).
- **Swap is a file in its own subvolume** (`@swap`), because btrfs can't
  snapshot a subvolume with an active swapfile in it.

## Mount options

```
compress=zstd:1,noatime
```

zstd compression usually saves a good share of disk space at almost no CPU
cost.

## Encryption

Encryption is on by default: you type your password when the computer
starts (as plain text for now; a styled unlock screen is planned).
`bunny install --no-encrypt` leaves it off.

## Setting it up

[`bunny install`](install.md) creates this layout on a new computer. The
[VM installer](testing-in-a-vm.md) makes a simpler version for testing: no
encryption, and the EFI partition at `/efi` with `/boot` inside `@`. To move
an existing install over, see
[Moving an existing install to the layout](converting.md).
