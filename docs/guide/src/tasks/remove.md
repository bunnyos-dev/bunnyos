# Remove a program

```sh
bunny remove mpv
```

The short name works if it's in your `bunny.toml`; so does the full name
(`media-video/mpv`).

bunny removes it from `bunny.toml`, shows what will change and asks. Then
emerge lists everything that will be removed: the program **and anything it
needed that nothing else uses**. That step **always asks**, even with `--yes`,
because removing packages is the one thing you want to see first. Type `y`.

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
