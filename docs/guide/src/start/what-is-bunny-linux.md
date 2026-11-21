# What Bunny Linux is

Bunny Linux is a Linux system built on **Gentoo**, with a tool called **bunny**
that makes it easier and safer to use.

## What it keeps from Gentoo

- **OpenRC** as the init system.
- **USE flags**: you choose which features each program is built with.
- **Compiling for your own computer**, when you want to. Gentoo also offers
  ready-made (prebuilt) packages, and bunny uses them when it can.
- Programs live in the normal places (`/usr/bin`, `/etc`), so things you
  download from the internet just work.

## What bunny adds

- **One file describes your whole system** (`bunny.toml`): which programs are
  installed, their USE flags, which services start at boot. See
  [The big idea](the-big-idea.md).
- **Undo for everything.** Before every change, bunny saves a snapshot of your
  system. If you don't like the result, you go back.
- **The same system on every computer.** Copy two files to another machine
  and get the same programs at the same versions.
- **Plain-English errors.** When Portage (Gentoo's package manager) prints
  something confusing, bunny tells you what it means and how to fix it.

## How it compares

|  | Gentoo | NixOS | Bunny Linux |
|---|---|---|---|
| Choose features per program | ✅ USE flags | ✅ (in the Nix language) | ✅ USE flags |
| Whole system in one file | ❌ | ✅ | ✅ (TOML) |
| Undo a change | ❌ | ✅ | ✅ |
| Same versions on every machine | ❌ | ✅ | ✅ |
| Normal file paths | ✅ | ❌ (`/nix/store/…`) | ✅ |
| Errors explained | ❌ | ❌ | ✅ |
| Init system | OpenRC or systemd | systemd | OpenRC |

## Where it is today

Bunny Linux installs from a live USB with `bunny install`, which sets up
encryption, OpenRC and a btrfs filesystem (btrfs is what makes undo
possible) and gives you a minimal system with a terminal: see
[Install Bunny Linux](installing.md). bunny also runs on an existing Gentoo
install that uses OpenRC and btrfs. A graphical installer and a ready-made
desktop are planned.
