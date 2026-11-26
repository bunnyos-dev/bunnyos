# Undo a change

Every `bunny switch`, `add`, `remove` and `update` saves your system first.
These saved versions are called **generations**.

## See them

```sh
bunny rollback list
```

```
Saved generations (oldest first):

   1. Nov 16, 9:24 PM   before adding tmux and cronie, starting cronie
      bunny-20261116-212417
   2. Nov 17, 9:15 AM   before adding mpv
      bunny-20261117-091500

Running now: your system after the switch at Nov 17, 9:15 AM.
```

Times are in your computer's time zone, the same as in the boot menu
(the year is added for anything from an earlier year). The name under each
one, like `bunny-20261117-091500`, is the moment in UTC.

Each one says what came **right after** it. So "before adding mpv" is
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
went back to, and so does this computer's host file (`hosts/<computer
name>.toml`). If you'd just run `bunny add htop` and then undid it, htop is gone
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

and run `bunny switch`. (New installs have it on already.) From then on, the
boot menu has two of bunny's entries, right under Bunny Linux's own:

```
Bunny Linux
Advanced options
Undo the last change (+ mangowm, + dejavu, Nov 18, 7:03 PM)
Earlier versions (11 saved)
UEFI Firmware Settings
```

The same switch tidies what Gentoo's setup leaves in the menu: its theme
(the menu is plain dark until Bunny Linux has its own look), "GNU/Linux"
after the name, and the "(EFI BootNext)" entries. Other systems on the
computer, like Windows, still show. `bunny plan` lists each of these under
"Boot menu, tidied" first, and undoing that switch puts them back.

**Undo the last change** says what it undoes and when (in your time zone).
If a change leaves the system unable to start, restart, pick it, and the
system starts as it was before the change. It's the same as `bunny
rollback`, done while starting up: the broken system is saved too, so you
can go back to it with `bunny rollback list`.

On an encrypted disk, where `/boot` is its own partition, bunny finishes the
undo early in that first start, putting `/boot`'s kernel back to the one that
version used (the same for **Go back to this version**, below). `bunny
status` says so on its **Kernel** line.

After you've used it, it's gone until your next change, so you can't go
further back by picking it again. (The same happens after `bunny
rollback`.) If it's somehow picked twice for the same change, the second
time does nothing and the computer starts normally.

**Earlier versions** lists the newest 10 saved versions, newest first, like
`Nov 18, 7:03 PM · before adding the repository guru, adding mangowm and dejavu`
(that version is your system just before those were added). Pick one
to see what came after it, one change per line, and **Go back to this
version** to start from it. That's undoable too. (The lines describing the
changes do nothing when picked: GRUB can only show menu entries.) `bunny
clean` removes the ones it deletes.

If the boot menu doesn't show up when you start the computer, hold **Shift**
(or press **Esc**) while it starts.

How it works and what it changes: [Generations and rollback, in
detail](../reference/generations.md#undo-from-the-boot-menu).
