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
```

## `packages`

The programs you want installed: Gentoo's **world set**. Write the full name,
`category/name`. Add `:slot` to ask for a specific
[slot](../glossary.md#slot).

You only list what you want *directly*. Whatever those need (dependencies) is
installed automatically. Removing something from the list removes it and any
dependencies nothing else needs.

`bunny add` and `bunny remove` edit this list for you.

## `services`

Services that start at boot, by their plain name (as in `/etc/init.d/`).
bunny adds them to OpenRC's `default` runlevel and starts them.

Services already started in the `boot` or `sysinit` runlevels (like
`elogind`) count as enabled. bunny doesn't turn off services you leave out of
the list. `bunny plan` shows them as "Also starting at boot, but not in your
config".

## `[system]`

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `init` | `"openrc"` | `"openrc"` | The init system. Only OpenRC for now. |
| `multilib` | `true` / `false` | `true` | 32-bit support (needed for Steam). Decided when Gentoo is installed. Changing it later needs a reinstall. |

## `[pkg."category/name"]`

Settings for one package.

| Setting | Example | Meaning |
|---|---|---|
| `use` | `["pgo", "-telemetry"]` | USE flags: turn features on (`pgo`) or off (`-telemetry`). |

Group flags (Gentoo's `PYTHON_TARGETS: python3_12` style) are written as one
word: `python_targets_python3_12`.

## `[boot]`

```toml
[boot]
undo_entry = true   # add "undo the last change" to the boot menu
```

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `undo_entry` | `true` / `false` | `false` | After each change, add a boot menu entry that undoes it, for when the system won't start. See [Undo a change](../tasks/undo.md#if-the-system-doesnt-start-at-all). |

It's off by default for now, while it's new, because it changes the startup
image. The BunnyOS installer will turn it on.

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
   checker** (`visudo -c` or `doas -C`)
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

With sudo, BunnyOS also:

- builds sudo **without `sendmail`**, which would otherwise install a mail
  server just to email root about failed password attempts. To keep it, add
  your own `[pkg."app-admin/sudo"]` entry (yours always wins).
- turns off sudo's one-time "lecture" message.

`sudo-rs` (a memory-safe rewrite of sudo) is planned once Gentoo marks it
stable.

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
- Testing (`~amd64`) versions (`/etc/portage/package.accept_keywords`)
- Config files in `/etc`
- Flatpaks

These are planned. Until then, manage them the usual Gentoo way.
