# Every command

`[config]` defaults to `/etc/bunny/bunny.toml`. Commands that change the
system ask for your password by themselves, through sudo or doas
(whichever your system uses; see [`[security]`](config.md#security)).
`--root <folder>` (where offered) points bunny at another system's files: it
reads and writes there and only *prints* commands instead of running them.
That's useful for testing.

## Looking (never change anything)

| Command | What it does |
|---|---|
| `bunny status [config]` | One screen saying whether everything is in order, and what to do if not. |
| `bunny diff [config]` | Your config and your system side by side, and how to fix each difference either way. Exits 1 if they differ. |
| `bunny check [config]` | Finds mistakes in the config and says how to fix each. |
| `bunny plan [config]` | Shows what a switch would change: packages, USE flags, services, packages that don't exist, and differences from `bunny.lock`. |
| `bunny search <words>` | Searches package names and descriptions. Marks installed ones. |
| `bunny rollback list` / `bunny generations` | Lists saved generations, numbered, with what each was saved before. |
| `bunny explain [file]` | Explains an emerge message. Without a file, paste it and press Ctrl+D. |
| `bunny import -o <file>` | Writes a config describing the system as it is now. Add `--force` to overwrite. |

## Changing (show the plan and ask first)

| Command | What it does |
|---|---|
| `bunny switch [config]` | Makes the system match the config. Keeps packages at their locked versions. |
| `bunny add <names> [--config file]` | Adds packages to the config, then switches. |
| `bunny remove <names> [--config file]` | Removes packages from the config, then switches. |
| `bunny update [config]` | Syncs the package tree, updates everything, records new versions in the lock. |
| `bunny lock [config]` | Accepts the system as it is now: writes `bunny.lock` from the installed versions. |
| `bunny rollback [number]` | Puts the system back to a generation (the last switch if no number). Takes effect after a reboot. |
| `bunny clean [--keep N]` | Removes old generations and rollback leftovers. Keeps the newest 5 by default. |

`switch`, `add`, `remove` and `update` take `-y` / `--yes` to skip bunny's
question and emerge's install question. **Removing packages from the system
always asks**, even with `--yes`.

## How questions work

- **`[Y/n]`**: Enter means yes. Used when the change can be undone (a snapshot
  is taken first).
- **`[y/N]`**: Enter means no. Used when it can't be undone (`clean`, or a
  switch on a system without `/.snapshots`).
- Anything other than yes or no is asked again rather than guessed.
