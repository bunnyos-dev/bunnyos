# BunnyOS Roadmap

The working to-do list. The daily work session takes the **first unchecked
item that isn't blocked**, does it on its own branch, and opens a pull request
with VM test steps. The maintainer tests in the VM and merges.

Legend: `[ ]` to do · `[~]` PR open, waiting for testing · `[x]` merged
**(VM)** = can only be fully checked in the VM; the PR must include exact steps.

## Where we are

| Phase | What it gives you | Status |
|---|---|---|
| 1. Read a system | `import`, `check`, `plan` | ✅ Done, tested on a real desktop |
| 2. Change a system | `switch`: packages, USE flags, services, backups | ✅ Done, tested in the VM |
| 3. Undo | snapshots, `generations`, `rollback`, `clean` | ✅ Done, tested in the VM (boot-menu undo still to do) |
| 4. Same system everywhere | lockfile, `update` | 🔶 Part 1 in review |
| 5. Understandable errors | plain-English emerge errors, `status`, `diff` | ⬜ |
| 6. Fast installs | prebuilt package cache, `bunny serve` | ⬜ |
| 7. The look | boot animation, GRUB theme, login screen | ⬜ |
| 8. Security + gaming | Secure Boot, gaming kernel | ⬜ |
| 9. Installing BunnyOS | installer, ISO, hosting | ⬜ |

**Version 1** (phases 1–5, running on an existing Gentoo install) is roughly
**half done**. The **whole vision** (all 9 phases) is roughly **a fifth done**.

### Done so far

- [x] `bunny import`: write a `bunny.toml` from the current system (world set,
      services, package.use incl. versioned and USE_EXPAND lines)
- [x] `bunny check`: plain-English config mistakes (incl. the TOML section trap)
- [x] `bunny plan`: packages, USE flags in effect, services, leftover services
- [x] `bunny switch`: snapshot, backups, package.use takeover, world, emerge,
      depclean (always asks), enable + start services, doas self-elevation
- [x] `bunny generations` / `rollback list`: numbered, with notes and a
      "running now" marker
- [x] `bunny rollback`: swap to a snapshot (undoable), carries nested
      subvolumes like `/srv`
- [x] `bunny clean`: remove old generations and leftovers safely
- [x] VM tools: `tools/bunny-vm.sh` (UEFI, Secure Boot, TPM, snapshots) and
      `tools/vm-install-gentoo.sh` (the BunnyOS disk layout)

## Version 1 — still to do

- [~] **Lockfile, part 1:** `bunny lock` writes `bunny.lock` next to the
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

## After version 1

Rough order. These get split into PR-sized items when we reach them.

### Phase 6: fast installs
- [ ] Recipe hash for binary packages (ebuild + USE + CFLAGS + deps)
- [ ] `[build] sources` / `mode`: download from a cache, compile as fallback
- [ ] `bunny serve`: share one machine's builds (e.g. over Tailscale)
- [ ] Per-host CPU targets so one builder can build for another machine

### Phase 7: the look (design work, done together)
- [ ] BunnyOS logo and color palette (pick from options)
- [ ] Plymouth boot animation, Esc shows the kernel log
- [ ] GRUB theme, hidden menu, flicker-free handoff to Plymouth
- [ ] greetd + Quickshell login screen
- [ ] `[boot]` config section; `bunny boot list` / `preview`

### Phase 8: security + gaming
- [ ] `secure_boot = true`: sbctl keys (keeping Microsoft's), automatic signing
- [ ] `bunny doctor secureboot` / `gaming` checklists
- [ ] Gaming kernel boot entry (lockdown, IOMMU, BPF LSM)
- [ ] Dual-boot helpers (BitLocker check, clock, boot order)
- [ ] TPM disk unlock

### Phase 9: installing BunnyOS
- [ ] `bunny install` (from `tools/vm-install-gentoo.sh`), encryption by default
- [ ] Baseline system image built on the server
- [ ] Slint installer (straight to the installer, no desktop)
- [ ] ISO build (Net and Offline editions), checksums and signatures
- [ ] Hosting: GitHub Releases + Cloudflare R2
- [ ] `bunny-overlay` repo (Quickshell, MangoWM, themes, bunny itself)

### Outside our control
- Epic accepting BunnyOS's signing key (anti-cheat games)
- A Microsoft-signed shim (Secure Boot on the install USB)

## Known issues / small fixes

- `bunny rollback --root <fake>` still reads the real machine's mount info.
- Kept package.use lines lose their original file name on a second switch.
