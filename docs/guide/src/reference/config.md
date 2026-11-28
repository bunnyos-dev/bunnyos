# bunny.toml, every setting

The default location is `/etc/bunny/bunny.toml`. Every command takes another
path if you keep it somewhere else (`bunny plan ~/my.toml`, or
`--config ~/my.toml` for `add` and `remove`).

The file is [TOML](https://toml.io). bunny checks it with `bunny check`.

## A complete example

```toml
# Top-level settings first (see "The TOML section trap" below).
packages = [
    "app-editors/neovim",
    "net-misc/freerdp:3",       # a specific slot
    "www-client/firefox",
]

services = [
    "NetworkManager",
    "sshd",
]

[system]
init = "openrc"
multilib = true

[pkg."www-client/firefox"]
use = ["pgo", "wayland", "-telemetry"]

[pkg."dev-lang/python:3.12"]
use = ["sqlite", "python_targets_python3_12"]

[desktop]
wayland = true
```

## `packages`

The programs you want installed: Gentoo's **world set**. Write the full name,
`category/name`. Add `:slot` to ask for a specific
[slot](../glossary.md#slot).

You only list what you want *directly*. Whatever those need (dependencies) is
installed automatically. Removing something from the list removes it and any
dependencies nothing else needs.

`bunny add` and `bunny remove` edit this list for you.

Two things are kept even if they're not in the list:

- the admin tool chosen in [`[security]`](#security) (sudo or doas),
- what [`[desktop]`](#desktop) needs (elogind and D-Bus), and
- bunny itself, when it was installed from the Bunny Linux overlay
  (`app-admin/bunny-bin` or `app-admin/bunny`), so a switch never removes
  bunny.

## `services`

Services that start at boot, by their plain name (as in `/etc/init.d/`).
bunny adds them to OpenRC's `default` runlevel and starts them. The one
exception is `elogind`, which goes in the `boot` runlevel, as Gentoo
recommends, so even the first login goes through it.

Services already started in the `boot` or `sysinit` runlevels (like
`elogind`) count as enabled. bunny doesn't turn off services you leave out of
the list. `bunny plan` shows them as "Also starting at boot, but not in your
config".

## `repositories`

Package repositories (overlays) besides Gentoo's own, by the name Gentoo's
[list of repositories](https://repos.gentoo.org) gives them:

```toml
repositories = ["guru"]
```

On the next `bunny switch`, bunny looks the repository up in Gentoo's list,
writes `/etc/portage/repos.conf/bunny-guru.conf`, and downloads it with git,
before checking and installing packages, so a package from it can be added
in the same switch. `bunny update` keeps it up to date with the rest, and
`bunny.lock` records its version like Gentoo's own.

While it downloads you see one line per repository:

```
  ✓ Downloaded guru · 17.5 MB
```

The download tool's own output (git's lines, Portage's "Performing Global
Updates") is saved in `/var/log/bunny/<time>-download-<name>.log`. If the
download fails, bunny shows the end of that output and where the log is.
With `--verbose`, the full output is shown as it comes instead.

> **Only add repositories you trust.** Gentoo's developers don't check
> them, and their packages run their build steps as root. GURU is run by a
> Gentoo project (users look after it together); most others are run by
> people outside Gentoo.

- Taking a repository out of the list removes bunny's `repos.conf` file for
  it. Its files stay in `/var/db/repos/<name>`, and packages installed from
  it stay installed but get no more updates.
- Repositories set up another way (like `eselect repository`) are left
  alone. `bunny plan` lists them as "not in your config".
- `bunny add` knows where a few well-known packages went (Hyprland,
  Mango), and offers to add their repository and testing versions for you.
- Repositories must be downloadable with git (`dev-vcs/git`, which the
  installed system has).
- **`"bunny"`** is Bunny Linux's own repository
  ([bunny-overlay](https://github.com/bunnyos-dev/bunny-overlay)). It isn't
  in Gentoo's list, so bunny knows its address itself, and doesn't show the
  warning above for it. It has `sys-apps/bunny-release`, which makes the
  system say Bunny Linux (`/etc/os-release`, and the logo in `fastfetch`),
  and bunny itself. A config from `bunny install` has it already.

Like `packages`, put `repositories` above the first `[section]`.

## `[system]`

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `init` | `"openrc"` | `"openrc"` | The init system. Only OpenRC for now. |
| `multilib` | `true` / `false` | `true` | 32-bit support (needed for Steam). Decided when Gentoo is installed. Changing it later needs a reinstall. |
| `profile` | `"minimal"` / `"desktop"` | not set (left as it is) | Gentoo's defaults. `"minimal"` is Gentoo's plain profile: only what a system needs is on. `"desktop"` is Gentoo's desktop profile: what most desktops need (Wayland, X, sound, D-Bus, …) is on by default, so installing a desktop needs fewer settings. Gentoo prebuilds packages for both. Changing it shows `Profile: minimal → desktop` in the plan; packages whose features change are downloaded again or rebuilt, and undo puts the old profile back. A system on another profile (hardened, plasma, systemd, …) is left alone, with a note. |

## `[pkg."category/name"]`

Settings for one package.

| Setting | Example | Meaning |
|---|---|---|
| `use` | `["pgo", "-telemetry"]` | USE flags: turn features on (`pgo`) or off (`-telemetry`). |
| `keywords` | `["~amd64"]` | Versions to accept beyond stable ones. `"~amd64"` allows testing versions (most work fine; Gentoo just hasn't declared them stable yet). `"**"` allows any version, including unreleased ones from git: avoid it unless you mean it. |
| `prebuilt` | `false` | Always compile this package instead of downloading Gentoo's prebuilt copy. Leave it out normally: bunny downloads a prebuilt copy whenever one was built with your flags. bunny sets it itself only when every prebuilt copy gets in the way (see [When emerge needs settings changed](../tasks/install.md#when-emerge-needs-settings-changed)). |

Group flags (Gentoo's `PYTHON_TARGETS: python3_12` style) are written as one
word: `python_targets_python3_12`.

`keywords` go in bunny's own `/etc/portage/package.accept_keywords/zz-bunny`.
Other files in that folder are left alone (and a `package.accept_keywords`
that's a single file is moved into the folder as `00-yours`, unchanged).
When emerge says a package is only available as a testing version, bunny's
explanation shows the exact lines to add.

## `[boot]`

```toml
[boot]
undo_entry = true   # "Undo the last change" and "Earlier versions" in the boot menu
```

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `undo_entry` | `true` / `false` | `false` | After each change, add a boot menu entry that undoes it, for when the system won't start. See [Undo a change](../tasks/undo.md#if-the-system-doesnt-start-at-all). |

It's off by default for now, while it's new, because it changes the startup
image. The Bunny Linux installer will turn it on.

## `[desktop]`

What a graphical desktop needs from the base system. Bunny Linux installs a
minimal system where logins don't go through
[elogind](https://wiki.gentoo.org/wiki/Elogind), and a Wayland desktop
(Mango, Sway, Hyprland, …) needs them to: without it, the desktop says
"XDG_RUNTIME_DIR must be set" or "libseat: No backend was able to open a
seat". One line sets it up:

```toml
[desktop]
wayland = true
```

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `wayland` | `true` / `false` | `false` | Set up elogind logins and seats, which any Wayland desktop needs. |

It's as if you'd written these yourself:

```toml
packages = ["sys-apps/dbus", "sys-auth/elogind"]
services = ["dbus", "elogind"]      # elogind in the boot runlevel

[pkg."sys-auth/pambase"]
use = ["elogind"]    # logins register with elogind, which makes /run/user/<id>

[pkg."sys-auth/seatd"]
use = ["elogind"]    # libseat asks elogind for the screen and keyboard
```

`bunny plan` and `bunny diff` mark each of these "(from [desktop])". Your own
settings win: a `[pkg."sys-auth/seatd"]` entry of yours replaces the one
`[desktop]` would add, and listing `elogind` in `services` too is fine.

Set this up by hand before (the `[pkg]` lines and services above)? Those
lines can stay, or you can replace them with the section: `bunny plan` then
says there's nothing to do.

After the switch that turns it on, **restart the computer once**: the seats
a desktop takes over (screen, keyboard) are set up at startup, and logins
from before don't go through elogind. bunny reminds you.

It doesn't install a desktop itself; add the one you want with `bunny add`.
Choosing a whole desktop in this section (`name = "mango"`, Bunny Desktop,
KDE Plasma, Xfce) is planned.

Why only these packages, and not `USE="elogind"` for everything or Gentoo's
desktop profile: both rebuild every installed package with that flag (the
profile also turns on about 70 other flags, like `pulseaudio`, `cups` and
`qt6`), which isn't what a minimal system wants. Some programs you add later
have an `elogind` flag of their own; if one needs it, turn it on in that
program's `[pkg]` entry.

## `[hardware]`

This computer's graphics cards and CPU features: Gentoo's `VIDEO_CARDS` and
`CPU_FLAGS_X86`. Usually in the [host file](#host-files); `bunny hardware`
suggests both. A task-by-task walk-through is in
[Graphics cards and CPU features](../tasks/hardware.md).

```toml
[hardware]
video_cards = ["intel", "nvidia"]
cpu_flags = ["aes", "avx", "avx2", "sse4_2"]
```

| Setting | Default | What it does |
|---|---|---|
| `video_cards` | not managed | Gentoo's `VIDEO_CARDS` values (`intel`, `nvidia`, `amdgpu`, `radeonsi`, `nouveau`, `virgl`…), the **whole list**: bunny writes `*/* VIDEO_CARDS: -* intel nvidia`, so Gentoo's defaults don't apply. `[]` means no graphics drivers (`-*` alone). |
| `cpu_flags` | not managed | Gentoo's `CPU_FLAGS_X86` values, as `cpuid2cpuflags` prints them, **added to** Gentoo's defaults: `*/* CPU_FLAGS_X86: aes avx …`. `[]` writes nothing. |

- Values are lowercase letters, digits and `_`; `bunny check` finds typos in
  that shape and names listed twice, not values Gentoo doesn't know.
- The lines go first in bunny's package.use file
  (`/etc/portage/package.use/zz-bunny`), so a package's own `[pkg]` flags
  (`video_cards_nvidia`, say) still win. make.conf isn't changed.
- A setting that's left out isn't managed: bunny writes no line for it, and
  lines of your own in package.use keep working (they're kept word for word
  in `00-bunny-kept`). Once `[hardware]` sets it, your line moves to the
  backup and bunny's replaces it. Taking a setting out of `[hardware]` again
  removes bunny's line, so Gentoo's defaults apply.
- In the host file, each setting wins over the main config's on its own:
  a host file with only `video_cards` keeps the main config's `cpu_flags`.
- `bunny plan` shows a change as
  `~ graphics cards (VIDEO_CARDS): nvidia → intel nvidia`; a list that adds
  to Gentoo's defaults reads `Gentoo's defaults + …`. `bunny diff` and
  `bunny status` count it too. The switch rebuilds what uses them
  (`emerge --newuse`), which can mean compiling packages that would otherwise
  be downloaded prebuilt.
- `bunny import` reads `*/* VIDEO_CARDS:` and `*/* CPU_FLAGS_X86:` lines into
  `[hardware]`, and suggests make.conf's `VIDEO_CARDS="…"` and
  `CPU_FLAGS_X86="…"` as comments.

## `[dev]`

Developer mode, for people testing Bunny Linux: records every command and sends
reports to the developers. Off unless you add it.

```toml
[dev]
report = "auto"   # false | true | "auto"
name = "laptop"
```

Every setting is explained in [Developer mode](developer-mode.md).

## `[security]`

Which tool gives admin rights, and who gets them. bunny only manages this if
the section is there; `bunny import` writes it commented out, with what it
found on your system.

```toml
[security]
privilege = "sudo"   # "sudo" (the default) or "doas"
persist = true       # remember your password for a few minutes
admins = ["you"]     # users who get admin rights
```

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `privilege` | `"sudo"` / `"doas"` | `"sudo"` | The admin tool. bunny installs it. |
| `persist` | `true` / `false` | `true` | Remember your password for a few minutes after typing it. |
| `admins` | user names | none | Added to the `wheel` group, which is what gets admin rights. |

On a switch, bunny:

1. installs the tool (it's added to your packages automatically)
2. writes `/etc/sudoers.d/bunny` (sudo) or `/etc/doas.conf` (doas) giving the
   `wheel` group admin rights, **after checking it with the tool's own
   checker** (`visudo -c` or `doas -C`). doas keeps all its settings in that
   one file, so bunny writes its rule first and keeps every other line you
   had after it (the plan says how many); doas uses the last rule that
   matches, so your own rules, like `nopass` for one command, still apply.
   The old file is also saved in the backup folder.
3. adds `admins` to the `wheel` group

Safety:

- bunny **never removes anyone** from `wheel`.
- If nobody would be in `wheel`, or an admin's name isn't a real user, bunny
  stops before changing anything, because that could lock everyone out.
- If the checker rejects the file, it's not put in place, and no packages are
  removed, so the tool you have keeps working.
- Switching from doas to sudo (or back) installs and sets up the new one
  before the old one can be removed. Take the old one out of `packages` once
  you're happy.

With sudo, Bunny Linux also:

- builds sudo **without `sendmail`**, which would otherwise install a mail
  server just to email root about failed password attempts. To keep it, add
  your own `[pkg."app-admin/sudo"]` entry (yours always wins).
- turns off sudo's one-time "lecture" message.

`sudo-rs` (a memory-safe rewrite of sudo) is planned once Gentoo marks it
stable.

## Host files

A file named after a computer, in a `hosts/` folder next to `bunny.toml`,
holds what belongs to that computer only:

```
/etc/bunny/
├── bunny.toml          everything you choose, the same on every computer
├── bunny.lock
└── hosts/
    ├── laptop.toml     read only on the computer named laptop
    └── desktop.toml
```

When bunny runs on `laptop`, it reads `bunny.toml`, then
`hosts/laptop.toml` on top. `bunny check` says when it used one:
`✓ /etc/bunny/bunny.toml (with /etc/bunny/hosts/laptop.toml, for this computer) looks good.`
The computer's name comes from `/etc/conf.d/hostname`.

**A copy somewhere else** (say `~/test/bunny.toml`, to try a change with
`bunny plan ~/test/bunny.toml`) usually has no `hosts/` folder next to it.
Then bunny uses this computer's host file from `/etc/bunny/hosts/` and says
so:

```
✓ /home/you/test/bunny.toml (with this computer's /etc/bunny/hosts/laptop.toml, since there's no hosts/ folder next to it) looks good.
```

Without it, everything only this computer has (GRUB, the kernel, firmware)
would show as "would be removed", and a switch from that copy would remove
them. If the copy has its own `hosts/` folder, bunny uses only that folder;
when there's no file for this computer in it, it says this computer's host
file isn't included, and what that means:

```
  ⚠ This computer's host file (/etc/bunny/hosts/laptop.toml) isn't included: /home/you/test/hosts has no laptop.toml.
```

A host file can have:

| Setting | What it does |
|---|---|
| `packages` | Added to the main list. A package in both is fine. |
| `services` | Added to the main list. |
| `[pkg."category/name"]` | USE flags for this computer. For a package in both files, the host file's entry wins. |
| `[hardware]` | Graphics cards and CPU features (see [`[hardware]`](#hardware)). Each setting here wins over the main config's. |
| `[machine]` | A record of how `bunny install` set up the disk: `disk`, `encrypted`, `efi_uuid`, `root_uuid`, `luks_uuid`, `home_disk`, `swap`, `hibernate`, `timezone`, `keymap`. bunny reads it but never changes disks from it. |

`bunny install` writes one with what the computer needs to start (the kernel,
firmware, GRUB, filesystem tools, `cryptsetup` for an encrypted disk) and the
`[machine]` record. `bunny add` and `bunny remove` change `bunny.toml`, not
the host file; edit the host file by hand.

Undo puts this computer's host file back too, along with `bunny.toml` and
`bunny.lock` (see [The config goes back too](generations.md#the-config-goes-back-too)).

## The TOML section trap

In TOML, **everything below a `[section]` header belongs to that section**.
So this is wrong:

```toml
[system]
init = "openrc"
packages = ["app-misc/tmux"]   # ✗ this is now [system].packages
```

Put `packages` and `services` **above** the first `[section]`. bunny spots this
mistake and tells you how to fix it.

## What bunny.toml doesn't cover yet

- Global USE flags and other settings in `/etc/portage/make.conf` (left as
  they are)
- Config files in `/etc`
- Flatpaks

These are planned. Until then, manage them the usual Gentoo way.
