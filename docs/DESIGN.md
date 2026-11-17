# BunnyOS Design

> Status: draft, November 2026. This is the source of truth for what BunnyOS is
> and what gets built in which order. Change it when a decision changes.

## The idea

BunnyOS is a Linux distribution built **on top of Gentoo**, not from scratch.
It keeps what makes Gentoo great (OpenRC, USE flags, compiling for your own
hardware) and adds what Gentoo is missing:

- **NixOS-style reproducibility**: one config file describes the whole machine.
- **A polished experience**: a quiet boot, a fast installer, plain-English errors.

The guiding rule comes from Steve Jobs: start with the experience the user
has, and work backwards to the technology.

## Principles

1. **Experience first.** If something is confusing, slow or ugly for the user,
   that's a bug, even if it's technically correct.
2. **One file is the truth.** `bunny.toml` describes the system. Everything
   bunny does can be reproduced from it on another machine.
3. **Nothing changes silently.** Every change is previewed, recorded in the
   config, and can be rolled back.
4. **Safe by default.** bunny runs as root. Destructive actions always show a
   preview first. The disk is never repartitioned outside of install.
5. **Defaults are supported, options are experimental.** Every option
   multiplies the testing work. An option is only "supported" once it's tested.
6. **Explain, don't dump.** Never show raw tool output when a sentence and a
   suggested fix would do.
7. **Don't depend on others for the core.** Things outside our control (game
   anti-cheat trust, Microsoft-signed shim, third-party bootloaders) stay
   optional.

## Architecture overview

```
bunny.toml  (what you want)
    │
    ▼
bunny plan  ──►  bunny.lock  (exactly what you get: tree commit, versions,
    │                          USE flags, binary hashes)
    ▼
build / download  (your builder → Gentoo binhost → compile locally)
    │
    ▼
bunny switch  ──►  new generation (btrfs snapshot), GRUB entry for rollback
```

- **Base:** Gentoo, Portage, the `::gentoo` tree, plus a `::bunny` overlay.
- **Language:** bunny, the installer and future tools are written in **Rust**.
  bunny talks to Portage through a small Python helper that returns JSON,
  instead of scraping emerge's text output. (Version 1 reads emerge's text
  as a shortcut; the helper is the next item after version 1, with text
  reading kept as a fallback.)
- **Init:** OpenRC. systemd may be added later (see "Init systems").

## The `bunny` tool

### Config and layers

`bunny.toml` is TOML. Layers stack, each overriding the one below:

```
BunnyOS Baseline   tested defaults, fully prebuilt
  └ your layer     personal tweaks (USE, CFLAGS, extra packages)
     └ host layer  per-machine (hosts/desktop.toml, hosts/laptop.toml)
```

Modules expand one line into packages + services + config files, for example
`[desktop] wm = "hyprland"` or `[security] encryption = true`.

### Lockfile

- `bunny.lock` is generated, never hand-edited.
- It records: the `::gentoo` commit, overlay commits, and for every package the
  version, USE flags and binary hash.
- `bunny update` is the only command that changes the lock.
- `bunny switch` installs exactly what the lock says, so dependency solving
  only happens during `bunny update`. This is also the main speed win over
  plain emerge.

**Current behaviour (part 1):** `bunny lock` writes `bunny.lock` next to
the config (`bunny.toml` → `bunny.lock`) with each repository's snapshot (git
commit, or the sync timestamp from `metadata/timestamp.chk`) and the exact
installed version of every package from `/var/db/pkg` (keyed `cat/name`, or
`cat/name:slot` when several slots are installed). `bunny plan` shows any
difference between the system and the lock.

**Part 2:** `bunny switch` pins every locked package with a Portage mask
(`/etc/portage/package.mask/zz-bunny-lock`, one `>cat/pkg-<locked version>`
line each), so a switch never moves past the locked versions, and a package
that drifted newer is put back. After the switch it rewrites the lock from the
system (creating it if there was none) and shows what changed.
`bunny update` is the only way forward: it runs `emerge --sync`, removes the
pins, takes a snapshot, runs the full world update, then writes the new lock
and pins. If the update fails, the old pins are put back. Limit: a locked
version that Gentoo has since removed from its tree can't be reinstalled on a
new machine; `bunny update` moves past it.

