# Install a program

## Find it

```sh
bunny search video player
```

```
  media-video/mpv         Media player based on MPlayer and mplayer2
  media-video/vlc         Media player and framework with support for most multimedia files
  ...
To install one: bunny add <name>
```

Search looks at names **and** descriptions, so you can search for what a
program does. Installed ones are marked `(installed)`.

## Add it

```sh
bunny add mpv
```

bunny:

1. finds the full name (`media-video/mpv`)
2. adds it to `/etc/bunny/bunny.toml` (keeping your comments and layout)
3. shows what will change and asks
4. saves a snapshot of your system, so you can undo
5. installs it, and records the exact version in `bunny.lock`

Press **Enter** at "Apply these changes? [Y/n]".

Then bunny asks emerge for its full list and shows it: what you asked for
first, then anything it needs that you don't have yet:

```
emerge will install 3 packages, all prebuilt:
  + media-video/mpv     0.40.0

  And 2 packages it needs:
  + media-libs/libass   0.17.3
  + media-video/ffmpeg  7.1.1

Install these? [Y/n]
```

That question only appears when the list has more than you asked for. A long
list of dependencies is folded after the first 15 (`--verbose` shows them
all). `compile` marks packages that are built on your computer instead of
downloaded ready-made.

Sometimes the list also has packages you already have, marked `~` and
installed again at the same version. bunny says why when emerge does:

```
  ~ net-misc/curl      8.16.0 (rebuild: USE +adns)
  ~ sys-devel/gettext  0.23.1 (rebuild: libunistring changed)
```

- `USE +adns`: a USE flag changed (`+` turned on, `-` turned off), so it's
  built with the new setting.
- `libunistring changed`: a library it uses got a new version that isn't
  compatible with the old one, so it has to be rebuilt against it. When
  bunny can't tell which library, it says `a library it uses changed`.
- Just `(rebuild)`: emerge didn't say why.

**Big programs:** some, like Firefox, take hours to compile. If a prebuilt
version exists (`www-client/firefox-bin`), bunny says so before anything
starts. Answer **n**, then `bunny add www-client/firefox-bin`.

While it installs you see one line that keeps moving:

```
  ⠹ [101/102] Compiling www-client/firefox 153.7.0  ·  37%  ·  24m 10s
```

The spinner turns as long as work is happening. The percentage appears when
the build reports how far it is; when it doesn't, bunny shows how long the
same package took last time, if it was installed on this computer before.
emerge's full output is saved in `/var/log/bunny/` (or run with `--verbose`
to watch it scroll by).

If you say **no** at either question, nothing is installed and `bunny.toml` is
put back exactly as it was.

Some packages leave a message after installing (a setup step, a tip). bunny
shows those at the end under **Messages from installed packages**.

## Several at once

```sh
bunny add mpv yt-dlp
```

## When the name isn't clear

Typo:

```
✗ There's no package called "mvp".
  Did you mean: media-video/mpv?
```

Several packages with the same name:

```
✗ Several packages are called "pip". Pick one:
    bunny add app-admin/pip
    bunny add dev-python/pip
```

Use the full name it suggests.

## With specific features (USE flags)

`bunny add` installs with the default features. To change them, edit
`bunny.toml` and add a section for the package:

```toml
[pkg."media-video/mpv"]
use = ["vaapi", "-lua"]
```

then run `bunny switch`. A word on its own turns a feature on; with `-` in
front it's turned off. To see what a package's flags mean, look it up on
[packages.gentoo.org](https://packages.gentoo.org).

## Big programs with flatpak

Some big desktop programs are easier as Flatpaks (no compiling, sandboxed).
bunny doesn't manage Flatpaks yet; install them with `flatpak` as usual.
