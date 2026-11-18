# Getting bunny

> Today, bunny is installed on an existing Gentoo system, from the Bunny Linux
> overlay. A ready-made Bunny Linux installer is planned.

## What you need

- **Gentoo with OpenRC.**
- **Root (`/`) on btrfs**, with a `/.snapshots` folder that's its own btrfs
  subvolume. That's what lets bunny undo changes. Without it bunny still
  works, but switches can't be rolled back (it warns you each time). The
  recommended layout is in [The disk layout](../reference/disk-layout.md).
- **sudo** or **doas** set up for your user. bunny asks for your password
  through whichever one you have when it needs to change the system. (bunny
  can also manage this for you: see [`[security]`](../reference/config.md#security).)

> **Try it in a virtual machine first.** The repository has a script that
> sets one up with the right layout. See
> [Testing Bunny Linux in a virtual machine](../reference/testing-in-a-vm.md).

## Add the Bunny Linux overlay

An overlay is an extra package repository that sits next to Gentoo's own.
Bunny Linux's is called `bunny`. Adding it needs `eselect repository`, and git
to download it:

```sh
sudo emerge --ask app-eselect/eselect-repository dev-vcs/git
sudo eselect repository add bunny git https://github.com/bunnyos-dev/bunny-overlay.git
sudo emaint sync -r bunny
```

## Install bunny

```sh
sudo emerge --ask app-admin/bunny-bin
```

That's the latest release, already built, so it installs in seconds. Check
it works:

```sh
bunny --version
```

## Update bunny itself

bunny updates with the rest of your system. `bunny update` syncs every
repository, the overlay included, and installs the newest release.

## Other ways to get bunny

**The newest code, not yet released** (builds from source, needs Rust; for
testing what's coming):

```sh
echo "app-admin/bunny **" | sudo tee /etc/portage/package.accept_keywords/bunny
sudo emerge --ask app-admin/bunny
```

`app-admin/bunny` and `app-admin/bunny-bin` can't be installed together;
emerge swaps one for the other.

**Without the overlay**, download the release yourself:

```sh
curl -LO https://github.com/bunnyos-dev/bunnyos/releases/latest/download/bunny-x86_64-linux
curl -LO https://github.com/bunnyos-dev/bunnyos/releases/latest/download/bunny-x86_64-linux.sha256
sha256sum -c bunny-x86_64-linux.sha256
sudo install -m 755 bunny-x86_64-linux /usr/local/bin/bunny
```

Next: [Your first five minutes](first-five-minutes.md).
