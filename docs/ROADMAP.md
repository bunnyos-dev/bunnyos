# Bunny Linux Roadmap

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
| 7. Installing Bunny Linux | installer, ISO, hosting | ⬜ (a first, minimal ISO comes sooner: see below) |
| 8. Security + gaming | Secure Boot, gaming kernel | ⬜ |
| 9. The look | boot animation, GRUB theme, login screen | ⬜ |

**Version 1** (phases 1–5, running on an existing Gentoo install) is **done**.
The **whole vision** (all 9 phases) is roughly **a third done**.

**Order agreed with the maintainer (November 2026):**

1. Code cleanup so other people can review the code, then an independent
   review
2. A first, minimal ISO (terminal installer, no desktop)
3. The maintainer's dry run in the VM: install from the ISO, then learn and
   customize the system using only the Guide
4. Guide and bunny fixes from what the dry run finds
5. The maintainer's laptop
6. The rest of the Portage helper

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
- [x] Boot-menu undo can't walk further back: the entry is removed after
      any rollback, and picking it twice does nothing
- [x] VM tools: `tools/bunny-vm.sh` (UEFI, Secure Boot, TPM, snapshots) and
      `tools/vm-install-gentoo.sh` (the Bunny Linux disk layout)

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
- [x] **The Bunny Linux Guide, part 1:** an mdBook site in `docs/guide/`,
      published to GitHub Pages by a GitHub Actions workflow. Two tracks:
      "Start here" (dummy-proof, task-based: what Bunny Linux is, first steps,
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

## Code cleanup and review (before the ISO)

Agreed with the maintainer: make the code easy for other people to read and
review before the installer adds a lot more of it.

- [ ] **Split the big files:** `apply.rs` (about 1,800 lines) into smaller
      modules (the switch steps, quiet install and removal, package.use
      takeover, admin rights), and the command handlers out of `main.rs`
      into their own modules. Long functions (`switch`, `install_quietly`)
      broken into named steps. No change in behaviour; all tests still pass.
- [ ] **`CONTRIBUTING.md`:** a code map for human contributors: where to
      start, what each module does, how a `bunny switch` flows through the
      files, how to run the tests and the VM, and the rules from
      `CLAUDE.md` that apply to everyone. Comments on each Portage call in
      `bunny-portage.py` explaining what it asks Portage and why.
- [ ] **Independent review:** a separate agent that hasn't seen the code
      being written reads the whole repo (Rust, the Portage helper and the
      shell scripts, especially the ones that touch disks) and files each
      finding as an issue. Real bugs get fixed before the ISO work.

## A first ISO (next)

Agreed with the maintainer: a plain, working ISO before the graphical
installer, so Bunny Linux can be installed from scratch and learned the way a
new user would. It boots to a terminal and installs a **minimal** system: the
Bunny Linux disk layout, encryption, OpenRC, a kernel, networking, one user,
sudo and bunny. No desktop: adding one with bunny is the first thing a new
user learns. Packages come from Gentoo's binary host, one by one (the
prebuilt Baseline image comes with phase 6). Not styled yet.

- [ ] **`bunny install`, part 1 (guided):** turn `tools/vm-install-gentoo.sh`
      into a real installer for any disk: lists the disks, asks which one,
      shows exactly what will be erased and asks to type the disk's name.
      Standard layout, LUKS2 encryption on by default. Options for the common
      changes: `--no-encrypt`, partition sizes, a swapfile for hibernation,
      `/home` on a second disk. Refuses to touch a disk that's mounted or in
      use. **(VM)**
- [ ] **`bunny install`, part 2 (the config):** writes a starter
      `bunny.toml` from your answers (user name, password, time zone,
      keyboard, host name), with comments pointing to the Guide. Disk details
      (partition IDs, encryption) go in a separate host file,
      `hosts/<host name>.toml`, so the main config works on any machine.
      `--edit` stops so you can edit the config before installing;
      `--config <file>` installs from your own (written by hand or from a git
      repo). Then runs the first `bunny switch` and `bunny lock` inside the
      new system and turns on boot-menu undo. **(VM)**
- [ ] **`bunny install`, part 3 (manual partitioning):** `bunny install
      --mounted /mnt` for people who partition and mount everything
      themselves. bunny doesn't touch the disks: it reads what's mounted,
      writes the host file from it, and explains anything that limits it
      (ext4: no undo; btrfs without subvolumes: offers to add them). Allowed
      and checked, but not every layout is tested. **(VM)**
- [ ] **The live USB image:** a script that builds a small bootable image
      (UEFI, GRUB) with bunny, `bunny install` and the tools it needs. It
      boots to a terminal welcome: what this is, `bunny install` to start,
      the Guide's URL, and how to get online (Ethernet works by itself;
      Wi-Fi with `iwctl`). **(VM)**
- [ ] **ISO in CI:** a GitHub Actions workflow builds the ISO on a tag (and
      by hand), with SHA256 sums, attached to the GitHub Release.
      `tools/bunny-vm.sh` can boot it (`ISO=…`).
