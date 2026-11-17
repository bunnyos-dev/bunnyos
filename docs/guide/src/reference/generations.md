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
3. moves the current root subvolume `@` aside to `@replaced-<time>`
4. makes a **writable copy** of generation N the new `@`
5. moves any **nested subvolumes** (subvolumes inside `@`, such as `/srv` on
   some systems) over to the new `@`, so their contents aren't lost

On the next boot, the system starts from the new `@`. `/boot` is inside `@`,
so the kernel goes back too.

If step 4 fails, the old `@` is moved back and nothing changes.

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

Limits:

- Nested subvolumes (see above) aren't carried over by the boot-menu undo.
  `bunny rollback` from a running system does carry them.
- The entry needs GRUB, dracut, and the system started with `root=UUID=…`.
  `bunny plan` says if something's missing; everything else still works.

## Requirements

- `/` on btrfs, mounted from a subvolume called `@`
- `/.snapshots` as its own subvolume

The [VM installer](testing-in-a-vm.md) sets this up. Without `/.snapshots`,
bunny still works but says each time that the change can't be rolled back.
