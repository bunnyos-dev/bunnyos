# BunnyOS Roadmap

The working to-do list. The daily work session takes the **first unchecked
item that isn't blocked**, does it on its own branch, and opens a pull request
with VM test steps. The maintainer tests in the VM and merges.

Legend: `[ ]` to do · `[~]` PR open, waiting for testing · `[x]` merged
**(VM)** = can only be fully checked in the VM; the PR must include exact steps.

## Version 1

- [ ] **Lockfile, part 1:** `bunny lock` writes `bunny.lock` next to the
      config: the `::gentoo` snapshot (git commit or timestamp) and the exact
      installed version of every package (from `/var/db/pkg`). `bunny plan`
      shows packages whose installed version differs from the lock. Tests on
      fake roots. **(VM)**
- [ ] **Lockfile, part 2:** `bunny update` syncs the tree, runs the switch, and
      rewrites the lock. `bunny switch` refuses to silently drift from the lock
      (explains what changed). **(VM)**
- [ ] **Error translation, part 1:** recognise emerge's "USE changes are
      necessary", masked/keyworded package, slot conflict, blocker and fetch
      failure messages and explain each in plain English with the fix. Use real
      outputs as test fixtures (see `tests/fixtures/` once created). Live
      `9999` / `**` suggestions must be flagged as risky, never auto-applied.
- [ ] **`bunny status`:** drift report: hand-edited files in package.use
      outside `zz-bunny`, world entries added by `emerge` directly, services
      enabled/disabled by hand. Read-only.
- [ ] **`bunny diff`:** show the config vs the system as a unified, readable diff.
- [ ] **Quieter emerge output:** run emerge with less noise and summarise news
      items and post-install messages instead of printing them raw. **(VM)**
- [ ] **`[security]` section:** generate `/etc/doas.conf` (with `persist`) from
      `privilege`, `persist`, `admins`. **(VM)**
- [ ] **GRUB "Undo last switch" entry:** boot-menu rollback for systems that
      won't boot (initramfs step that does the same swap as `bunny rollback`).
      **(VM)**

## Later (not v1)

- Binary cache / `bunny serve`
- Boot experience (Plymouth, GRUB theme, greetd)
- Secure Boot + gaming kernel
- Installer + ISO

## Known issues / small fixes

- `bunny rollback --root <fake>` still reads the real machine's mount info.
- Kept package.use lines lose their original file name on a second switch.
