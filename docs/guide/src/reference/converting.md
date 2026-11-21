# Moving an existing install to the Bunny Linux layout

If you installed Gentoo yourself on btrfs, `/` is probably the top of the
btrfs volume, with no `@` subvolume. bunny works there, but **undo doesn't**:
it needs [the Bunny Linux layout](disk-layout.md). `tools/convert-layout.sh`
moves a running system over without reinstalling.

> Try it in the [laptop-shaped test VM](testing-in-a-vm.md#a-second-vm-shaped-like-a-hand-made-install)
> first, and back up `/home` before running it on a real computer.

## What it does

| Becomes | Mounted at | From |
|---|---|---|
| `@` | `/` | a btrfs copy of the whole system as it is now |
| `@home` | `/home` | a copy of `/home` |
| `@log` | `/var/log` | a copy of `/var/log` |
| `@cache` | `/var/cache` | a copy of `/var/cache` |
| `@build` | `/var/tmp/portage` | empty (compile scratch space) |
| `@snapshots` | `/.snapshots` | empty (saved versions go here) |
| `@swap` | `/swap` | a new swapfile, the same size as your old one (only if you have one) |

The copies are **reflinks**: they take seconds and almost no extra space,
however big `/home` is. **Nothing is moved or deleted**, and the old system
stays exactly where it was.

It also:

- copies subvolumes that were inside `/` (like `/srv`) into `@` as normal
  folders, since nested subvolumes aren't part of saved versions. A
  subvolume at one of the folders above (like `/home` as a subvolume) is
  copied into its new `@` subvolume instead, and its old fstab line is kept
  as a comment;
- leaves alone any of those folders that's already its own filesystem (say
  `/home` on another disk, or `/var/tmp/portage` in memory as tmpfs): no
  subvolume is made for it, and its fstab line stays;
- moves swap out of `/`, because btrfs won't save a version of a subvolume
  with an active swapfile in it, and adds a small boot service, `swapfiles`,
  that turns that swap on once `/swap` is mounted (OpenRC's own swap
  service runs too early to see it);
- writes `@`'s `/etc/fstab` for the new layout, keeping every other line
  (`/boot`, network shares) and saving the old one as
  `/etc/fstab.before-bunny-layout`;
- adds `rootflags=subvol=@` to GRUB and runs `grub-mkconfig`;
- adds a boot menu entry, **"Before the Bunny Linux layout"**, that starts the
  old, untouched system.

**`/boot` has to be its own partition** (usually the EFI partition, as on
an encrypted install). Then the boot menu and kernels sit outside the btrfs
volume and keep working as they are. If `/boot` is a folder inside `/`, the
script stops before changing anything: GRUB would have to be reinstalled
too, which it doesn't do yet. Encrypted disks are fine; nothing about the
encryption changes.

## Do it

See the plan first. It changes nothing:

```sh
doas sh tools/convert-layout.sh
```

Then:

```sh
doas sh tools/convert-layout.sh --apply
```

It asks you to type `convert`. Then restart. It starts from `@` by itself.
Check:

```sh
findmnt -no OPTIONS /
```

That includes `subvol=/@`. If anything is wrong, restart, hold **Esc**, and
pick **Before the Bunny Linux layout**.

## Free the space afterwards

Once you're happy with the new layout, delete the old copy:

```sh
doas sh tools/convert-layout.sh --finish
```

It lists what it will delete and asks you to type `delete`. After that,
"Before the Bunny Linux layout" can't start anymore and is removed from the
boot menu.

## If it stops partway

It says so, and says which new subvolumes to delete before trying again. The
old system is untouched either way.

## The kernel and undo

With `/boot` on its own partition, saved versions don't include the kernel.
bunny keeps a copy of each version's kernel on `/boot` instead, so undo puts
the kernel back too (see
[Generations](generations.md#when-boot-is-its-own-partition)).
