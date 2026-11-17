# The big idea: one file describes your computer

On most Linux systems, your setup is the result of everything you've ever
typed: every install, every removal, every edited file. Nobody could write it
down, so nobody can repeat it.

With bunny, your setup is **a file you can read**:

```toml
# /etc/bunny/bunny.toml
packages = [
    "app-editors/neovim",
    "app-misc/tmux",
    "www-client/firefox",
]

services = [
    "NetworkManager",
    "sshd",
]

[pkg."www-client/firefox"]
use = ["pgo", "-telemetry"]
```

That says: these programs are installed, these services start at boot, and
Firefox is built with `pgo` and without `telemetry`.

## Two files: what you want, and what you got

| File | What it says | Who writes it |
|---|---|---|
| `bunny.toml` | **What you want**: "I want Firefox." | You (or `bunny add`) |
| `bunny.lock` | **What you got**: "Firefox 133.0, from the Gentoo package tree as it was on 16 Nov." | bunny, automatically |

Think of a recipe and a receipt. The recipe (`bunny.toml`) is short and
yours. The receipt (`bunny.lock`) lists exactly what was bought, so you can buy
the same things again. See [The lockfile](../reference/lock-and-pins.md).

## Changing your system

You don't install programs one by one. You change the file, then tell bunny
to make the system match it:

```sh
bunny switch
```

`bunny add` and `bunny remove` do both steps for you: they edit the file and
switch.

## Why this is nice

- **You can see your whole setup** in one place, and keep it in git.
- **A second computer is easy**: copy the two files and switch.
- **Undo is real**: every switch saves the system first, so going back is
  one command. See [Undo a change](../tasks/undo.md).
- **Nothing drifts silently.** If someone changes the system by hand,
  `bunny plan` shows it.

> **Coming from NixOS?** Same idea, with TOML instead of the Nix language,
> normal file paths, OpenRC, and Gentoo's USE flags. Undo works on the whole
> system (btrfs snapshots) rather than per package.
