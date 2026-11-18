# What to do first

You've [installed Bunny Linux](installing.md) and logged in to a terminal.
Three things are worth doing first: learn how a change works, keep your
config safe in git, and add a desktop.

## Your system is one file

Everything you choose about this computer is in `/etc/bunny/bunny.toml`:
the programs, the services that start with it, who can use sudo. To change
the system, you change that file and run `bunny switch`, or let bunny change
the file for you:

```sh
bunny add app-misc/tmux
```

bunny shows what will change, asks, saves a copy of the system first, then
installs it and adds it to `bunny.toml`. If you don't like the result:

```sh
bunny rollback
```

That's the whole idea; [The big idea](the-big-idea.md) explains it more, and
[Everyday tasks](../tasks/install.md) covers the rest.

Two more files sit next to `bunny.toml`:

- **`bunny.lock`**: the exact versions installed. bunny keeps them until you
  run `bunny update`.
- **`hosts/<computer name>.toml`**: what this computer needs to start (the
  kernel, the boot menu, the disk tools) and a record of how its disk was
  set up. Keeping these apart is what lets the same `bunny.toml` work on
  another computer. See [host files](../reference/config.md#host-files).

## Keep your config in git

Then you have the history of every change, and you can install the same
system on another computer with one command.

```sh
cd /etc/bunny
sudo git init
sudo git add bunny.toml bunny.lock hosts
sudo git commit -m "My system, as installed"
```

To keep it on GitHub, make a new **private** repository there (empty, no
README), then, with its address:

```sh
sudo git remote add origin https://github.com/you/my-system.git
sudo git push -u origin main
```

From then on, after a change: `sudo git commit -am "Added tmux"` and
`sudo git push`.

To install the same system on another computer, start it from the USB and run:

```sh
bunny install --config github:you/my-system
```

(For a private repository, that needs its `https://` address with a token in
it. Or copy `bunny.toml` onto the USB and give its path instead.)

## Add a desktop

Bunny Linux starts with only a terminal on purpose: adding a desktop is the
first thing you learn to do with bunny. A `[desktop]` section that does it
in one line (Bunny Desktop, KDE Plasma, Xfce, Hyprland) is planned. Until
then, you add the pieces yourself.

> **Being tested:** these steps haven't been tried on many computers yet,
> and some packages may compile for a while instead of downloading. If
> something fails, `bunny` explains what happened; if it's still unclear,
> [send a report](helping-test.md).

Hyprland, with kitty (the terminal Hyprland's starting settings open):

```sh
bunny add gui-wm/hyprland x11-terms/kitty sys-auth/elogind sys-apps/dbus
```

Hyprland needs two services running: `elogind` (it gives you access to the
screen and keyboard) and `dbus`. Open `bunny.toml`:

```sh
sudo nano /etc/bunny/bunny.toml
```

and add them to `services`:

```toml
services = [
    "dhcpcd",
    "sysklogd",
    "elogind",
    "dbus",
]
```

Save (Ctrl+O, Enter), close (Ctrl+X), and apply:

```sh
bunny switch
```

Restart, log in, and start it:

```sh
Hyprland
```

Super+Q opens a terminal (kitty). To leave, Super+M. Hyprland's settings are
in `~/.config/hypr/hyprland.conf`.

Next: the rest of [Everyday tasks](../tasks/status.md).
