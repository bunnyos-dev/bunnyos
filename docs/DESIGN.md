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
  instead of scraping emerge's text output.
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
- Each snapshot gets a note in `/.snapshots/<name>.toml` saying what came
  right after it (a switch with a short summary, or a rollback and its
  target), and `/.snapshots/bunny-state.toml` remembers the last action, so
  `bunny generations` can say what each one is and which one is running.
  `/.snapshots` is its own subvolume, so notes survive rollbacks.
- `bunny clean [--keep N]` removes old generations (keeps the newest 5 by
  default) and `@replaced-*` leftovers, never the subvolume `/` is running
  from. It shows the list first and defaults to No, because it can't be undone.
- **Next:** a GRUB "Undo last switch" entry for systems that won't boot. It
  will use the same swap, triggered early in boot (needs an initramfs step),
  because Gentoo doesn't boot cleanly from a read-only snapshot directly.

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
| `bunny status` | Show drift (anything changed outside bunny) |
| `bunny diff [--baseline]` | Show differences from the config or from the Baseline |
| `bunny why <pkg>` | Explain why a package or flag is set |
| `bunny flags <pkg>` | Explain a package's USE flags in plain English |
| `bunny pin <pkg> <ver>` | Pin a package (keeps a copy of its ebuild and binary) |
| `bunny search <term>` | Fast search from a prebuilt index |
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
privilege = "doas"     # "doas" | "sudo" | "sudo-rs"
persist = true
admins = ["you"]
```

- Steam's Linux client went 64-bit in October 2026, but Valve still requires
  32-bit libraries. bunny enables 32-bit only for the packages Steam needs.
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

- Hyprland and MangoWM, with Quickshell for the shell.
- One of them is the polished default; the other is "supported, less polished".

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
- Flow: language → Wi-Fi → disk (Erase & Install, Encrypt on) → account →
  look → summary → progress bar → reboot.
- "Advanced" reveals everything else. "Restore from config" rebuilds a machine
  from a git repo or USB. A Utilities menu holds a terminal, disk tool and logs.
- Installs by writing a prebuilt Baseline system image, not package by package.
- `bunny install --explain` prints a handbook for the chosen path only.

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

## Testing

- All changes are tested in a QEMU VM with UEFI, Secure Boot (Setup Mode),
  TPM 2.0 and virtio GPU: `tools/bunny-vm.sh`.
- Snapshot before every test; restore afterwards.
- Unit tests for everything that doesn't touch the system. Anything that writes
  to a system takes a `--root` so it can be tested against a fake root.
