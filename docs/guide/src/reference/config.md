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
