# Finding and using USE flags

Most Gentoo packages can be built with features switched on or off. Each
switch is a **USE flag**: `wayland` (support Wayland), `pulseaudio` (play
sound through PulseAudio or PipeWire), `telemetry` (send usage data to the
program's makers). You choose them per package in `bunny.toml`.

This page shows how to find a package's flags and what they mean, using
Gentoo's own pages and your own system, rather than a list to copy: flags
change from one version to the next, and Gentoo's pages are kept up to date
by the people who package the program.

The example is Firefox. Its flags below are **as they were at the time of
writing (November 2026); check the pages for the current ones.**

## 1. Look the package up

Open the package's page on **packages.gentoo.org**. For Firefox that's
<https://packages.gentoo.org/packages/www-client/firefox>. It shows:

- **The versions**, and which are stable on your kind of computer (`amd64`
  for most PCs) and which are only testing (`~amd64`).
- **The USE flags**, each with a one-line description. They come in two
  kinds:
  - **Global** flags mean the same thing in every package (`wayland`,
    `pulseaudio`, `debug`).
  - **Local** flags only exist for this package, and the package explains
    them itself (Firefox's `pgo`, `hwaccel`, `eme-free`).

Then look for the program on the **Gentoo wiki**: for Firefox,
<https://wiki.gentoo.org/wiki/Firefox>. Wiki pages explain things the flag
list can't: what's worth changing, what breaks, and alternatives. They're
written by volunteers and can be older than the package, so when the wiki
and packages.gentoo.org disagree about which flags exist, trust
packages.gentoo.org.

**What a description tells you.** Read it for the cost as well as the
feature. At the time of writing, Firefox's own descriptions included:

| Flag | What Gentoo's description says |
|---|---|
| `pgo` | "Add support for profile-guided optimization for faster binaries - this option will double the compile time" |
| `hwaccel` | "Force-enable hardware-accelerated rendering" |
| `eme-free` | "Disable EME (DRM plugin) capability at build time" (no Netflix and the like) |
| `jumbo-build` | "combines source files to speed up build process, but requires more memory" |
| `telemetry` | (global) "Send anonymized usage information to upstream so they can better understand our users" |

And the wiki's Firefox page said, at the time of writing, that a prebuilt
version exists (`www-client/firefox-bin`) but that "almost all" of the flags
can't be set for it, that its own list of flags is "not comprehensive", and
it suggested `equery uses www-client/firefox` for the full list (step 2).
Parts of that page were written for much older Firefox versions; the flag
list on packages.gentoo.org is the one to go by.

**Which flags are on to begin with.** A package turns some flags on by
itself. In the package's recipe (its ebuild) they're written with a `+`:
Firefox 153.7.0's starts `IUSE="+clang dbus debug eme-free hardened hwaccel
…"`, so `clang` is on unless you turn it off, and the others are off unless
something turns them on. Your system's profile turns more on (the desktop
profiles turn on `wayland`, `pulseaudio`, `dbus` and many more). You don't
need to work this out by hand: step 2 shows the result.

## 2. Check them on your own system

Two ways, both read-only.

**With emerge**, which every Gentoo system has:

```sh
emerge --pretend --verbose --nodeps www-client/firefox
```

```
[binary  N g   ] www-client/firefox-153.7.0-18:esr::gentoo  USE="X clang dbus gmp-autoupdate … telemetry wayland -debug -eme-free -hardened -hwaccel -jack … -pgo …"
```

- `flag` is on, `-flag` is off.
- `(flag)` in brackets is decided by Gentoo for your system (forced or
  masked); you can't change it.
- `flag*` changed since it was last installed; `flag%` is new in this
  version.
- `binary` at the start means a prebuilt copy with exactly these flags
  exists. `ebuild` means it would be compiled on your computer.

**With equery**, which explains every flag in one list. It's in Gentoo's
`app-portage/gentoolkit`:

```sh
bunny add app-portage/gentoolkit
```

```sh
equery uses www-client/firefox
```

Each line is a flag, a `+` or `-` for whether it's on, and Gentoo's
description. (gentoolkit also gives `bunny status` its security check.)

## 3. Write what you want in bunny.toml

Open the config:

```sh
sudo nano /etc/bunny/bunny.toml
```

Add a section for the package, with its full name in quotes. List the flags
to turn on, and the ones to turn off with a `-` in front:

```toml
[pkg."www-client/firefox"]
use = ["hwaccel", "-telemetry"]
```

Flags you don't list keep their usual setting. Only list what you want
different.

**Testing versions.** If the version you want is only `~amd64` on
packages.gentoo.org (at the time of writing, Firefox 161.0 was testing while
153.7.0, the long-term "ESR" release, was stable), allow testing versions
for that one package:

```toml
[pkg."www-client/firefox"]
use = ["hwaccel", "-telemetry"]
keywords = ["~amd64"]
```

Most testing versions work fine; Gentoo just hasn't declared them stable
yet. Avoid `"**"`, which also allows unreleased versions straight from git.

## 4. See what it changes, then apply it

```sh
bunny plan
```

```
USE flag changes:
  ~ www-client/firefox: (none) → hwaccel -telemetry
```

`bunny plan` changes nothing. When it looks right:

```sh
sudo bunny switch
```

Before installing, the switch shows emerge's own list. Look for
**`compile`** next to the package:

```
emerge will install 1 package, 1 to compile:
  ~ www-client/firefox  153.7.0 (rebuild: USE +hwaccel -telemetry)  compile
```

(That's with Firefox already installed. Installing it for the first time
shows `+ www-client/firefox  153.7.0  compile`, often with packages it
needs.)

Gentoo's prebuilt packages are built with one set of flags. Change a flag
away from that set and the package is compiled on your computer instead,
which for Firefox takes hours and a lot of memory. If that's not what you
want, answer **n**: nothing has changed yet. Then take the flag back out of
`bunny.toml`, or use the prebuilt `www-client/firefox-bin` and change the
setting inside Firefox instead (telemetry, for example, can be turned off in
Firefox's own settings).

If you go ahead and don't like the result, [undo it](undo.md):

```sh
sudo bunny rollback
```

## More

- Every `[pkg]` setting: [bunny.toml, every setting](../reference/config.md#pkgcategoryname)
- Gentoo's explanation of USE flags: <https://wiki.gentoo.org/wiki/USE_flag>
- Every global flag in one list: <https://www.gentoo.org/support/use-flags/>

Not yet in bunny: USE flags for every package at once (Gentoo's `make.conf`
`USE=` line) is planned, and so is a check that warns when a flag you wrote
doesn't exist for that package.
