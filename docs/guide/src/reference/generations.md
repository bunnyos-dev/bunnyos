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
successful switch, bunny keeps a copy of the config and lock it just applied in
`/var/lib/bunny/applied/`. That folder is part of the system, so every later
generation carries the config that matches it.

When you roll back (with `bunny rollback` or from the boot menu), that copy is
put back where your config lives, as long as that's under `/etc`. A config in
`/home` is on its own subvolume and is left alone.

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
- After each switch, bunny writes one entry to `/boot/grub/custom.cfg` (GRUB
  reads that file at boot; anything else in it is kept). The entry boots
  **the kernel and startup image saved in that generation**, with
  `bunny.rollback=<generation>` added. Using the generation's own kernel means
  everything matches after the swap, even if the change updated the kernel.
- On a normal boot the step does nothing. When the entry is picked, it does
  the same as `bunny rollback` before the system is mounted: saves the current
  system as `bunny-<time>-before-rollback`, moves it aside to
  `@replaced-<time>`, and makes a copy of the generation the new `@`. If any
  step fails, the current system is left (or put back) as it was, and it
  starts normally.
- `/boot/grub` is part of the system, so the restored system brings back its
  own, older `custom.cfg`, whose entry would undo one change further back.
  Both `bunny rollback` and the boot-menu undo remove bunny's entry from the
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
  `/boot/bunny/files/`, finding the partition by its UUID. The undo itself
  runs in the startup image, which can't reach `/boot`, so it leaves a note
  in the restored system (`/var/lib/bunny/boot-undo`, the generation's name).
  Early in that first start, the `bunny-finish-undo` service (in OpenRC's
  `boot` runlevel; bunny adds it together with the undo step) runs `bunny
  finish-undo`, which makes `/boot`'s kernel files exactly what that
  generation had, the same way `bunny rollback` does, rebuilds the boot menu
  with `grub-mkconfig`, removes the undo entry (its file is on `/boot` too)
  and clears the note. `bunny status` then says what it did:

  ```
  ✓ Kernel        /boot has kernel 6.12.58-gentoo-dist again, put back after the undo from the boot menu
                  (it had 6.12.60-gentoo-dist from the version you went back from)
  ```

  That line shows until your next change. If it couldn't be done, the note
  stays, bunny tries again at the next start, and `bunny status` shows a
  **Kernel** warning with the reason; `sudo bunny finish-undo` tries again
  straight away. A note is only acted on while that undo is still the last
  thing that happened (an older note that came back with a snapshot is
  dropped). Generations saved before bunny kept kernels, or before it had
  this service, leave `/boot` as it is; then `bunny status` warns if `/boot`'s
  kernel has no drivers in the restored system, and you can pick the older
  kernel under "Advanced options" in the boot menu.
- **`bunny clean`** removes the lists of generations it deletes, and any
  copies no remaining generation uses.

## Requirements

- `/` on btrfs, mounted from a subvolume called `@`
- `/.snapshots` as its own subvolume

The [VM installer](testing-in-a-vm.md) sets this up. Without `/.snapshots`,
bunny still works but says each time that the change can't be rolled back.
