# Bunny Linux Design

> Status: draft, November 2026. This is the source of truth for what Bunny Linux is
> and what gets built in which order. Change it when a decision changes.

## The name

**Bunny Linux** (decided November 2026; it was called BunnyOS before). It
says what it is, and "BunnyOS" was already used by an unrelated AI and
crypto-token site that ranks first in search results, plus several hobby
projects. The command stays `bunny`, the shell is **Bunny Desktop**, and the
GitHub organisation and repositories keep their `bunnyos` names for now, so
links and the overlay's sync address don't break.

## The idea

Bunny Linux is a Linux distribution built **on top of Gentoo**, not from scratch.
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
  reading kept as a fallback.) **Part 1:** `helper/bunny-portage.py`,
  embedded in bunny and run as `python3 -c`, answers `plan` with the install
  list as JSON, using `_emerge`'s `parse_opts` (with EMERGE_DEFAULT_OPTS),
  `backtrack_depgraph` and the vartree, and FEATURES=getbinpkg handled like
  emerge does. For a rebuild it also says why (USE flags that changed, and
  libraries whose sub-slot changes in the plan); emerge's text gives the
  same from `--verbose`'s marked USE flags and its `r` flag. The quiet install uses it first and falls back to emerge's
  text (which also explains refusals). The weekly canary checks the helper
  agrees with emerge. **Part 3 (error explanations, one kind at a time):**
  when there's no plan, `plan` also lists `problems` from the depgraph's
  own records: `keywords` (`_needed_unstable_keywords`; `~ARCH` if the
  version has that testing keyword, else `**`), `use`
  (`_needed_use_config_changes`, flag → on/off), and `masked`
  (`_needed_p_mask_changes`, and `_unsatisfied_deps_for_display` entries
  where every matching version has a reason from
  `portage.getmaskingstatus`, with `getmaskingreason`'s comment), each with
  the chain of packages that pulled it in. The helper exits 0 for this
  answer. bunny still runs `emerge --pretend` (for the log, and for kinds
  the helper doesn't report yet: slot conflicts, blockers, missing
  packages, fetch failures), then explains each kind the helper reported
  from its data, and the rest from the text. Exception: when emerge's text
  says its USE suggestion breaks a package's REQUIRED_USE rules, the
  text's explanation is kept (the helper doesn't report those rules yet).
  Data and text use the same wording. These depgraph fields are internal
  to Portage, so each is read defensively and an empty list means "use the
  text".
- **Init:** OpenRC. systemd may be added later (see "Init systems").

## The `bunny` tool

### Config and layers

`bunny.toml` is TOML. Layers stack, each overriding the one below:

```
Bunny Linux Baseline   tested defaults, fully prebuilt
  └ your layer     personal tweaks (USE, CFLAGS, extra packages)
     └ host layer  per-machine (hosts/desktop.toml, hosts/laptop.toml)
```