- [ ] **Guide: "Install Bunny Linux":** writing the USB, booting it,
      `bunny install` step by step, first boot, and "what to do first" (add a
      desktop, set up your config in git). Plus a page on turning on
      developer mode during the dry run.
- [ ] **Dry run** (the maintainer): install from the ISO in the VM, then use
      it for a day or two following only the Guide, with developer mode on
      and notes on anything confusing. Each confusing spot becomes a Guide or
      bunny fix below.

### Fixes from the dry run

(Filled in from the maintainer's notes and developer-mode reports.)

- [ ] **Live USB welcome says "Not online yet" when it is online.** The
      welcome checks once, 3 seconds after login, by pinging
      `distfiles.gentoo.org` (needs DNS too). In the VM, `ping 1.1.1.1`
      worked right after. Likely the network or DNS wasn't up yet at that
      moment. Fix: wait up to ~15 s ("Checking the connection…"), check the
      address and DNS separately (so "online, but names don't work" is its
      own message), and say "type welcome to check again".
- [ ] **The live USB console can't draw ✓ or ✗** (they show as `v` and
      `x`, e.g. "v Dry run finished"). Check which symbols the console font
      has (✓ ✗ ⚠ ·) and load a font that has them on the USB (or fall back
      to plain text when the terminal is the Linux console).
