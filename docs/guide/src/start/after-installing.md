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
first thing you learn to do with bunny. Choosing a whole desktop in one line
(Bunny Desktop, KDE Plasma, Xfce, Hyprland) is planned. Until then, you add
the desktop yourself, and one line in `bunny.toml` sets up what it needs.

> **Being tested:** these steps are new, and some packages compile for a
> while instead of downloading. If something fails, bunny explains what
> happened; if it's still unclear, [send a report](helping-test.md).

A Wayland desktop gets the screen and keyboard through **elogind**, which
Bunny Linux's minimal system doesn't use yet. Open `bunny.toml`:

```sh
sudo nano /etc/bunny/bunny.toml
```

and add this at the end of the file:

```toml
[desktop]
wayland = true
```

Save (Ctrl+O, Enter) and close (Ctrl+X). That turns on elogind and D-Bus,
and logins that go through elogind. (The exact settings are in
[`[desktop]`](../reference/config.md#desktop).)

[Mango](https://mangowm.github.io) is a small, fast Wayland desktop, set up
with one text file. It isn't in Gentoo's own package tree: it's in **GURU**,
a repository Gentoo users look after together, and only as a testing
version. bunny sets both up for you:

```sh
bunny add mangowm gui-apps/foot media-fonts/dejavu
```

bunny says Mango is in the GURU repository and asks whether to add it. Type
`y`. It adds three things to `bunny.toml`: `repositories = ["guru"]`, and
`keywords = ["~amd64"]` for Mango and the effects library it uses, which
allows their testing versions. Then it shows the plan, with the elogind
pieces marked "(from [desktop])", and asks again before changing anything.

(foot is the terminal Mango opens, and DejaVu is a font, since a fresh
system has none.)

It may say a font is no longer needed (the one something else pulled in
before DejaVu); that's fine to remove.

**Log out and back in** (`exit`), so your login goes through elogind (bunny
reminds you at the end). `echo $XDG_RUNTIME_DIR` should now print
`/run/user/1000`. Then, at the computer itself (not over SSH, which has no
screen), copy Mango's starting settings so you can change them, and start it:

```sh
mkdir -p ~/.config/mango
cp /etc/mango/config.conf ~/.config/mango/
mango
```

Alt+Enter opens a terminal, Alt+Q closes a window, and Super+M leaves Mango.
Everything else is in `~/.config/mango/config.conf`; Super+R reloads it.

> **Prefer something in Gentoo's own tree?** Sway works the same way without
> a second repository: the same `[desktop]` lines, then `bunny add
> gui-wm/sway gui-apps/foot media-fonts/dejavu`, log in again, and `sway`
> (Super+Enter for a terminal, Super+Shift+E to leave).

If `mango` says "XDG_RUNTIME_DIR must be set", your login isn't going through
elogind yet: check that `[desktop]` has `wayland = true` (`bunny plan` should
say nothing is left to do) and log in again. "No backend was able to open a
seat" means the same, or that you started it over SSH.

Next: the rest of [Everyday tasks](../tasks/status.md).
