# The lockfile and version pins

## What's in bunny.lock

`bunny.lock` sits next to your config (`bunny.toml` → `bunny.lock`). bunny
writes it; don't edit it by hand.

```toml
version = 1

[repos.gentoo]
timestamp = "Mon, 16 Nov 2026 22:15:00 +0000"

[packages]
"app-misc/tmux" = "3.6a"
"dev-lang/python:3.12" = "3.12.8_p1"
"dev-lang/python:3.13" = "3.13.2"
"sys-libs/glibc" = "2.42-r1"
```

- **`[repos]`**: each package repository and the snapshot it was at: the git
  commit (git-synced repositories) or the sync time (rsync/webrsync).
- **`[packages]`**: the exact version of **every** installed package, not just
  the ones in your config. Packages installed in several
  [slots](../glossary.md#slot) are listed per slot.

## How the lock holds your system in place: pins

bunny turns the lock into **pins**: one Portage mask per package that forbids
anything *newer* than the locked version. They live in one file:

```
/etc/portage/package.mask/zz-bunny-lock
```

```
>app-misc/tmux-3.6a
>dev-lang/python-3.12.8_p1:3.12
```

So emerge can't move past the lock, and if a package somehow became newer, the
next switch puts it back.

## When the lock changes

| Command | Effect on the lock |
|---|---|
| `bunny switch` / `add` / `remove` | Locked packages stay put. New packages are added to the lock, removed ones taken out. |
| `bunny update` | Removes the pins, updates everything, then writes the new versions and pins. If the update fails, the old pins are put back. |
| `bunny lock` | Accepts the system as it is right now. |
| `bunny plan` | Doesn't change it; shows differences. |

## Same recipe, not bit-for-bit

The same lock gives the **same versions, USE flags and settings** on every
machine. If packages are compiled locally, the binaries can still differ
slightly (build times, CPU optimisations). NixOS goes further and makes builds
identical; bunny aims for "same recipe", which is what matters day to day.

## Limits

- Gentoo removes old versions from its tree after a while. A lock that asks for
  a version that's gone can't be installed on a new machine; `bunny update`
  moves past it.
- A locked version that has a security problem stays installed until you run
  `bunny update`. Update regularly.
