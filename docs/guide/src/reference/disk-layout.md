# The disk layout

bunny's undo needs btrfs, laid out like this:

```
Disk (GPT)
├─ EFI partition   2 GB   FAT32    mounted at /efi
└─ Bunny Linux         rest   btrfs
   ├─ @            /                   ← snapshotted as generations
   ├─ @home        /home               ← never rolled back
   ├─ @log         /var/log            ← never rolled back
   ├─ @cache       /var/cache          ← downloads and packages
   ├─ @build       /var/tmp/portage    ← compile scratch space
   └─ @snapshots   /.snapshots         ← generations live here
```

## Why it's split this way

- **Rollback restores the system, not your stuff.** `/home`, logs and caches
  are separate subvolumes, so going back to yesterday's system doesn't take
  your files or logs with it.
- **Builds don't bloat snapshots.** Compiling a browser writes gigabytes to
  `/var/tmp/portage`. Keeping it separate keeps generations small.
- **The EFI partition is 2 GB**, more than the usual 512 MB, to leave room for
  several kernels later (boot-menu rollback and a gaming kernel are planned).

## Mount options

```
compress=zstd:1,noatime
```

zstd compression usually saves a good share of disk space at almost no CPU
cost.

## Planned

Encryption (LUKS2, unlocked at a pretty boot screen) is planned as the
default for the Bunny Linux installer. The VM installer doesn't encrypt yet.

## Setting it up

The [VM installer](testing-in-a-vm.md) creates exactly this layout. To move
an existing install over, see
[Moving an existing install to the layout](converting.md).
