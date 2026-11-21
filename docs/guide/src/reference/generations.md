# Generations and rollback, in detail

## What a generation is

A **read-only btrfs snapshot** of `/`, taken right before bunny changes
anything. It's stored in `/.snapshots/bunny-<date>-<time>`. Snapshots are
cheap: they share all the data that hasn't changed since.

Next to each one, bunny keeps a small note (`/.snapshots/bunny-….toml`)
saying what came right after it: a switch (with a summary like
`+ tmux, start cronie`) or a rollback (and which generation it went to).
`/.snapshots/bunny-state.toml` remembers the last thing bunny did, which is how
`rollback list` knows what you're running.

`/.snapshots` is its own subvolume, so the notes survive rollbacks.

## What rollback does

`bunny rollback N`:

1. mounts the top level of the btrfs filesystem
2. saves the current system as `bunny-<time>-before-rollback` (so the rollback
   can be undone)
3. makes a **writable copy** of generation N, under a temporary name
   (`@restoring-<time>`)
4. moves the current root subvolume `@` aside to `@replaced-<time>`, and
   renames the copy to `@`
5. moves any **nested subvolumes** (subvolumes inside `@`, such as `/srv` on
   some systems) over to the new `@`, so their contents aren't lost

On the next boot, the system starts from the new `@`. `/boot` is inside `@`,
so the kernel goes back too. (If `/boot` is its own partition, bunny puts the
kernel back itself; see [below](#when-boot-is-its-own-partition).)

If the copy can't be made, nothing changes. If the rename to `@` fails, the
old `@` is moved back (and bunny checks that worked; if it didn't, it says
exactly how to put it back before restarting). Ctrl+C is held off during
the two renames. The boot-menu undo works the same way.

While a rollback is waiting for a restart, `switch`, `add`, `remove`,
`update` and another `rollback` refuse to run: their changes would go into
the system that's about to be replaced. `bunny clean` still works.

If the config is a link (say `/etc/bunny/bunny.toml` pointing into
`/home`), it's treated like a config in `/home`: it lives outside the
system, so undo leaves it alone rather than writing through the link.

## The config goes back too

A generation is saved right before a change, but by then `bunny.toml` has
usually changed already (`bunny add` edits it first; so do you). So after each
successful switch, bunny keeps a copy of the config, the lock and this
computer's host file (`hosts/<computer name>.toml`, if there is one) it just
applied in `/var/lib/bunny/applied/`. That folder is part of the system, so every later
generation carries the config that matches it.

When you roll back (with `bunny rollback` or from the boot menu), that copy is
put back where your config lives, as long as that's under `/etc`. A config in
`/home` is on its own subvolume and is left alone. `bunny rollback` says what
it put back:

```
  /etc/bunny/bunny.toml and /etc/bunny/hosts/laptop.toml are put back as they were then,
  so they match the restored system.
```

Only the host file for this computer is kept; files for your other computers
in `hosts/` aren't changed. Generations saved before this version of bunny
have no copy of the host file, so undoing to one of those leaves the host
file as it is.

## What's never rolled back

Anything on its own subvolume (see [The disk layout](disk-layout.md)):
`/home`, `/var/log`, `/var/cache`, `/var/tmp/portage` and `/.snapshots`
itself.

## Leftovers and cleaning

`@replaced-<time>` is the system you rolled back *from*. It stays until you run
[`bunny clean`](../tasks/free-space.md), which deletes nested subvolumes first
(btrfs can't delete a subvolume that still contains another one). It never
deletes the subvolume you're currently running from, even if you haven't
rebooted yet.

## Undo from the boot menu

With `[boot] undo_entry = true`:

- bunny adds a small step to the startup image (a dracut module in
  `/usr/lib/dracut/modules.d/90bunny-rollback/`) and rebuilds the image once.
  The new image is built as a separate file and only swapped in when it's
  complete; the old one is saved in the backup folder. This happens before
  that switch's snapshot, so that very generation can be undone from the menu.
- After each switch, bunny writes its entries to `/boot/grub/bunny.cfg`:
  "Undo the last change", and "Earlier versions" (the newest 10
  generations that can be started this way, each a submenu with "Go back
  to this version" and what came after it). `/etc/grub.d/11_bunny` makes
  GRUB read that file right after the system's own entries; bunny installs
  it and runs `grub-mkconfig` once. GRUB reads `bunny.cfg` each time the
  menu is shown, so later changes need no `grub-mkconfig`. (Older versions
  of bunny wrote the undo entry to `custom.cfg`, at the bottom of the menu;
  the next switch moves it.) The undo entry boots
  **the kernel and startup image saved in that generation**, with
  `bunny.rollback=<generation>` added. Using the generation's own kernel means
  everything matches after the swap, even if the change updated the kernel.
- The same switch tidies the menu (shown in `bunny plan` under "Boot menu,
  tidied", once; again only if something brings them back):
  - **Gentoo's theme:** a `GRUB_THEME=` line in `/etc/default/grub` is
    commented out, with a note above it saying so.
  - **"Bunny Linux GNU/Linux":** GRUB's `/etc/grub.d/10_linux` always adds
    " GNU/Linux" to `GRUB_DISTRIBUTOR`. bunny adds `/etc/grub.d/10_bunny`,
    which runs `10_linux` and changes only the titles: "Bunny Linux", its
    submenu "Advanced options", and "Bunny Linux, with Linux 6.12.58" inside
    it. Everything else (kernels, startup images, microcode, settings) is
    still `10_linux`'s own work. `10_linux` is made non-executable, so
    `grub-mkconfig` doesn't list everything twice. If a GRUB update makes it
    executable again, `10_bunny` prints nothing (the menu says "GNU/Linux"
    again, but nothing is listed twice) and the next switch tidies it.
  - **"(EFI BootNext)" entries:** `/etc/grub.d/31_efi_bootnext` is made
    non-executable. "UEFI Firmware Settings" (`30_uefi-firmware`) and other
    systems (`30_os-prober`) stay.

  Each file is copied to the switch's backup folder
  (`/var/lib/bunny/backups/<time>/etc/…`) first, then `grub-mkconfig` runs
  once. The files are in `/etc`, so undoing the switch puts them back. With
  `/boot` on its own partition, the menu itself keeps the new titles until
  the next `grub-mkconfig` (a kernel update or switch). Turning `undo_entry`
  off leaves the tidied menu as it is.
- On a normal boot the step does nothing. When the entry is picked, it does
  the same as `bunny rollback` before the system is mounted: saves the current
  system as `bunny-<time>-before-rollback`, moves it aside to
  `@replaced-<time>`, and makes a copy of the generation the new `@`. If any
  step fails, the current system is left (or put back) as it was, and it
  starts normally.
- `/boot/grub` is part of the system, so the restored system brings back its
  own, older `bunny.cfg`, whose entry would undo one change further back.
  Both `bunny rollback` and the boot-menu undo remove the undo entry (Earlier
  versions stays, since it has its own markers) from the
  restored system (other entries in the file stay). The next switch adds a
  fresh one.
- If `/.snapshots/bunny-state.toml` says the last thing was a rollback to the
  same generation the entry points at, the step does nothing and the system
  starts normally. That covers picking the entry twice, or GRUB remembering
  the last choice.

Limits:

- Nested subvolumes (see above) aren't carried over by the boot-menu undo.
  `bunny rollback` from a running system does carry them.
- The entry needs GRUB, dracut, and the system started with `root=UUID=…`.
  `bunny plan` says if something's missing; everything else still works.
  (Run as yourself on a system whose `/boot` only root can read, `bunny plan`
  can't look, so it says "Boot menu: not checked" and to run it with sudo
  instead of guessing.)

## When `/boot` is its own partition

On an encrypted system, `/boot` is usually the EFI partition, outside `@`
(the boot menu can't read the encrypted disk). Snapshots don't include it, so
on their own they'd keep the newest kernel, whose drivers the restored system
might not have. bunny makes up for that:

- **Each generation keeps its kernel on `/boot`.** Right after a
  generation's snapshot, bunny copies `/boot`'s kernel files (`vmlinuz-*`,
  `initramfs-*.img`, `System.map-*`, `config-*` and microcode images) into
  `/boot/bunny/files/`, and lists which ones that generation had in
  `/boot/bunny/generations/<generation>`. A file that's already there with
  the same contents isn't copied again, so generations that share a kernel
  cost nothing extra. A rebuilt startup image with the same name is kept as
  `<name>~2`, `~3` and so on.
- **There must be room.** bunny leaves 64 MB free on `/boot` for the next
  kernel update. If a copy wouldn't fit, the switch goes ahead and says that
  undoing it can't put the kernel back. `bunny clean` frees space.
- **`bunny rollback` puts the kernel back.** Before changing anything, it
  keeps the current kernel files (for the `before-rollback` generation, so
  the rollback can be undone completely). Then, after the swap, it makes
  `/boot`'s kernel files exactly what that generation had, copying everything
  in before removing anything, and rebuilds the boot menu with
  `grub-mkconfig`. Its question says which kernel you'll get, e.g.
  `The kernel goes back too: 6.12.60 → 6.12.58`. If the kernel copy can't be
  made, nothing is changed.
- **Older generations** (saved before bunny kept kernels) can still be rolled
  back to; `/boot` just keeps its current kernel, and bunny says so.
- **The boot-menu undo** boots the generation's kernel straight from
  `/boot/bunny/files/`, finding the partition by its UUID. It can't change
  the kernel files on `/boot` itself, so after it the normal menu entry may
  start a kernel the restored system has no drivers for. `bunny status` then
  shows a **Kernel** warning; until that's fixed, pick the older kernel under
  "Advanced options" in the boot menu. (Putting `/boot` back automatically
  after a boot-menu undo is planned.)
  The undo entry also stays in the menu (its file is on `/boot` too), but
  picking it again does nothing, since that change is already undone.
  `bunny rollback` does remove it.
- **`bunny clean`** removes the lists of generations it deletes, and any
  copies no remaining generation uses.

## Requirements

- `/` on btrfs, mounted from a subvolume called `@`
- `/.snapshots` as its own subvolume

The [VM installer](testing-in-a-vm.md) sets this up. Without `/.snapshots`,
bunny still works but says each time that the change can't be rolled back.
