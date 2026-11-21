# Undo a change

Every `bunny switch`, `add`, `remove` and `update` saves your system first.
These saved versions are called **generations**.

## See them

```sh
bunny rollback list
```

```
Saved generations (oldest first):

   1. 2026-11-16 21:24:17 UTC   before a switch: + tmux, + cronie, start cronie
      bunny-20261116-212417
   2. 2026-11-17 09:15:00 UTC   before a switch: + mpv
      bunny-20261117-091500

Running now: your system after the switch at 2026-11-17 09:15:00 UTC.
```

Each one says what came **right after** it. So "before a switch: + mpv" is
your system as it was just before mpv was added.

## Go back

```sh
bunny rollback 2
```

Then restart:

```sh
sudo reboot
```

After the restart you're running the system exactly as it was at that point:
programs, settings, kernel.

**What's not touched:** your files in `/home`, logs, and download caches.
Undo is for the system, not your documents.

## Changed your mind?

Rollback saves the system before it, too, so you can go back again. After
rolling back, bunny tells you the number to use:

```
Changed your mind? After rebooting, run: bunny rollback 3
```

## Your config file after a rollback

Your `bunny.toml` goes back too, to the version that matched the system you
went back to. If you'd just run `bunny add htop` and then undid it, htop is gone
from the system **and** from `bunny.toml`, so `bunny status` is all ✓ right away.

This works for a config in `/etc` (the default, `/etc/bunny/bunny.toml`). A
config kept somewhere else, like your home folder, isn't touched. Run `bunny
diff` afterwards to see where it and the restored system differ, with the edits
that would make them match.

It also only works for generations saved after the switch that turned this on
(bunny keeps a copy of each config it applies, starting from that switch).

## If the system doesn't start at all

Turn on the boot menu's undo entry once, in `bunny.toml`:

```toml
[boot]
undo_entry = true
```

and run `bunny switch`. From then on, every change adds an entry to the boot
menu:

```
Bunny Linux: undo the last change (back to 2026-11-17 09:15:00 UTC)
```

If a change leaves the system unable to start, restart, pick that entry, and
the system starts as it was before the change. It's the same as `bunny
rollback`, done while starting up: the broken system is saved too, so you can
go back to it with `bunny rollback list`.

On an encrypted disk, where `/boot` is its own partition, bunny finishes the
undo early in that first start, putting `/boot`'s kernel back to the one that
version used. `bunny status` says so on its **Kernel** line.

After you've used it, the entry is gone until your next change, so you
can't go further back by picking it again. (The same happens after `bunny
rollback`.) To move between saved versions after that, use `bunny rollback
list`. If the entry is somehow picked twice for the same change, the second
time does nothing and the computer starts normally.

If the boot menu doesn't show up when you start the computer, hold **Shift**
(or press **Esc**) while it starts.

How it works and what it changes: [Generations and rollback, in
detail](../reference/generations.md#undo-from-the-boot-menu).
