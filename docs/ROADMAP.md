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
| 3. Undo | snapshots, `generations`, `rollback`, `clean`, boot-menu undo | ✅ Done, tested in the VM |
| 4. Same system everywhere | lockfile, `update` | ✅ Done, tested in the VM |
| 5. Understandable errors | plain-English emerge errors, `status`, `diff` | ✅ Done |
| 6. Fast installs | prebuilt package cache, `bunny serve` | ⬜ |
| 7. The look | boot animation, GRUB theme, login screen | ⬜ |
| 8. Security + gaming | Secure Boot, gaming kernel | ⬜ |
| 9. Installing BunnyOS | installer, ISO, hosting | ⬜ |

**Version 1** (phases 1–5, running on an existing Gentoo install) is roughly
**two-thirds done**. The **whole vision** (all 9 phases) is roughly **a fifth done**.

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
- [x] `bunny lock`: exact installed versions and package-tree snapshot in
      `bunny.lock`; `bunny plan` shows drift from it
- [x] Version pins: `bunny switch` keeps to the lock and updates it;
      `bunny update` syncs, updates everything and relocks
- [x] `bunny explain`: emerge messages in plain English with a fix; risky
      live/`**` versions flagged
- [x] Automatic explanations: `switch`/`update` explain emerge failures
      themselves, turn its notices into tips, and catch misspelled package
      names before changing anything
- [x] `bunny search` / `add` / `remove`: search names and descriptions,
      short names and typo suggestions, config edits that keep comments
- [x] VM tools: `tools/bunny-vm.sh` (UEFI, Secure Boot, TPM, snapshots) and
      `tools/vm-install-gentoo.sh` (the BunnyOS disk layout)

## Version 1 — still to do

Order agreed with the maintainer: search/add/remove, then the rest of the
list, with the CI check along the way. (The Guide was moved up and is done;
feature PRs now update it as they go.)

- [x] **Lockfile, part 1:** `bunny lock` writes `bunny.lock` next to the
      config: the `::gentoo` snapshot (git commit or timestamp) and the exact
      installed version of every package (from `/var/db/pkg`). `bunny plan`
      shows packages whose installed version differs from the lock. Tests on
      fake roots. **(VM)**
- [x] **Lockfile, part 2:** `bunny update` syncs the tree, runs the switch, and
      rewrites the lock. `bunny switch` refuses to silently drift from the lock
      (explains what changed). **(VM)**
- [x] **Error translation, part 1:** recognise emerge's "USE changes are
      necessary", masked/keyworded package, slot conflict, blocker and fetch
      failure messages and explain each in plain English with the fix. Use real
      outputs as test fixtures (see `tests/fixtures/` once created). Live
      `9999` / `**` suggestions must be flagged as risky, never auto-applied.
- [x] **Error translation, part 2:** run emerge so bunny sees its output
      (while still showing it live), and explain failures automatically at the
      end of `switch` / `update` instead of asking the user to paste them. **(VM)**
- [x] **`bunny search`, `bunny add`, `bunny remove`:** the everyday commands.
      `search` finds packages fast (by name and description, from a local
      index of the package tree) and shows plain-English descriptions.
      `add <name>` and `remove <name>` edit `bunny.toml` for you (keeping your
      comments and layout, via `toml_edit`), accept short names when they're
      unambiguous (`firefox` → `www-client/firefox`), show the plan, and run
      the switch. "Installing" stays declarative: it's always "add to the
      config, then switch". **(VM)**
- [x] **`bunny status`:** one screen saying whether everything is in order:
      config applied (packages or flags added outside bunny show up here),
      services enabled by hand, versions vs the lock, the last change,
      undo available, and when the package tree was last synced. Read-only.
- [x] **`bunny diff`:** the config and the system side by side: packages,
      USE flags (only the flags that differ, in words), services and
      versions, each marked "config only" or "system only", with both ways
      to fix it (`bunny switch`, or config edits + `bunny lock`). Exits 1 when
      they differ.
- [x] **Quieter emerge output:** a dry run first, shown as a short list
      (with dependencies marked), one progress line while installing, the
      full output in `/var/log/bunny/`, package messages summarised at the
      end, and the same for removals. `--verbose` shows everything. Also:
      sudo without a mail server, no sudo lecture. **(VM)**
- [x] **`[security]` section:** `privilege` ("sudo" by default, or "doas"),
      `persist`, `admins`. Installs the tool, writes and checks
      `/etc/sudoers.d/bunny` or `/etc/doas.conf`, adds admins to `wheel`, never
      locks you out. bunny's own messages use whichever tool you have. **(VM)**
- [x] **GRUB "Undo last switch" entry:** `[boot] undo_entry = true` adds a
      dracut step (the same swap as `bunny rollback`, before mounting) and,
      after each switch, a GRUB entry that boots the generation's own kernel
      with `bunny.rollback=<generation>`. **(VM)**