- [ ] **"Welcome to your new account!"** appears above the welcome on the
      live USB, which is confusing there (it isn't a new account). Find what
      prints it at the first login and turn it off on the USB.
- [ ] **`bunny install --dry-run` is a wall of commands.** Every
      `(would run)` line is shown, and long ones wrap. For someone learning,
      the steps in plain words would be clearer: each step's name and what
      it does, with the exact commands behind `--dry-run --verbose`.
- [ ] **The installed system's boot menu looks like Gentoo's.** It has the
      Gentoo logo behind it, calls the system "Bunny Linux GNU/Linux", and
      lists every firmware entry (UiApp, DVD-ROM, PXE and HTTP boot, a second
      "Bunny Linux (EFI BootNext)"). Until the real look (phase 9): no Gentoo
      theme, the name "Bunny Linux", and only Bunny Linux, its advanced
      options, Undo and firmware settings in the list.
- [ ] **The Undo entry's time is in UTC** ("back to 2026-11-18 21:46:15 UTC")
      even though the install asked for a time zone. Show local time, and for
      the first generation say "as installed".
- [ ] **`tools/bunny-vm.sh ssh` warns "REMOTE HOST IDENTIFICATION HAS
      CHANGED" for a new VM.** Every named VM (`VM=laptop`, `VM=fresh`, …)
      uses port 2223, so SSH remembers the last VM's key and refuses the new
      one. Fix: keep each VM's key in its own known_hosts file
      (`$VM_DIR/known_hosts`), or give each VM its own port.
- [ ] **Developer mode can't record `bunny install`.** It's set in
      `bunny.toml`, and the live USB has none, so the install itself (the
      part being tested most) isn't recorded. Idea: preview ISOs record by
      default, and the recording is copied into the new system so
      `bunny report` sends it after the first start.

## Talk to Portage directly (after the dry run and the laptop)

Agreed with the maintainer: version 1 reads emerge's screen text, which breaks
whenever emerge changes its wording. The design always said bunny should ask
Portage for structured answers instead.

- [x] **Developer mode:** `[dev] report` records every command as it looked
      in the terminal; `bunny report` files them as an issue in a private
      repo (personal details swapped out); `"auto"` sends problems by
      themselves. Daily sessions read the reports and turn bugs into PRs.
- [ ] **Portage helper:** (part 1, the install list, done in #18; next: depclean,
      then error explanations) a small Python script shipped with bunny that uses
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

- [x] **Releases:** version tags (`v0.1.0`, …), a `CHANGELOG.md`, and a
      GitHub Actions workflow that builds bunny and attaches it to a GitHub
      Release when a tag is pushed. Write down the release steps in
      `docs/RELEASING.md`. Release when a few features have piled up and have
      run on a real machine for a while, not on every merge.
- [x] **Weekly canary check:** a scheduled GitHub Actions job runs bunny in a
      fresh Gentoo container (`gentoo/stage3`) against the latest package
      tree: `bunny import`, `check`, `plan`, and a dry-run install
      (`emerge --pretend`) read by bunny. If anything breaks (emerge changed
      its messages, a profile moved), it opens an issue so the daily session
      picks it up.
- [x] **Security advisories in `bunny status`:** a line saying whether any
      Gentoo security advisory (GLSA, via `glsa-check`) affects installed
      packages, and to run `bunny update` if so.
- [x] **`bunny` ebuild in a Bunny Linux overlay** (`bunny-overlay` repo, GPL-2):
      a release ebuild and a `-9999` one, so bunny installs with `emerge`.
      Moved up from the installing phase.

## Repository housekeeping

- [x] `main` protected from force pushes and deletion
- [x] GitHub Actions check that runs `cargo fmt --check`, `clippy` and
      `cargo test` on every pull request; then require it in the ruleset

## After version 1

Rough order. These get split into PR-sized items when we reach them.

**Function first (decided November 2026):** each thing is made to work well,
plainly styled, before it gets its finished look. The mockups ("Bunny Linux:
boot, installer and desktop mockups") are the reference for later, not the
to-do list. Appearance work comes last and a little at a time.

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

### Phase 7: installing Bunny Linux
- [ ] **`[disk]` section:** describe the disk layout in the config (like
      NixOS's disko), and `bunny install` partitions to match. Used only at
      install, never on a running system.
- [ ] **One-step install from a config:** `bunny install --config
      github:you/my-system` partitions from `[disk]`, writes the Baseline
      image and switches to your config and lock in one command, compiling
      only what isn't prebuilt (in the build container). Needs phase 6.
- [ ] Baseline system image built on the server
- [ ] Slint installer (straight to the installer, no desktop): centered, one
      decision per screen, Utilities menu. Working and plainly styled first;
      the mockups' finished look comes in phase 9
- [ ] Installer terminal mode: a console with a welcome, how to get back to
      the graphical installer, the install guide's URL, and the commands to
      start from the Baseline or your own config
- [ ] Guide: "Installing by hand" chapter (what terminal mode points to)
- [ ] **Reset:** `bunny reset` (keep files; undoable, saves the system first)
      and "Erase everything" from the USB
- [ ] **Bunny Desktop:** Hyprland + Bunny Linux's own Quickshell shell that works
      like a full desktop (settings, launcher, notifications, panels); the
      selling point. Function first: Settings that edit `bunny.toml` and
      Apply / Undo, a launcher that can get apps, notifications with Undo. Plus KDE Plasma, Xfce, bare Hyprland, bare MangoWM and
      "no desktop" as installer choices (`[desktop]` section)
- [ ] ISO build (Net and Offline editions), checksums and signatures
- [ ] Hosting: GitHub Releases + Cloudflare R2
- [ ] Quickshell, MangoWM and theme ebuilds in `bunny-overlay` (the repo
      itself is set up during Upkeep), with a job that opens a PR when upstream
      releases a new version

### Phase 8: security + gaming
- [ ] `secure_boot = true`: sbctl keys (keeping Microsoft's), automatic signing
- [ ] `bunny doctor secureboot` / `gaming` checklists
- [ ] Gaming kernel boot entry (lockdown, IOMMU, BPF LSM)
- [ ] Dual-boot helpers (BitLocker check, clock, boot order)
- [ ] TPM disk unlock

### Phase 9: the look (design work, done together, a little at a time)
- [ ] Bunny Linux logo (hired designer) and the palette from DESIGN.md "Look and feel"
- [ ] Installer and Bunny Desktop styled to match the mockups, one screen
      per PR
- [ ] Plymouth boot animation, Esc shows the kernel log
- [ ] GRUB theme, hidden menu, flicker-free handoff to Plymouth
- [ ] greetd + Quickshell login screen
- [ ] `[boot]` config section; `bunny boot list` / `preview`

### Long-term: Portage-free systems
- [ ] Portage (Python, its tools) lives only inside the build container;
      installed systems get finished packages through a small Rust installer
      in bunny. Bash probably stays (package install scripts use it).
- [ ] Own dependency solver in Rust (e.g. `resolvo`), with Portage only
      doing the building.

### Outside our control
- Epic accepting Bunny Linux's signing key (anti-cheat games)
- A Microsoft-signed shim (Secure Boot on the install USB)

## The maintainer's laptop

Getting Bunny Linux onto a real, encrypted Gentoo laptop safely. Everything
is tested first in a VM with the same layout.

- [x] **A second VM shaped like the laptop** (`VM=laptop`, `LAYOUT=laptop`):
      LUKS2, the EFI partition at `/boot`, btrfs without subvolumes, `/srv`
      as a subvolume, a swapfile.
- [x] **Conversion script** to the Bunny Linux layout (`@`, `@home`,
      `@snapshots`, …, swap in its own subvolume), dry run first, tested in
      the laptop VM until it's boring.
- [ ] **A kernel per generation on the EFI partition**, so undo works across
      kernel updates when `/boot` isn't part of the system (encrypted disks).
- [ ] **The laptop itself**, after a backup of `/home`.

## Known issues / small fixes


- The boot menu's main entry says "Gentoo GNU/Linux", not Bunny Linux (set
  `GRUB_DISTRIBUTOR`; part of the boot menu work).
- [x] The install progress line (`[2/2] Compiling …`) wasn't cleared before
      "✓ Installed 2 packages", and stood still during long builds. Now a
      spinner, times and a percentage; the list fits narrow terminals.
- [x] Pressing Ctrl+C during an install ended bunny with no explanation, and
      at a question it could leave the package in `bunny.toml`. Now Ctrl+C
      at a question means no, and during an install bunny explains.