**Current behaviour (host layer):** `hosts/<host name>.toml` next to the
config (host name from `/etc/conf.d/hostname`, then `/etc/hostname`, then the
kernel's) adds `packages`, `services` and `[pkg]` entries (its `[pkg]` wins),
plus a `[machine]` record from `bunny install` that's read but never acted on
(the future `[disk]` section builds on it). `bunny add`/`remove` edit only the
main file. Rollback restores the main config, its lock and this computer's
host file. The Baseline layer is still planned.

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
- **When `/boot` is its own filesystem** (encrypted installs, where it's the
  EFI partition): right after each generation's snapshot, bunny copies
  `/boot`'s kernel files into `/boot/bunny/files/` (identical files stored
  once; a rebuilt one with the same name becomes `<name>~2`) and lists the
  generation's set in `/boot/bunny/generations/<gen>`. Skipped with a note if
  it wouldn't leave 64 MB free. `bunny rollback` saves the current set first
  (refusing to change anything if it can't), then after the swap makes
  `/boot`'s kernel files exactly that generation's set (copy in, then remove)
  and runs `grub-mkconfig`. The undo entry boots the copies from `/boot`
  (`search --fs-uuid` on that partition). The boot-time undo can't touch
  `/boot`, so `bunny status` warns about kernels in `/boot` with no
  `/lib/modules` in the running system; restoring `/boot` after a boot-menu
  undo is a follow-up. `clean` drops lists of deleted generations and unused
  copies.
- **Nested subvolumes** (subvolumes inside `@`, e.g. `/srv` when tmpfiles
  creates it as a subvolume on btrfs) aren't part of snapshots. Rollback moves
  them over to the restored system, like `/home`, and `clean` deletes them
  inside-out. The installer creates `/srv` as a plain folder to avoid this.
- `clean` never removes the generation you're running a copy of.
- **The config goes back too:** after every successful switch or update (and
  a switch with nothing to do), bunny copies the config, its lock and the
  config's path to `/var/lib/bunny/applied/`, plus this computer's host file
  (`host.toml`, with its path in `host-path`; dropped when there's none). That's inside `@`, so each later
  snapshot carries the config that matches it. Rollback (and the boot-menu
  hook) copies it back over the config in the restored `@`, only for a config
  under `/etc` (one in `/home` is on its own subvolume and isn't rolled
  back). The host file goes back under the same rule; other computers' host
  files aren't touched, and older generations without a copy leave it as is.
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
  it. After each switch it writes `/boot/grub/bunny.cfg` (read via bunny's
  `/etc/grub.d/11_bunny`, right after `10_linux`'s entries; installed once
  with one `grub-mkconfig`, then read at boot so later changes need none):
  "Undo the last change" (summary and local time) plus an "Earlier versions"
  submenu (newest 10 bootable generations, each with "Go back to this
  version" and its summary as do-nothing lines), in separate markers so the
  hook only drops the undo entry. That undo block that boots the kernel and
  initramfs **from the generation's snapshot** (`/@snapshots/<gen>/boot/…`)
  with the running system's command line plus `bunny.rollback=<gen>`, so
  kernel modules match after the swap. Only added when the generation's
  initramfs is newer than its copy of the module. Limits: nested subvolumes
  aren't carried over by the boot-time swap; needs GRUB, dracut and
  `root=UUID=`. Off by default until it's had more testing; the installer
  turns it on.
- **A tidy boot menu (current behaviour):** the menu is Bunny Linux ·
  Advanced options · Undo the last change · Earlier versions · (other
  systems) · UEFI Firmware Settings. A switch with `undo_entry` on, and
  `bunny install` itself (right after the chroot step, whatever the config
  says), tidy Gentoo's leftovers, each shown in the plan and backed up first:
  `GRUB_THEME` in `/etc/default/grub` is commented out; `31_efi_bootnext` is
  made non-executable; and the main entry loses " GNU/Linux". That last one
  is `/etc/grub.d/10_bunny`, which runs GRUB's own `10_linux` (made
  non-executable) and rewrites only the `menuentry`/`submenu` titles with
  sed. Chosen over writing the entries from scratch because `10_linux`'s
  kernel discovery, microcode, recovery entries and settings keep working
  and keep up with GRUB; and over editing `10_linux` or `grub.cfg` because
  GRUB updates and every kernel update's `grub-mkconfig` would undo it. A
  GRUB update can make `10_linux` executable again (Portage restores the
  mode of an unedited file); `10_bunny` then prints nothing, so nothing is
  listed twice, and the next switch tidies it again. Everything changed is
  in `/etc`, so undo puts it back.

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
  asks the Portage helper what depclean would remove (or reads `emerge
  --depclean --pretend`'s "All selected packages" line, falling back to
  emerge's own `--ask` if the count doesn't match), and always asks. Cleanup also runs after USE changes and
  updates, since both can orphan dependencies. `--verbose` restores the old
  live, emerge-asks behaviour.
- **Ctrl+C:** stops bunny at once, except at a question during a switch
  (it counts as "no", so the usual clean-up puts the config, world file and
  package.use back) and while emerge installs (emerge stops; bunny says how
  many packages finished, which one didn't, and offers `bunny rollback` or
  `bunny switch`). A parent bunny waiting on sudo, doas or the developer-mode
  recorder ignores it, so the child can finish explaining.
- **Bunny Linux default USE flags:** packages bunny installs itself can get
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

Read-only commands never need root. (`bunny plan` as yourself can't read a
`/boot` that only root may read, like an EFI partition; it then says the
boot menu wasn't checked and to run it with sudo or doas, rather than
reporting problems it couldn't see.) Commands that change the system ask for
elevation themselves through the configured tool (see `[security]`; sudo
when nothing is set and both are installed).

### `bunny switch` (current behaviour)

1. Shows the plan and asks before changing anything (`--yes` skips this).
   Re-runs itself through sudo or doas (see `[security]`) when not root.
   If sudo or doas refuses (exit code 1, or doas stopped by Ctrl+C; bunny
   run as root never uses 1 and handles Ctrl+C itself),
   bunny says nothing was changed and to run the command again.
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
- GUI permission prompts use polkit with a Bunny Linux-styled agent.

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

**Current behaviour (repositories and testing versions, November 2026):**
`repositories = ["guru"]` in `bunny.toml` names repositories from Gentoo's
list (`api.gentoo.org/overlays/repositories.xml`). A switch writes
`repos.conf/bunny-<name>.conf` (git, auto-sync) and runs `emaint sync` right
after the snapshot, then checks packages; repositories set up outside bunny
are reported, not touched. `"bunny"` (Bunny Linux's own overlay,
`bunny-overlay`) is built in: bunny knows its git address, so it needs no
lookup and gets no trust warning. The starter config from `bunny install`
lists it and `sys-apps/bunny-release` (Bunny Linux's `/etc/os-release` and
fastfetch logo); the first switch inside the new system installs it, since
the install script's own emerge only sees Gentoo's tree. `[pkg] keywords` go in
`package.accept_keywords/zz-bunny`. `bunny add` has a short list of
well-known packages outside `::gentoo` (Mango in GURU, Hyprland in
hyproverlay) and offers their repository and keywords. Custom repository
addresses (not in Gentoo's list) are later work.

## Desktop

Decided November 2026. The installer offers, in this order:

1. **Bunny Desktop** (the selling point, recommended): Hyprland with
   Bunny Linux's own Quickshell shell that works like a full desktop (settings,
   app launcher, notifications, file and Wi-Fi panels), so someone who never
   wants to open a terminal doesn't have to.
2. **KDE Plasma**: a full, familiar desktop (works on OpenRC with elogind).
3. **Xfce**: light and classic.
4. **Hyprland, set it up yourself**: plain Hyprland and a terminal.
5. **MangoWM, set it up yourself.**
6. **No desktop**: terminal only.

GNOME leans heavily on systemd, so it isn't offered while OpenRC is the only
init system. Bunny Desktop is the one that gets the polish and testing;
the others are packaged and supported, the "set it up yourself" ones on
purpose bare. Config: `[desktop] name = "bunny" | "plasma" | "xfce" |
"hyprland" | "mango" | "none"`, `accent = "#…"`.

**Current behaviour (`[desktop] wayland = true`, November 2026):** the base
a Wayland desktop needs on the minimal install, found in the dry run:
`sys-auth/pambase` and `sys-auth/seatd` with `USE=elogind` (logins register
with elogind, which makes `XDG_RUNTIME_DIR`; libseat asks elogind for the
seat), `sys-auth/elogind` and `sys-apps/dbus` in the world set, and the
`elogind` (boot runlevel) and `dbus` services. bunny adds them as if they
were in the file (`Config::use_flags`, `wanted_services`, `wanted_world`),
marks them "(from [desktop])" in `plan` and `diff`, and the user's own
`[pkg]` entry for one of those packages wins. After a switch that changes
pambase it says to log in again. The future `name` implies `wayland = true`
for the Wayland desktops.

Why per-package flags, not `USE="elogind"` in make.conf or Gentoo's desktop
profile: the profile (`targets/desktop`) turns on about 70 global flags
(`elogind` among `pulseaudio`, `cups`, `qt6`, `X`, …), and a global flag
rebuilds every installed package that has it; both are against the minimal
install, and bunny manages neither make.conf nor the profile (a profile
switch is a big, separate change). Per-package flags change only what a
session needs, show up in `plan` like any other `[pkg]` setting, and leave
room for the user to override one. The cost: packages added later that
also have an `elogind` flag don't get it automatically; the list can grow
as real desktops need more (and `name = "plasma"` will bring its own).

## Look and feel

Decided November 2026. Mockups: "Bunny Linux: boot and installer mockups"
(claude.ai artifact), which covers the boot screens, installer and desktop.

The idea: Apple hides the system; Bunny Linux shows it calmly, and only when you
ask. The calm, centered, one-thing-at-a-time feel is kept. What's ours is
that your system is a readable file, every change is saved, and anything can
be undone, and the design says so.

- **Black and monochrome.** True black background (`#000000`), dark greys
  for surfaces (`#0B0B0D`, `#0D0D0F`, `#111113`), hairline borders
  (`#1C1C1F`, `#26262A`), warm white text (`#F2F1EE`), muted text
  (`#9A9AA0`), faint text (`#5C5C63`). No colored wallpapers behind the
  boot screens or installer.
- **Color means something.** One accent, used sparingly: the focused window,
  the main button, the current item. Default is **Paper** (the off-white, so
  the default look is fully monochrome); choices are Amber `#F5B85C`, Mint
  `#8BE0C1`, Lilac `#C6B6FF`, Coral `#FF8E7A`. Status colors: amber for
  "needs attention", coral for "can't be undone". Not Apple blue.
- **Two type voices.** Geist for words; Geist Mono for anything that is
  "the system": file names, versions, times, config lines, key hints.
- **Centered by default, under the hood on request.** Screens are centered,
  one decision at a time. A `{ } bunny.toml` button (installer) or
  `{ } under the hood` (Settings) slides a panel in from the right showing
  the file your choices write, with the current change highlighted. It is
  never needed, and closed by default.
- **Undo is visible.** The boot menu, Settings → History and notifications
  ("Saved a version first, so you can go back. Undo") all show it.
- **Glass, barely.** Flat surfaces. A soft shadow only on things that float
  (menus, the launcher, notifications). No live blur needed anywhere, so the
  same look works in GRUB, Plymouth, the installer and the desktop.
- **Motion:** short and quiet (about 0.4 s, ease-out). Panels slide; nothing
  bounces.
- **Logo:** the placeholder bunny until a designer makes the real one.

### Staying light

The look must not cost speed. Targets (not yet measured):

- Bunny Desktop idle memory under 600 MB; Settings opens in under 300 ms.
- Power button to login screen under 10 seconds on an NVMe disk.
- Boot screens are static images plus one progress bar: no video, no blur.
- The shell is one Quickshell process; no background services that only
  exist to look nice.
- Every visual feature has an off switch in `[desktop]`, and the "set it up
  yourself" desktops ship none of it.

## Boot experience

```
power on → hidden GRUB (hold Esc for the Bunny Linux-themed menu)
         → Plymouth boot animation (Esc shows the kernel log)
         → password prompt inside Plymouth (if encrypted)
         → greetd + Quickshell greeter
```

- `quiet splash loglevel=3`, early KMS, and `GRUB_GFXPAYLOAD_LINUX=keep` for a
  flicker-free handoff.
- GRUB is the default loader with a real Bunny Linux theme, hidden by default.
- **The menu is grouped:** *Start* (Bunny Linux, Bunny Linux Gaming, other systems
  found), *Go back to* (recent generations named by what changed, e.g.
  "Before you added mpv · today 09:15", then "Earlier versions"), *Other*
  (firmware settings). A *History* timeline beside it shows each saved
  change. Starting an older version never deletes anything newer.
- **Loading screen:** black, the white logo, one thin progress line, and a
  faint "esc · details". Esc shows the startup log; a failed service gets a
  plain-English note under the log.

```toml
[boot]
loader = "grub"
menu = "hidden"        # "hidden" | "always" | "timeout"
theme = "bunny"
default = "last-used"
kernel_args = ["quiet", "splash"]
```

- **aarambh** (graphical UEFI menu) can become an option if it stays
  maintained or is forked. A Bunny Linux bootloader in Rust (`uefi` crate) is a
  long-term idea.

### Dual boot

- bunny detects Windows and other OSes itself.
- Detects BitLocker and stops before enrolling Secure Boot keys (key changes
  trigger BitLocker recovery).
- Always keeps Microsoft's keys when Windows is present.
- Clock fix: `[dualboot] clock = "windows-compatible"`.
- `keep_first = true` keeps Bunny Linux first in the boot order after Windows updates.
- Installer offers "Install alongside <the system it found>" (Windows, macOS
  on Intel Macs, another Linux, etc.), with safety checks for that system
  (for Windows: hibernation, Fast Startup, BitLocker).

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
- **Gaming kernel from the start:** two boot entries, "Bunny Linux" (your own
  kernel and keys) and "Bunny Linux Gaming" (prebuilt, centrally signed, with
  lockdown, IOMMU and BPF LSM). Same system and home.
- Epic's EAC Linux v2 (announced October 2026) requires a kernel signed with
  well-known keys, TPM 2.0, Secure Boot, measured boot, IOMMU, lockdown
  (integrity), kexec disabled and BPF LSM. Being trusted needs Epic to accept
  Bunny Linux's key and likely a Microsoft-signed shim. Long-term, outside our
  control. `bunny doctor gaming` checks the requirements meanwhile.

## Installer

- The USB boots straight into one fullscreen installer, with no desktop behind it.
- UI toolkit: **Slint** (Rust, draws directly to the screen via `linuxkms`, no
  compositor, software-render fallback). Plain Qt Quick is the fallback choice.
- The installer only writes a `bunny.toml`; `bunny install` does the work.
- **Layout (decided November 2026):** not the sidebar-and-steps layout
  most Linux installers share. Full-screen black with a centered column, a
  small top bar (installer name, a Utilities menu, language, Wi-Fi, battery,
  and the `{ } bunny.toml` button), and **one decision per screen**: a step
  counter (`02 / 07  REGION`), a big title, a line of explanation, the
  choice, and Back / Continue at the bottom. See "Look and feel".
- **bunny.toml panel:** hidden by default. When opened it slides in from the
  right, the content moves aside, and the file fills in section by section
  as you answer, with the current section highlighted. The summary links to
  it ("See the file this creates").
- Flow: welcome (language) → region → disk (whole disk or alongside the
  system it found, encryption on) → account → desktop → color → summary →
  installing → done.
- **Region:** a dotted world map. Clicking a city sets the country, time
  zone and keyboard; scrolling, pinching or +/− zooms in and shows more
  cities; dragging moves the map. Choosing a country or time zone from the
  lists below moves the pin there. Shows the local time and coordinates.
- **Utilities menu:** "Use the terminal instead" switches to a text console
  that says: welcome to the Bunny Linux installer (terminal mode), how to get back
  (`bunny-install --gui`, or Ctrl+Alt+F1), the install guide's URL ("Installing
  by hand"), and the commands to start from the Baseline config or install
  from your own. For people who want to leave the Baseline and set up
  everything themselves. Also: Reset an installed Bunny Linux, Disks, logs.
- "Restore from config" rebuilds a machine from a git repo or USB.
- **Partitioning, three ways** (decided November 2026):
  1. **Guided** (default, fully supported): pick a disk; the standard layout
     with encryption. Common changes as options: no encryption, sizes,
     hibernation swapfile, `/home` on a second disk, alongside another OS.
  2. **From the config:** a `[disk]` section describes the layout (like
     NixOS's disko) and `bunny install` partitions to match.
  3. **Manual:** partition and mount at `/mnt` yourself, then `bunny install
     --mounted /mnt`. bunny never touches the disks, reads what's mounted,
     and explains anything that limits it (ext4: no undo; btrfs without
     subvolumes: offers to add them). Allowed and checked, not every layout
     tested.
- **The config is written for you, but it's yours.** Every mode writes a
  starter `bunny.toml` (like `nixos-generate-config`). `--edit` stops to edit
  it before installing; `--config <file or repo>` installs from your own.
  Disk details go in the host file (`hosts/<host>.toml`), like NixOS's
  `hardware-configuration.nix`, so the main config works on any machine.
- **One step from the config (later, needs phase 6):** `bunny install
  --config github:you/my-system` partitions, writes the Baseline image and
  applies the config and lock in one command. Same recipe, not bit-for-bit
  (see "Lockfile").
- **Current behaviour (part 1, guided):** `bunny install` lists whole disks
  from `lsblk --json` and leaves out any in use (anything mounted, swap, or
  something opened on it like LUKS or LVM), read-only or under 20 GB. After
  the disk, "Use the recommended setup? [Y/n]" (`install::questions`):
  Enter takes it (encrypted, zram swap); `n` asks encrypt ([Y/n]) and swap
  (Enter zram; a size for zram plus a swap file; `hibernate`; `none`). An
  option answers its question (`--no-encrypt`; `--swap`, `--hibernate` or
  `--no-zram`). zram is bunny's own small OpenRC service (`zram`, boot
  runlevel), not sys-block/zram-init (its `maxs` setting breaks on kernels
  from 6.15): as big as memory up to 8 GB like Fedora, zstd when the kernel
  has it, priority 100 so it's used before a swap file. `/home` on a second
  disk isn't asked yet (on hold; `--home-disk` works). No partition sizes,
  mount points or filesystem choice: btrfs is needed for undo, and full
  control is `--mounted`. The preview then names the disks left untouched
  and, when the answers differ from the recommended setup, the options that
  repeat them (without `--disk`, which differs per computer). The
  preview lists every partition that will be erased and the layout you get;
  you confirm by typing the disk's name. Layout: GPT, EFI (2 GB, FAT32) at
  `/boot`, the rest LUKS2 (opened as `bunny-root`) with btrfs subvolumes
  `@ @home @log @cache @build @snapshots` (+ `@swap` with a swapfile).
  `/boot` on the EFI partition means kernels sit outside `@`, so undo relies
  on the kernel copies in `/boot/bunny/`. Then: the newest OpenRC stage3
  (SHA512-checked), fstab, Portage settings with signed binary packages, a
  dracut config, the sudo file from `security::render`, and an embedded
  `inside.sh` run in a chroot (packages from the binhost, GRUB named
  "Bunny Linux" plus the fallback path, the kernel's startup image, the
  user). One password unlocks the disk and logs in; root stays locked.
  The password can't be empty; under 8 characters gets a warning (easy to
  guess, a few words are better) and can be kept by typing it again, or
  replaced by pressing Enter. It's your disk and your choice.
  `--home-disk` encrypts a second disk with the same password plus a random
  key file in `/etc/bunny/keys`, unlocked by OpenRC's dmcrypt. Passwords go
  only to the tools' standard input, never to files or the screen. On a
  failure everything is unmounted and closed, so a second run starts clean.
  `--dry-run` doesn't ask for the password (it stands in a placeholder that
  nothing uses) and shows each step's name and what it does in plain words
  (StepsRunner); `--dry-run --verbose` prints every command and file
  (PrintRunner).
- **Your config's history (current behaviour):** after the first switch,
  `bunny install` makes the new `/etc/bunny` a git repository (`git init -b
  main`, run with the USB's git), with a repository-only identity from the
  install's answers (the user name and a placeholder `<user>@<host>.local`),
  a `.gitignore` that keeps out `keys/` (the `/home` disk's key) and leftover
  copies, and a first commit "As installed". A `--config` repository keeps
  its own history. Failures are explained and never stop the install.
  Either way it adds `/etc/bunny` to git's system-wide `safe.directory`
  (a `[safe]` section appended to the new system's `/etc/gitconfig`,
  written directly rather than with the USB's `git config --system`, and
  only if no `[safe] directory` line already covers it or `*`): the folder
  belongs to root, and git refuses to read it as another user ("detected
  dubious ownership"). `bunny publish` does the same, so older installs get
  it, and its preview says so.
- **`bunny publish [repo]` (current behaviour):** sends `/etc/bunny` to
  GitHub with the invoking user's own `gh` sign-in (`SUDO_USER`/`DOAS_USER`;
  refused when root logged in directly). It starts the history if there's
  none, makes the repository private if it doesn't exist (`my-system` by
  default), sets a repository-only credential helper `!runuser -u <user> --
  env HOME=… gh auth git-credential` (after clearing any helper root has), so
  root's git asks the user's gh each time and no token is copied, adds
  `origin`, commits uncommitted changes ("Saved with bunny publish") and
  pushes. It shows the steps and asks first (a public repository gets a
  warning and defaults to No); `--dry-run` reads for real and prints the
  rest. Named `publish` rather than `git connect` because the same command
  is what you run after every change, not only once.
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

Needs the Baseline image (phases 6 and 7).

### ISO

- **Bunny Linux Net** (target ~300–400 MB) and **Bunny Linux Offline** (target ~1.5 GB).
- No desktop, no Wayland, no Mesa, no Portage or Python on the USB. BusyBox
  userland is fine there.
- **Current behaviour (the first, minimal image):** `tools/build-iso.sh`
  builds it from Gentoo's stage3 with prebuilt packages: gentoo-kernel-bin,
  linux-firmware, the disk tools, dhcpcd + iwd, chrony, git and bunny.
  dracut's `dmsquash-live` boots a squashfs root (`root=live:CDLABEL=BUNNY`,
  overlayfs for changes); `grub-mkrescue` makes a hybrid ISO that's also a
  USB image. It logs in as root on tty1 and shows a welcome (`welcome`).
  **Console font** (also set by `bunny install`): the Linux console's usual
  font has no ✓ or ✗ (they showed as `v`, `x`), and Terminus's 512-glyph
  console fonts don't have them either (only its full, 1356-glyph BDF
  does). kbd's `eurlatgr` (part of every Gentoo system) has ✓ ✗ · but no
  ⚠, so both scripts copy it to `/usr/local/share/consolefonts/bunny.psfu`
  with ⚠ added to its `!` glyph (`psfgettable`/`psfaddtable`), and set it in
  `/etc/conf.d/consolefont` with the `consolefont` service in the boot
  runlevel. The ISO build checks the font has all four marks. A real ⚠
  glyph would need our own font (later, with the logo work).
  Portage and Python are still in it (removing them is later work), so it's
  bigger than the target; the package tree, docs and kernel sources are
  removed.
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
6. A first, minimal ISO: terminal installer (`bunny install`), no desktop,
   so Bunny Linux can be installed from scratch and learned from the Guide.
