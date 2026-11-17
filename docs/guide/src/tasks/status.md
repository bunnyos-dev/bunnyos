# Check that everything is in order

```sh
bunny status
```

It changes nothing and doesn't need your password. Each line is one thing
bunny keeps an eye on:

| Line | ✓ means | If it's ⚠ or ✗ |
|---|---|---|
| **Config** | The system matches `bunny.toml`. | Something isn't applied yet, or was installed outside bunny (say, with `emerge` directly). Run `bunny plan` to see it, `bunny switch` to apply your config. |
| **Versions** | Installed versions match `bunny.lock`. | `bunny switch` puts them back; `bunny lock` accepts them as they are. |
| **Last change** | (just information) | If it says to restart after a rollback, run `doas reboot`. |
| **Undo** | Generations are being saved. | Your disk isn't set up for undo; see [The disk layout](../reference/disk-layout.md). |
| **Updates** | The package list was synced in the last week. | Run [`bunny update`](update.md). |

If you see **"Also starting at boot, not in your config"**, a service was
turned on outside bunny. Add it to `services` in `bunny.toml` to keep it, or
turn it off with `doas rc-update del <name>`.
