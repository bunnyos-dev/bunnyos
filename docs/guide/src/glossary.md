# Glossary

## btrfs

A Linux filesystem that can take **snapshots**: instant, space-saving copies
of a folder tree. bunny's undo is built on them.

## Category

The first half of a Gentoo package name: `www-client` in
`www-client/firefox`. It groups similar programs.

## Dependency

A package another package needs to work. You don't list dependencies in
`bunny.toml`; they're installed automatically.

## Depclean

emerge's command for removing packages nothing needs anymore. bunny runs it
(and it always asks) when you remove something.

## ebuild

Gentoo's recipe for building one version of one package.

## emerge

Gentoo's command for installing and removing packages. bunny runs it for you.

## Generation

A saved copy of your whole system from right before a change. See
[Undo a change](tasks/undo.md).

## Keyword (`~amd64`)

Marks which versions Gentoo considers **stable** (`amd64`) or **testing**
(`~amd64`) on your kind of computer. `**` means "allow anything, even untested
development code": avoid it.

## Live version (`9999`)

A version number Gentoo uses for "build the latest unreleased code straight
from the developers' git repository". Not tested, not a release.

## Lockfile (`bunny.lock`)

The exact versions installed on your system, written by bunny. See
[The lockfile](reference/lock-and-pins.md).

## Mask

A Gentoo rule that blocks a package (or some versions of it). Gentoo masks
packages that are broken or about to be removed. bunny uses masks as
**pins** to hold packages at their locked versions.

## OpenRC

The init system BunnyOS uses: it starts services at boot. Its services live in
`/etc/init.d/`, and which ones start is set by **runlevels**.

## Overlay

An extra package repository on top of Gentoo's main one (for example GURU).

## Pin

bunny's word for a mask that stops a package from going past its locked
version.

## Portage

Gentoo's package manager. `emerge` is its main command.

## Rollback

Going back to a generation: `bunny rollback`.

## Runlevel

A group of OpenRC services that start together: `sysinit`, `boot`, then
`default` (where your services go).

## Slot

Some packages can have several versions installed side by side, each in its
own **slot** (Python 3.12 and 3.13, for example). In `bunny.toml` you can
write `dev-lang/python:3.12` to mean one slot.

## Snapshot

See **btrfs**.

## sudo / doas

Commands that run another command as root (the administrator), after asking
for your password. **sudo** is the common one and the BunnyOS default;
**doas** is a smaller, simpler alternative. Pick one in
[`[security]`](reference/config.md#security).

## Subvolume

A btrfs folder that can be snapshotted and mounted on its own. The BunnyOS
layout uses several, so rollback restores the system without touching
`/home`. See [The disk layout](reference/disk-layout.md).

## Switch

Making the system match `bunny.toml`: `bunny switch`.

## USE flag

A switch for an optional feature of a package. `pgo` turns a feature on,
`-telemetry` turns one off. Set per package in `bunny.toml`.

## World set

The list of packages you asked for directly (in Gentoo, the file
`/var/lib/portage/world`). bunny writes it from `packages`.

## wheel

The group whose members get admin rights through sudo or doas. `[security]
admins` puts users in it.