7. Binary cache and `bunny serve`.
8. Graphical installer, Baseline image, Bunny Desktop, full ISO.
9. Secure Boot + gaming kernel.
10. The look: boot experience (Plymouth, GRUB theme, greetd), polish.

(Reordered November 2026: function first, and a minimal ISO early so the
maintainer can learn Bunny Linux as a new user would.)

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
- **Dogfooding:** the maintainer runs Bunny Linux on a real machine as early as
  it's safe, with their `bunny.toml` in git.
- **Developer mode (`[dev] report`):** testers' machines record every bunny
  command with util-linux `script` (terminal output exactly as seen, without
  changing behaviour), keeping recordings in `~/.local/state/bunny/sessions/`.
  `bunny report` cleans them (colour codes, redrawn progress lines),
  swaps user names, home folders, the hostname, emails and `*.ts.net` names
  for placeholders, and files one issue in the private
  `bunnyos-dev/bunny-reports` with the user's own `gh` login (REST API).
  `"auto"` sends any command that fails or shows ⚠/✗, except ones that
  stopped because of the user's own input: those exit with code 4
  (`YOUR_INPUT`: a package name that doesn't exist, config mistakes, sudo or
  doas getting no password, Ctrl+C at its question included), which bunny
  as root passes through to the bunny that elevated. They're still recorded
  and sent by a manual `bunny report`. Other Ctrl+C stops are still sent,
  since a "hang" the user had to stop can be bunny's problem. Daily sessions read
  new reports first.
- **Recording the install:** the live USB has no config, so preview images
  (CI builds that aren't releases, `PREVIEW=1 tools/build-iso.sh`) ship
  `/etc/bunny-preview.toml` with `[dev] report = true`, which bunny uses when
  the config doesn't turn recording on. Just before it unmounts, a recorded
  `bunny install` copies its recording so far, with notes (exit code 0), to
  `/home/<user>/.local/state/bunny/sessions/` in the new system (owned by the
  user), so `bunny report` there sends it. A failed install leaves it on the
  USB. Release images record nothing.

## Testing

- All changes are tested in a QEMU VM with UEFI, Secure Boot (Setup Mode),
  TPM 2.0 and virtio GPU: `tools/bunny-vm.sh`. Each VM (`VM=<name>`) keeps
  its SSH host key in its own `known_hosts` file in its folder
  (`StrictHostKeyChecking=accept-new`), because named VMs share port 2223;
  `forget-key` clears it after a reinstall.
- Snapshot before every test; restore afterwards.
- Unit tests for everything that doesn't touch the system. Anything that writes
  to a system takes a `--root` so it can be tested against a fake root.
