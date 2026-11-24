# Check that everything is in order

```sh
bunny status
```

It changes nothing and doesn't need your password. Each line is one thing
bunny keeps an eye on:

| Line | ✓ means | If it's ⚠ or ✗ |
|---|---|---|
| **Config** | The system matches `bunny.toml`. | Something isn't applied yet, or was installed outside bunny (say, with `emerge` directly). Run `bunny diff` to see what. |
| **Versions** | Installed versions match `bunny.lock`. | `bunny switch` puts them back; `bunny lock` accepts them as they are. If it says packages are "not in bunny.lock yet" (installed outside bunny), `bunny switch` records them. |
| **Last change** | (just information) | If it says to restart after a rollback, run `sudo reboot`. |
| **Undo** | Generations are being saved. | Your disk isn't set up for undo; see [The disk layout](../reference/disk-layout.md). |
| **Kernel** | (only shown when there's a problem) | `/boot` starts a kernel this system has no drivers for, usually after undoing a kernel update from the boot menu. Hold Esc while starting and pick the older kernel under "Advanced options". |
| **Updates** | The package list was synced in the last week. | Run [`bunny update`](update.md). |
| **Security** | No Gentoo security advisory affects your packages. | Run [`bunny update`](update.md) to get the fixes. `glsa-check --dump <number>` shows an advisory in full. |

The **Security** line needs `glsa-check`, from Gentoo's gentoolkit. If it isn't
installed, status says so; `bunny add app-portage/gentoolkit` turns it on.
Advisories arrive with the package list, so it's only as current as your last
`bunny update`.

If you see **"Also starting at boot, not in your config"**, a service was
turned on outside bunny. Add it to `services` in `bunny.toml` to keep it, or
turn it off with `sudo rc-update del <name>`.

## In scripts and scheduled checks

```sh
bunny status --check
```

Prints the same screen, then exits with code 1 if anything is marked ⚠ or ✗
(0 when everything is in order), so a script or a scheduled job can tell
something needs a look.

## See exactly what's different

```sh
bunny diff
```

```
Comparing /etc/bunny/bunny.toml with this system.

Packages
  config only   media-video/mpv   (in your config, not installed yet)
  system only   app-text/tree   (installed outside bunny; a switch would remove it)

Services at boot (OpenRC)
  system only   rsyncd   (turned on outside bunny; a switch leaves it alone)

To make the system match your config:
    bunny switch

(`bunny plan` shows exactly what that switch would do.)

To keep the system as it is instead:
  Edit /etc/bunny/bunny.toml:
    • add to `packages`: "app-text/tree"
    • take out of `packages`: "media-video/mpv"
    • add to `services`: "rsyncd"
  Then accept the installed versions:
    bunny lock
```

**config only** means it's in `bunny.toml` but not on the system;
**system only** means the opposite. For each difference you get both ways to
fix it, and you choose:

- **Your config is right:** run `bunny switch`.
- **The system is right** (you installed something on purpose and want to
  keep it): make the edits it lists, then `bunny lock`. Nothing gets
  installed or removed.

For USE flags, only the flags that differ are shown, as "wayland on" or
"telemetry off".
