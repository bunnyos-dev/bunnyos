# Remove a program

```sh
bunny remove mpv
```

The short name works if it's in your `bunny.toml`; so does the full name
(`media-video/mpv`).

bunny removes it from `bunny.toml`, shows what will change and asks. Then it
lists everything that will be removed: the program **and anything it needed
that nothing else uses**:

```
These packages aren't needed anymore (nothing you asked for uses them):
  - media-video/mpv  0.40.0
  - media-libs/libass  0.17.3
Remove them? [Y/n]
```

That question **always** appears, even with `--yes`, because removing
packages is the one thing you want to see first.

Like every change, removal saves a snapshot first, so it can be undone with
[`bunny rollback`](undo.md).

## Removing several

```sh
bunny remove mpv yt-dlp
```

## "It isn't in your config"

```
"vlc" isn't in /etc/bunny/bunny.toml, so there's nothing to remove.
```

bunny only removes what's in your config. If something is installed but not
in `bunny.toml`, it's a dependency of something else, or it was installed
outside bunny. `bunny plan` shows anything installed that isn't in your
config, under "Packages not in your config".