**Reproducibility is "same recipe", not bit-for-bit.** Same lock means same
versions, flags and settings. Locally compiled binaries can still differ
(timestamps, CPU flags). This is a deliberate difference from NixOS.

### Builds and binaries

```toml
[build]
mode = "hybrid"        # "cache" | "local" | "hybrid"
sources = ["http://desktop.tailnet:8080", "gentoo"]
fallback = "local"
isolated = false       # compile inside a clean container
```

- Binaries are looked up by a hash of their recipe (ebuild, USE, CFLAGS, deps).
  Changing a flag changes the hash, so that package is built locally while
  everything else still downloads.
- `bunny serve` shares one machine's builds with others (e.g. over Tailscale).
- Each host declares its CPU so a builder can compile for another machine's
  CPU, not just its own.

### Generations and rollback

- Every `bunny switch` creates a generation: a btrfs snapshot of `/`, labelled
  with the lock hash.
- GRUB shows a "Previous versions" submenu.
- `/home`, logs, caches and build scratch space live on separate subvolumes and
  are never rolled back.

### Rollback (current behaviour)

- `bunny generations` lists the snapshots in `/.snapshots/bunny-*`.
- `bunny rollback [name]` (default: the latest switch) mounts the btrfs top
  level, snapshots the current `@` as `bunny-<time>-before-rollback` (so the
  rollback can be undone), moves `@` aside to `@replaced-<time>`, and makes a
  writable copy of the chosen snapshot the new `@`. It takes effect on reboot.
  If restoring fails, the current system is moved back unchanged.
- `/boot` lives inside `@`, so the kernel rolls back together with the system.
- **Nested subvolumes** (subvolumes inside `@`, e.g. `/srv` when tmpfiles
  creates it as a subvolume on btrfs) aren't part of snapshots. Rollback moves
  them over to the restored system, like `/home`, and `clean` deletes them
  inside-out. The installer creates `/srv` as a plain folder to avoid this.
- `clean` never removes the generation you're running a copy of.
- **The config goes back too:** after every successful switch or update (and
  a switch with nothing to do), bunny copies the config, its lock and the
  config's path to `/var/lib/bunny/applied/`. That's inside `@`, so each later
  snapshot carries the config that matches it. Rollback (and the boot-menu
  hook) copies it back over the config in the restored `@`, only for a config
  under `/etc` (one in `/home` is on its own subvolume and isn't rolled
  back).
- Each snapshot gets a note in `/.snapshots/<name>.toml` saying what came
  right after it (a switch with a short summary, or a rollback and its
  target), and `/.snapshots/bunny-state.toml` remembers the last action, so
  `bunny generations` can say what each one is and which one is running.
  `/.snapshots` is its own subvolume, so notes survive rollbacks.
- `bunny clean [--keep N]` removes old generations (keeps the newest 5 by
  default) and `@replaced-*` leftovers, never the subvolume `/` is running
  from. It shows the list first and defaults to No, because it can't be undone.
- **Undo from the boot menu (current behaviour, opt-in with `[boot]
  undo_entry = true`):** a dracut module (`90bunny-rollback`, pre-mount hook)
  does the same swap in the initramfs when the kernel command line has
  `bunny.rollback=<generation>`, and nothing otherwise. bunny installs it and
  rebuilds `/boot/initramfs-<ver>.img` once (staged as `.bunny-new`, old image
  backed up), *before* the switch's snapshot so that generation already has
  it. After each switch it writes a block in `/boot/grub/custom.cfg` (read at
  boot via `41_custom`, so no `grub-mkconfig`) that boots the kernel and
  initramfs **from the generation's snapshot** (`/@snapshots/<gen>/boot/…`)
  with the running system's command line plus `bunny.rollback=<gen>`, so
  kernel modules match after the swap. Only added when the generation's
  initramfs is newer than its copy of the module. Limits: nested subvolumes
  aren't carried over by the boot-time swap; needs GRUB, dracut and
  `root=UUID=`. Off by default until it's had more testing; the installer
  will turn it on.

### Explaining and fixing errors