- [x] **The BunnyOS Guide, part 1:** an mdBook site in `docs/guide/`,
      published to GitHub Pages by a GitHub Actions workflow. Two tracks:
      "Start here" (dummy-proof, task-based: what BunnyOS is, first steps,
      install/remove a program, update, undo a mistake, what the lock is) and
      "Going further" (full `bunny.toml` reference, pins, generations, how bunny
      drives Portage). Plus a glossary of Gentoo/NixOS terms. Written for an
      experienced Linux user who has never used NixOS.
- [x] **Undo restores the config too:** a generation is saved after `bunny
      add`/`remove` (or your own edit) has already changed `bunny.toml`, so
      after an undo (`bunny rollback` or the boot menu) the config still lists
      the change. Save the config from *before* the change with each
      generation (next to its note in `/.snapshots`), and put it back on
      rollback, so `bunny status` is ✓ right after an undo. Update the guide's
      "Your config file after a rollback". **(VM)**

## Talk to Portage directly (after the config fix)

Agreed with the maintainer: version 1 reads emerge's screen text, which breaks
whenever emerge changes its wording. The design always said bunny should ask
Portage for structured answers instead.

- [ ] **Portage helper:** a small Python script shipped with bunny that uses
      Portage's own library (the code emerge uses) and prints JSON: what a
      switch would install or remove (with versions, prebuilt or compile),
      and, when it can't, the structured reason (needed USE changes, rules
      a package's flags must meet, masks, keywords, slot conflicts,
      blockers). bunny calls it instead of `emerge --pretend` and
      `--depclean --pretend`, and builds its explanations from the data.
      Reading emerge's text stays as the fallback (and for `bunny explain`,
      which is pasted text). Start with the install list and depclean, then
      move error explanations over one kind at a time. **(VM)**

## Upkeep (after the Portage helper, before phase 6)

Agreed with the maintainer: automate what would otherwise be weekly chores,
so the maintainer's time goes to testing and decisions.

- [ ] **Releases:** version tags (`v0.1.0`, …), a `CHANGELOG.md`, and a
      GitHub Actions workflow that builds bunny and attaches it to a GitHub
      Release when a tag is pushed. Write down the release steps in
      `docs/RELEASING.md`. Release when a few features have piled up and have
      run on a real machine for a while, not on every merge.
- [ ] **Weekly canary check:** a scheduled GitHub Actions job runs bunny in a
      fresh Gentoo container (`gentoo/stage3`) against the latest package
      tree: `bunny import`, `check`, `plan`, and a dry-run install
      (`emerge --pretend`) read by bunny. If anything breaks (emerge changed
      its messages, a profile moved), it opens an issue so the daily session
      picks it up.
- [ ] **Security advisories in `bunny status`:** a line saying whether any
      Gentoo security advisory (GLSA, via `glsa-check`) affects installed
      packages, and to run `bunny update` if so.
- [ ] **`bunny` ebuild in a BunnyOS overlay** (`bunny-overlay` repo, GPL-2):
      a release ebuild and a `-9999` one, so bunny installs with `emerge`.
      Moved up from phase 9.

## Repository housekeeping

- [x] `main` protected from force pushes and deletion
- [x] GitHub Actions check that runs `cargo fmt --check`, `clippy` and
      `cargo test` on every pull request; then require it in the ruleset

## After version 1

Rough order. These get split into PR-sized items when we reach them.

### Phase 6: fast installs
- [ ] **Clean build container** (`[build] isolated = true`): compile inside
      a throwaway container made from a pinned stage3, so the live system's
      state can't leak into builds. Works locally or on a build server.
- [ ] Recipe hash for binary packages (ebuild + USE + CFLAGS + deps)
- [ ] `[build] sources` / `mode`: download from a cache, compile as fallback
- [ ] `bunny serve`: share one machine's builds (e.g. over Tailscale)
- [ ] Per-host CPU targets so one builder can build for another machine
- [ ] **Tested baseline ("channel"), like NixOS channels:** once a week a
      build machine syncs the tree, builds the Baseline package set, boots it
      in a VM automatically, and only then publishes it (tree snapshot +
      binary packages). `bunny update` moves to the latest published
      baseline instead of whatever the tree had that morning. Off-baseline
      setups still work; they just aren't tested by us.

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
- [ ] Quickshell, MangoWM and theme ebuilds in `bunny-overlay` (the repo
      itself is set up during Upkeep), with a job that opens a PR when upstream
      releases a new version

### Long-term: Portage-free systems
- [ ] Portage (Python, its tools) lives only inside the build container;
      installed systems get finished packages through a small Rust installer
      in bunny. Bash probably stays (package install scripts use it).
- [ ] Own dependency solver in Rust (e.g. `resolvo`), with Portage only
      doing the building.

### Outside our control
- Epic accepting BunnyOS's signing key (anti-cheat games)
- A Microsoft-signed shim (Secure Boot on the install USB)

## Known issues / small fixes


- `bunny rollback --root <fake>` still reads the real machine's mount info.
- Kept package.use lines lose their original file name on a second switch.
