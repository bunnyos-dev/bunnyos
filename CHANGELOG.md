# Changelog

Every change people would notice, newest first. Each pull request adds its
line under **Unreleased**; a release moves those lines under its version
(see `docs/RELEASING.md`).

The format follows [Keep a Changelog](https://keepachangelog.com), and
versions follow [Semantic Versioning](https://semver.org) (while it's 0.x,
a minor version can change how things work).

## [Unreleased]

- The reminder to only add repositories you trust is now marked `·`
  instead of ⚠, so with developer mode's `report = "auto"` a switch that
  adds a repository and works isn't sent as a problem.
- New: `repositories` in `bunny.toml` for package repositories besides
  Gentoo's own (like GURU). bunny looks them up in Gentoo's list, sets them
  up and downloads them before installing, so a package from one installs in
  the same switch.
- New: `keywords` under `[pkg]` allows testing versions (`["~amd64"]`).
  emerge's "only available as a testing version" is now explained with the
  lines to add.
- `bunny add` knows where a few packages that aren't in Gentoo's own tree
  went (Mango, Hyprland), and offers to add their repository and testing
  versions.
- New: `bunny install` puts Bunny Linux on a computer from a live USB. It
  lists the disks (leaving out ones in use), shows exactly what will be
  erased, asks you to type the disk's name, and installs a minimal encrypted
  system with the Bunny Linux layout. Options for no encryption, a swapfile,
  hibernation, a second disk for `/home`, and SSH; `--dry-run` shows every
  step without changing anything.
- `bunny install` asks for your time zone and keyboard layout, writes a
  starter `bunny.toml` for the new system, and applies it before you restart,
  so the system starts matching its config (with the boot menu's Undo entry
  on). `--config` installs from your own config (a file, or a git repository
  like `github:you/my-system`), checked before anything is erased; `--edit`
  lets you change it first.
- `bunny install --mounted <folder>` installs onto partitions you made and
  mounted yourself, without erasing anything. It explains what limits your
  layout (like no undo on ext4) and can make the standard btrfs subvolumes
  on an empty filesystem.
- New: the Bunny Linux live USB image. `tools/build-iso.sh` builds it; it
  starts to a terminal with a welcome that explains `bunny install`, links
  the Guide and says how to get online.
- Guide: a new "Install Bunny Linux" chapter (the USB, `bunny install` step
  by step, first start), "What to do first" (git, a desktop) and "Helping
  test" (developer mode).
- New: host files. `hosts/<computer name>.toml` next to `bunny.toml` adds
  packages, services and USE flags for one computer only, so the main config
  works everywhere. `bunny install` writes one with what the computer needs
  to start and a record of its disk.
- Undo now puts the kernel back on systems where `/boot` is its own partition
  (usually encrypted ones). Each generation keeps a copy of its kernel on
  `/boot`, shared when nothing changed; `bunny rollback` restores it and says
  which kernel you'll get, and the boot menu's undo entry starts it.
  `bunny status` warns if `/boot` has a kernel the system has no drivers for.
- `tools/bunny-vm.sh stop` shuts the test VM down from your desktop.
- `bunny --help` described `report` with `import`'s words; each now has its own.
- `tools/convert-layout.sh` moves an existing Gentoo on btrfs (with `/` at the
  top of the volume) to the Bunny Linux layout, so undo works. Plan first,
  nothing deleted, and a boot menu entry back to the old system.

- Ctrl+C is safe everywhere. At a question it counts as "no" and the config
  is put back; during an install, bunny says how far it got and how to undo
  (`bunny rollback`) or finish (`bunny switch`) it. Before, it stopped with
  no explanation, and could leave a package in `bunny.toml` that was never
  installed.
- BunnyOS is now called **Bunny Linux**. The boot menu entry reads
  "Bunny Linux: undo the last change". The `bunny` command is unchanged.

- The install progress line keeps moving: a spinner, how long the current
  package has taken, a percentage when the build reports one, and otherwise
  how long it took last time. It never wraps in a narrow terminal.
- The install list shows what you asked for first, then "And N packages it
  needs", folded after 15, so it stays readable in a narrow terminal.
- Before a big compile, bunny says when a prebuilt version exists
  (`www-client/firefox-bin` for `www-client/firefox`).
- Developer mode (`[dev] report`): bunny records every command as it looked
  in the terminal, and `bunny report` sends the recordings to a private
  GitHub repository as an issue, with personal details swapped out. With
  `report = "auto"`, problems are sent by themselves.

- bunny installs with emerge from the Bunny Linux overlay (`app-admin/bunny-bin`
- A switch with nothing else to do records packages installed outside bunny
  in `bunny.lock`, and `bunny status` says so instead of suggesting a fix
  that didn't apply.

## [0.1.0] - 2026-11-17

The first release: version 1 of bunny.

### Reading a system
- `bunny import` writes a `bunny.toml` from the system as it is (world set,
  services, USE flags).
- `bunny check` finds mistakes in the config and says how to fix each.
- `bunny plan` shows what a switch would change, including packages that
  don't exist and differences from the lock.
- `bunny status`: one screen saying whether everything is in order.
- `bunny diff`: config and system side by side, with both ways to fix each
  difference.
- `bunny status` says whether any Gentoo security advisories affect the
  system (needs gentoolkit's `glsa-check`).

### Changing a system
- `bunny switch` makes the system match the config: packages, USE flags,
  services. It takes over `package.use` (nothing is lost; old files are
  backed up).
- `bunny search`, `bunny add` and `bunny remove` (comments in the config are
  kept).
- `bunny update` syncs, updates and records the new versions.
- Quieter emerge: a short install list first, one progress line, the full
  output saved to `/var/log/bunny/`, and package messages summarised. The
  news tip only comes back when the number of unread items changes, and
  emerge's "remove unused packages" reminder isn't repeated.
- `[security]`: sudo (the default) or doas, checked before it's put in place,
  and built so it can't lock you out.

- Install plans come straight from Portage (a small helper script that
  returns JSON) instead of reading emerge's printed text, so they don't break
  when emerge changes its wording. emerge's text is still the fallback.

### The same system everywhere
- `bunny.lock` records every installed version; switches keep to it with
  version pins, and only `bunny update` moves it.

### Undo
- A generation (btrfs snapshot) before every change; `bunny rollback`,
  `bunny rollback list` and `bunny clean`.
- `[boot] undo_entry`: an "undo the last change" entry in the boot menu, for
  when a change stops the system from starting.
- Undoing a change (from bunny or the boot menu) puts `bunny.toml` and
  `bunny.lock` back too, so config and system match right away.
- After an undo, the boot menu's undo entry is removed until the next change,
  so picking it again can't take you further back; picking it twice does
  nothing.

### Errors in plain English
- `bunny explain`, and automatic explanations when emerge stops: USE changes,
  testing and live versions, masks, slot conflicts, blockers, download
  failures, typos, and flag rules emerge can't satisfy.

### Docs and checks
- The Bunny Linux Guide, at https://bunnyos-dev.github.io/bunnyos/.
- Tests on every pull request.
