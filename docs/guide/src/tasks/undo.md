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

Rollback restores the **system**. If your `bunny.toml` lives somewhere outside
the system (like your home folder), it isn't rolled back. Run `bunny plan`
afterwards: it shows any difference between the config and the restored
system, and you can either edit the config or `bunny switch` again. `bunny
diff` shows the same differences side by side, with the edits that would
make the config match.

## If the system doesn't start at all

A boot-menu "Undo last switch" entry is planned. Until then, `bunny rollback`
needs a working system.
