# Changelog

Every change people would notice, newest first. Each pull request adds its
line under **Unreleased**; a release moves those lines under its version
(see `docs/RELEASING.md`).

The format follows [Keep a Changelog](https://keepachangelog.com), and
versions follow [Semantic Versioning](https://semver.org) (while it's 0.x,
a minor version can change how things work).

## [Unreleased]

The first release, `0.1.0`, will be version 1 of bunny:

### Reading a system
- `bunny import` writes a `bunny.toml` from the system as it is (world set,
  services, USE flags).
- `bunny check` finds mistakes in the config and says how to fix each.
- `bunny plan` shows what a switch would change, including packages that
  don't exist and differences from the lock.
- `bunny status`: one screen saying whether everything is in order.
- `bunny diff`: config and system side by side, with both ways to fix each
  difference.

### Changing a system
- `bunny switch` makes the system match the config: packages, USE flags,
  services. It takes over `package.use` (nothing is lost; old files are
  backed up).
- `bunny search`, `bunny add` and `bunny remove` (comments in the config are
  kept).
- `bunny update` syncs, updates and records the new versions.
- Quieter emerge: a short install list first, one progress line, the full
  output saved to `/var/log/bunny/`, and package messages summarised.
- `[security]`: sudo (the default) or doas, checked before it's put in place,
  and built so it can't lock you out.

### The same system everywhere
- `bunny.lock` records every installed version; switches keep to it with
  version pins, and only `bunny update` moves it.

### Undo
- A generation (btrfs snapshot) before every change; `bunny rollback`,
  `bunny rollback list` and `bunny clean`.
- `[boot] undo_entry`: an "undo the last change" entry in the boot menu, for
  when a change stops the system from starting.

### Errors in plain English
- `bunny explain`, and automatic explanations when emerge stops: USE changes,
  testing and live versions, masks, slot conflicts, blockers, download
  failures, typos, and flag rules emerge can't satisfy.

### Docs and checks
- The BunnyOS Guide, at https://bunnyos-dev.github.io/bunnyos/.
- Tests on every pull request.