- bunny recognises the common emerge failures (slot conflicts, blockers, masked
  or keyworded packages, required USE changes, circular dependencies, fetch
  failures) and explains each in plain English with a suggested fix.
- Build logs are hidden behind a progress bar; on failure bunny shows only the
  lines that matter.
- `bunny explain` translates any pasted emerge message.
- Auto-fix levels:

```toml
[fix]
mode = "auto-safe"     # "ask" | "auto-safe" | "off"
```

  - Automatic: required USE changes, mirror retries, preserved-library
    rebuilds, merging config updates for managed files.
  - Asks first: testing (`~amd64`) versions, licence acceptance, removing or
    downgrading packages, leaving the Baseline.
  - Can't fix (but helps): upstream compile errors.
- **Every fix is written into `bunny.toml` with a comment explaining why**,
  never hidden in `/etc/portage`.
- **Current behaviour (part 1):** `bunny explain [file]` (or pasted text)
  recognises required USE changes (both message forms), keyword changes
  (testing versions, and live `9999` / `**` versions flagged as risky with a
  "don't accept" warning), masked packages (with Gentoo's reason), slot
  conflicts, blockers, fetch failures, waiting config-file updates, unread
  news and the depclean reminder. Unknown messages are reported honestly.
  Real outputs live in `crates/bunny/tests/fixtures/` as test cases.
- **Part 2:** `switch` and `update` run emerge with its output passed
  through live (byte by byte, so `[Yes/No]` questions still show and answers
  still reach it) while keeping a copy (`--color=y` on terminals, color codes
  stripped before explaining). If emerge fails, bunny prints its explanation
  and where the old system is saved. If it succeeds, notices (news,
  reminders) are shown as short tips after the summary. Also recognises
  misspelled or removed package names.

- **Part 3 (quieter output):** by default `switch`/`update` run
  `emerge --pretend` first (output captured, nothing changes), show the
  merge list as a short table (prebuilt vs compile, what's a dependency), and
  ask again only if emerge wants more than bunny's plan showed. If emerge
  refuses at that stage, bunny restores the world file and package.use from
  a full copy taken before the takeover, so the system is untouched. The real
  run has no `--ask`; one progress line is drawn from `>>> Emerging (n of m)`
  lines, and the full output is saved to `/var/log/bunny/<time>-emerge.log`.
  Package messages (elog) are shown at the end, a few lines each. Removal
  uses `emerge --depclean --pretend`, lists the packages from its "All
  selected packages" line (falling back to emerge's own `--ask` if the count
  doesn't match), and always asks. Cleanup also runs after USE changes and
  updates, since both can orphan dependencies. `--verbose` restores the old
  live, emerge-asks behaviour.
- **BunnyOS default USE flags:** packages bunny installs itself can get
  default flags (applied unless the config has its own `[pkg]` entry):
  sudo is built with `-sendmail`, so it doesn't pull in a mail server.

### Commands (planned)

| Command | Does |
|---|---|
| `bunny import` | Write a `bunny.toml` from the current system (world set, services, package.use) |
| `bunny check` | Validate `bunny.toml` and explain any problems |
| `bunny plan` | Show what would change, how long it takes, what loses prebuilt binaries |
| `bunny lock` | Record exactly what's installed in `bunny.lock` |
| `bunny update` | Sync the tree and refresh `bunny.lock` |
| `bunny switch` | Apply the lock as a new generation |
| `bunny generations` | List saved generations |
| `bunny rollback [number or name]` | Restore a generation (takes effect on reboot); `bunny rollback list` lists them |
| `bunny clean [--keep N]` | Remove old generations and rollback leftovers |
| `bunny try` | Build and boot a generation without making it permanent |
| `bunny status` | One screen: config applied?, versions vs lock, last change, undo, last sync |
| `bunny diff [--baseline]` | Config vs system side by side, with both ways to fix each difference (`--baseline`: later) |
| `bunny why <pkg>` | Explain why a package or flag is set |
| `bunny flags <pkg>` | Explain a package's USE flags in plain English |
| `bunny pin <pkg> <ver>` | Pin a package (keeps a copy of its ebuild and binary) |
| `bunny search <term>` | Fast search from a prebuilt index |
| `bunny add <name>` / `bunny remove <name>` | Edit `bunny.toml` for you (keeps comments), then switch |
| `bunny explain [file]` | Explain an emerge message in plain English |
| `bunny news` | Gentoo news items, translated, marked if they affect you |
| `bunny adopt <file>` | Bring a hand-edited config file under bunny's management |
| `bunny doctor [topic]` | Checklists (e.g. `secureboot`, `gaming`) |
| `bunny shell <pkg>` | Run a program in a throwaway sandbox without installing it |
| `bunny dev` | Enter a project's dev environment from `bunny.dev.toml` |
| `bunny serve` | Share this machine's builds |
| `bunny boot list/preview` | Inspect boot entries; preview the theme in a VM |
| `bunny reboot windows` | Reboot into another OS once |
| `bunny disk backup-header` | Back up the LUKS header |

Read-only commands never need root. Commands that change the system ask for
elevation themselves through the configured tool (doas by default).

### `bunny switch` (current behaviour)

1. Shows the plan and asks before changing anything (`--yes` skips this).
   Re-runs itself through doas when not root.
2. Takes a read-only btrfs snapshot of `/` in `/.snapshots/bunny-<time>`. If
   that fails, nothing is changed.
3. Backs up the world file and package.use to `/var/lib/bunny/backups/<time>/`.
4. **Takes over package.use:** the user's files move to the backup, and
   bunny writes `zz-bunny` (generated from the config, read last so it wins)
   plus `00-bunny-kept` (lines bunny can't manage yet, word for word).
5. Writes the world file from `packages`.
6. Runs `emerge --update --deep --newuse @world` (with `--ask` unless `--yes`).
7. Runs `emerge --depclean --ask` if packages were removed. **Always asks.**
8. Enables services with `rc-update` and starts them, checking again after install.

With `--root` pointing anywhere but `/`, files are written to that root and
commands are only printed.

### Everyday commands (current behaviour)

- `bunny search <words>` searches package names and descriptions from each
  repository's metadata cache (`metadata/md5-cache`; folder names when a
  repository has no cache), best matches first, marking installed ones.
- `bunny add <names>` accepts short names when they're unambiguous
  (`tmux` → `app-misc/tmux`), suggests similar names for typos, and asks you
  to pick when several categories share a name. It edits `bunny.toml` with
  `toml_edit`, keeping comments, order and layout (new entries go in
  alphabetical position; comments stay with their own lines), then runs the
  switch. If nothing gets applied (cancelled, or problems found), the config
  is put back as it was.
- `bunny remove <names>` takes full or short names and works the same way;
  removing from the system goes through `emerge --depclean --ask`.
- `plan`'s "doesn't exist" check uses the same package index.
- `bunny diff` is read-only. It shows packages, USE flags (only the flags
  that differ, as "x on"/"x off"), services and lock versions, each marked
  "config only" or "system only", then both ways to resolve them:
  `bunny switch`, or the config edits (and `bunny lock`) that keep the system
  as it is. It suggests edits rather than `bunny add`/`remove`, because those
  switch and would apply the other differences too. Exit status: 0 when
  there are no differences, 1 when there are.
- `bunny status` is read-only and needs no root. One line each, marked
  ✓/⚠/✗ with what to do underneath:
  - **Config:** applied or not (anything added outside bunny, like `emerge
    foo` or a hand-edited package.use file, shows as a change not applied),
    plus services started at boot that aren't in the config.
  - **Versions:** installed versions vs `bunny.lock`.
  - **Last change:** from `bunny-state.toml`, and whether a rollback is
    waiting for a restart.
  - **Undo:** whether `/.snapshots` exists, and how many generations.
  - **Security:** Gentoo security advisories affecting the system, from
    `glsa-check --nocolor --list affected` (gentoolkit), lines like
    `202501-03 [N] Title ( cat/pkg )`; `[A]`/`[U]` are skipped. Not
    installed: a neutral suggestion to add gentoolkit. Only on the real
    system.
  - **Updates:** when the Gentoo tree was last synced (newest of
    `metadata/timestamp.chk` and `.git/FETCH_HEAD`); older than 7 days is a ⚠.
    It doesn't run emerge to count pending updates, to stay instant.

### Config files (`/etc`)

- Managed files are stored as **overrides on top of the package default**:

```toml
[files."/etc/ssh/sshd_config"]
set = { PasswordAuthentication = "no" }
```

- When an update ships a new default, bunny reapplies the overrides. It only
  asks when the same line changed on both sides.
- Untouched files take the new version silently. Hand-edited unmanaged files
  are flagged and can be adopted.
- No `._cfg0000_` files are left behind.

### Keeping explanations current

- bunny explains its own actions, so the explanation can't drift from the code.
- Package details come from Gentoo's own metadata (USE descriptions, mask
  reasons, news items), pinned by the lock.
- Docs link to the Handbook and wiki instead of copying them. CI checks links.
- A scheduled job watches relevant Handbook pages and opens an issue when one
  changes.

## System options

```toml
[system]
init = "openrc"        # systemd: later
libc = "glibc"         # musl: experimental
userland = "gnu"       # busybox: experimental, cache mode only
multilib = true        # decided at install, can't change later

[gaming]
steam = true           # ensures glibc + multilib, or offers Flatpak Steam

[security]
privilege = "sudo"     # "sudo" (default) | "doas"; "sudo-rs" later
persist = true
admins = ["you"]
```

- Steam's Linux client went 64-bit in October 2026, but Valve still requires
  32-bit libraries. bunny enables 32-bit only for the packages Steam needs.
- **sudo is the default** (decided November 2026): most scripts, guides and
  answers online assume it, so it's the least surprising for new users. doas
  is a one-word choice. sudo-rs is planned once Gentoo marks it stable (it's
  `~amd64` only today, and bunny doesn't manage testing versions yet).
- **Current behaviour:** `[security]` is optional; without it bunny doesn't
  touch admin rights. With it, a switch installs the tool (added to the world
  set), writes `/etc/sudoers.d/bunny` (mode 0440) or `/etc/doas.conf` (0600)
  granting `wheel`, checked first with `visudo -cf` / `doas -C` from a staged
  `.bunny-new` file, then adds `admins` to `wheel` with `gpasswd`. This
  happens after emerge and before depclean, so a failed check stops the
  switch while the old tool still exists. bunny never removes users from
  `wheel`, and refuses to switch if `wheel` would be empty or an admin isn't a
  user. `bunny import` writes the section commented out, with the installed
  tool and wheel members. bunny's own elevation and every command it suggests
  use the configured tool if installed, otherwise whichever of sudo/doas is.
- On doas systems an optional `sudo` shim covers scripts that hardcode `sudo`.
- GUI permission prompts use polkit with a BunnyOS-styled agent.

### Init systems

- **OpenRC is the default and the only init system in v1.**
- Services are listed by plain name and translated by bunny.
- Rust code talks to the init system through one interface (enable, disable,
  start, stop, status) so systemd can be added later as a second implementation.
- The init system is chosen at install time, never switched later.

## Packages and apps

1. **Portage + binhost** for the base system.
2. **`::bunny` overlay** (separate repo, ebuilds licensed GPL-2) for Quickshell,
   MangoWM, themes and bunny itself. Live (`-9999`) ebuilds cover "install
   straight from git", instead of pkgit, which installs files Portage can't
   track.
3. **Flatpak** for large GUI apps.

## Desktop

Decided November 2026. The installer offers, in this order:

1. **BunnyOS Desktop** (the selling point, recommended): Hyprland with
   BunnyOS's own Quickshell shell that works like a full desktop (settings,
   app launcher, notifications, file and Wi-Fi panels), so someone who never
   wants to open a terminal doesn't have to.
2. **KDE Plasma**: a full, familiar desktop (works on OpenRC with elogind).
3. **Xfce**: light and classic.
4. **Hyprland, set it up yourself**: plain Hyprland and a terminal.
5. **MangoWM, set it up yourself.**
6. **No desktop**: terminal only.

GNOME leans heavily on systemd, so it isn't offered while OpenRC is the only
init system. BunnyOS Desktop is the one that gets the polish and testing;
the others are packaged and supported, the "set it up yourself" ones on
purpose bare. Config: `[desktop] name = "bunny" | "plasma" | "xfce" |
"hyprland" | "mango" | "none"`, `accent = "#…"`.

## Boot experience

```
power on → hidden GRUB (hold Esc for the BunnyOS-themed menu)
         → Plymouth boot animation (Esc shows the kernel log)
         → password prompt inside Plymouth (if encrypted)
         → greetd + Quickshell greeter
```

- `quiet splash loglevel=3`, early KMS, and `GRUB_GFXPAYLOAD_LINUX=keep` for a
  flicker-free handoff.
- GRUB is the default loader with a real BunnyOS theme, hidden by default.

```toml
[boot]
loader = "grub"
menu = "hidden"        # "hidden" | "always" | "timeout"
theme = "bunny"
default = "last-used"
kernel_args = ["quiet", "splash"]
```

- **aarambh** (graphical UEFI menu) can become an option if it stays
  maintained or is forked. A BunnyOS bootloader in Rust (`uefi` crate) is a
  long-term idea.

### Dual boot

- bunny detects Windows and other OSes itself.
- Detects BitLocker and stops before enrolling Secure Boot keys (key changes
  trigger BitLocker recovery).
- Always keeps Microsoft's keys when Windows is present.
- Clock fix: `[dualboot] clock = "windows-compatible"`.
- `keep_first = true` keeps BunnyOS first in the boot order after Windows updates.
- Installer offers "Install alongside Windows" with safety checks
  (hibernation, Fast Startup, BitLocker).

## Disk layout

```
GPT
├─ EFI   2 GB  FAT32  bootloader + signed kernels (UKIs)
└─ LUKS2 (rest)
   └─ btrfs: @ (/), @home, @log, @cache, @build (/var/tmp/portage), @snapshots
```

- zstd compression, `noatime`, async discard.
- Swap: zram by default; `hibernate = true` creates an encrypted swapfile.
- `[disk]` is applied **only at install**. Later changes are reported, never
  applied automatically.
- ext4/xfs allowed, but generations need btrfs (bunny says so).

## Secure Boot and gaming

- `secure_boot = true`: bunny checks firmware Setup Mode, creates and enrolls
  keys with `sbctl` (keeping Microsoft's keys), and signs kernels and modules on
  every update.
- Disk unlock can later use the TPM.
- **Gaming kernel from the start:** two boot entries, "BunnyOS" (your own
  kernel and keys) and "BunnyOS Gaming" (prebuilt, centrally signed, with
  lockdown, IOMMU and BPF LSM). Same system and home.
- Epic's EAC Linux v2 (announced October 2026) requires a kernel signed with
  well-known keys, TPM 2.0, Secure Boot, measured boot, IOMMU, lockdown
  (integrity), kexec disabled and BPF LSM. Being trusted needs Epic to accept
  BunnyOS's key and likely a Microsoft-signed shim. Long-term, outside our
  control. `bunny doctor gaming` checks the requirements meanwhile.

## Installer

- The USB boots straight into one fullscreen installer, with no desktop behind it.
- UI toolkit: **Slint** (Rust, draws directly to the screen via `linuxkms`, no
  compositor, software-render fallback). Plain Qt Quick is the fallback choice.
- The installer only writes a `bunny.toml`; `bunny install` does the work.
- **Layout (decided November 2026):** not the sidebar-and-steps layout
  most Linux installers share. One centered window over a dark background, a
  small top bar (installer name, a Utilities menu, language, Wi-Fi,
  battery), and **one decision per screen**: a big picture or title, a line of
  explanation, the choice, and Back / Continue at the bottom with step dots.
  Mockups: "BunnyOS: boot and installer mockups" (claude.ai artifact).
- Flow: welcome (language) → disk (whole disk or next to Windows, encryption
  on) → account → desktop → color → summary (with "Show bunny.toml") →
  installing → done.
- **Utilities menu:** "Use the terminal instead" switches to a text console
  that says: welcome to the BunnyOS installer (terminal mode), how to get back
  (`bunny-install --gui`, or Ctrl+Alt+F1), the install guide's URL ("Installing
  by hand"), and the commands to start from the Baseline config or install
  from your own. For people who want to leave the Baseline and set up
  everything themselves. Also: Reset an installed BunnyOS, Disks, logs.
- "Restore from config" rebuilds a machine from a git repo or USB.
- Installs by writing a prebuilt Baseline system image, not package by package.
- `bunny install --explain` prints a handbook for the chosen path only.

### Reset ("factory reset")

Back to the latest tested Baseline, kept up to date (not the original
install). Two kinds:

- **Reset the system, keep my files** (from the running system or the USB):
  the current system is saved as a generation first (so the reset itself can
  be undone from the boot menu), a fresh, up-to-date Baseline image becomes the
  new `@`, `/home` is untouched, the desktop and color choices are kept, and
  the old config is kept as `bunny.toml.before-reset` so things can be brought
  back one at a time.
- **Erase everything** (USB only): wipes the disk and installs fresh, for
  selling or giving a computer away. Can't be undone; asks to type ERASE.

Needs the Baseline image (phase 6 and 9).

### ISO

- **BunnyOS Net** (target ~300–400 MB) and **BunnyOS Offline** (target ~1.5 GB).
- No desktop, no Wayland, no Mesa, no Portage or Python on the USB. BusyBox
  userland is fine there.
- Without a Microsoft-signed shim, Secure Boot must be off to boot the USB.

### Hosting

- ISOs: GitHub Releases (public repo; each file under 2 GiB).
- Packages (binhost) and ISO mirror: Cloudflare R2 (no egress fees).
- Never from a home server. Always publish SHA256 sums and signatures.

## Ideas for later (not v1)

- **Portage-free systems:** Portage lives only in the build container; installed
  systems use a small Rust installer for finished packages (bash likely stays
  for package install scripts).
- Own dependency solver in Rust (e.g. `resolvo`).
- systemd as an init option.
- Own bootloader in Rust.

## Version 1 scope

v1 runs on an **existing Gentoo install (OpenRC, btrfs)**, tested in the VM
first, then on the author's desktop, laptop and ThinkPad T440p.

**In v1:**
- `bunny.toml`: packages, USE flags, services, managed config files
- `bunny.lock`
- `bunny import`, `check`, `plan`, `update`, `switch`, `status`, `diff`
- Generations as btrfs snapshots with GRUB rollback entries
- Plain-English explanations for the most common emerge errors

**Not in v1:** installer, ISO, binhost/cache, Secure Boot, gaming kernel, dual
boot handling, boot theme, musl/BusyBox, systemd, sandboxes and dev shells.

## Build order

1. Config parsing, `bunny check`, `bunny plan` (read-only, safe).
2. OpenRC service management and Portage file generation.
3. Lockfile and `bunny update`.
4. `bunny switch` with generations and GRUB entries.
5. Error translation.
6. Binary cache and `bunny serve`.
7. Boot experience (Plymouth, GRUB theme, greetd).
8. Secure Boot + gaming kernel.
9. Installer + ISO.
10. Polish.

## Upkeep

Decided November 2026. A solo-maintained distro mostly fails from maintainer
burnout, so anything weekly is automated, and the maintainer's time goes to
testing and decisions.

- **bunny:** daily sessions build, CI checks, the maintainer tests in the VM
  and merges. Releases are version tags with a changelog, built by CI and
  attached to a GitHub Release, and installed through an ebuild in
  `bunny-overlay`.
- **Keeping up with Gentoo:** a weekly scheduled CI job runs bunny in a fresh
  Gentoo container against the latest tree and opens an issue if anything
  breaks. Every new emerge message bunny gets wrong becomes a test fixture.
- **What users run (phase 6):** a tested weekly baseline, like NixOS
  channels. `bunny update` follows the published baseline, not the raw tree,
  so only tested combinations are supported.
- **Security:** `bunny status` reports Gentoo security advisories (GLSAs)
  that affect the system.
- **Dogfooding:** the maintainer runs BunnyOS on a real machine as early as
  it's safe, with their `bunny.toml` in git.

## Testing

- All changes are tested in a QEMU VM with UEFI, Secure Boot (Setup Mode),
  TPM 2.0 and virtio GPU: `tools/bunny-vm.sh`.
- Snapshot before every test; restore afterwards.
- Unit tests for everything that doesn't touch the system. Anything that writes
  to a system takes a `--root` so it can be tested against a fake root.
