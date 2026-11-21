# Install Bunny Linux

This page takes you from a blank computer to a working Bunny Linux: a
terminal, your account, encryption, and undo. A desktop is the
[next step](after-installing.md).

> **New:** the live USB and `bunny install` are new. If something on this
> page is unclear or doesn't match what you see, please
> [send a report](helping-test.md). That's exactly what's being tested.

## What you need

- A 64-bit (x86-64) computer that starts in **UEFI** mode. Almost every
  computer from the last ten years does.
- A USB stick of 2 GB or more. It's erased.
- A disk of at least 20 GB for Bunny Linux. **The whole disk is erased**
  (to keep another system on it, see
  [installing onto your own partitions](../reference/install.md#installing-onto-your-own-partitions)).
- The internet, by cable or Wi-Fi. The system is downloaded while it
  installs.
- About 30 minutes.

> **Try it in a virtual machine first** if you can:
> `tools/bunny-vm.sh run bunny-linux-live-<version>.iso` from the
> repository. See [Testing Bunny Linux in a virtual machine](../reference/testing-in-a-vm.md).

## 1. Download the USB image

From the [newest release](https://github.com/bunnyos-dev/bunnyos/releases/latest),
download `bunny-linux-live-<version>.iso` and the `.sha256` file next to it.
Check that the download is complete:

```sh
sha256sum -c bunny-linux-live-*.iso.sha256
```

It should say `OK`.

## 2. Write it to the USB stick

**On Linux:** find the stick's name with `lsblk` (it's the one the size of
your stick, like `sdb`). Then, with your stick's name instead of `sdX`:

```sh
doas dd if=bunny-linux-live-<version>.iso of=/dev/sdX bs=4M status=progress oflag=sync
```

Use `sudo` instead of `doas` if that's what you have. Double-check the name:
`dd` erases whatever disk you give it.

**On Windows or macOS:** use [Fedora Media Writer](https://github.com/FedoraQt/MediaWriter/releases)
or [balenaEtcher](https://etcher.balena.io): pick the `.iso` and the stick,
then write. (With Rufus, choose **DD image mode** when it asks.)

## 3. Start the computer from the stick

1. **Turn off Secure Boot** in the computer's firmware settings (often under
   "Security" or "Boot"). The USB isn't signed by Microsoft yet, so it won't
   start with Secure Boot on. Keeping Secure Boot on with Bunny Linux is
   planned.
2. Plug in the stick, start the computer and open its **boot menu**. The key
   is often F12, F11, F8 or Esc, shown briefly at start-up.
3. Pick the USB entry that starts with **UEFI**.

A menu titled "Bunny Linux live USB" appears and starts by itself after 5
seconds. A minute later you're at a terminal with a welcome:

```
  Bunny Linux  ·  live USB

  This is a small Bunny Linux system running from the USB stick. Nothing on
  this computer has been changed.
  …
```

Under it, "Checking the connection…" waits up to about 15 seconds for the
network, then says one of:

- **✓ You're online.** Skip to step 5.
- **✗ Not online yet.** Connect first (step 4).
- **✗ Connected to a network, but it doesn't reach the internet.** Often a
  network that wants a sign-in page first; use another one.
- **⚠ Online, but names don't work.** The internet answers, but names like
  `distfiles.gentoo.org` can't be looked up (DNS), so downloads would fail.
  It's often just a few seconds late; if it stays, the message shows a line
  that uses a public name server.

Nothing on the computer changes until you confirm the install in step 6.
Type `welcome` any time to see the welcome again and check the connection
again.

## 4. Get online

**Cable:** plug it in. It connects by itself in a few seconds.

**Wi-Fi:** with your network's name:

```sh
iwctl station wlan0 scan
iwctl station wlan0 get-networks
iwctl station wlan0 connect "Network name"
```

It asks for the Wi-Fi password. (If `wlan0` isn't found, `iwctl device list`
shows your Wi-Fi's name.)

Check:

```sh
welcome
```

It ends with "✓ You're online." when you are.

## 5. See what will happen (optional)

```sh
bunny install --dry-run
```

This goes through every question and then lists each step and what it
does, without changing anything. It's a good way to see what's coming:

```
==> Partitioning /dev/nvme0n1
    Erases /dev/nvme0n1's partition table and makes two partitions: EFI (2 GB),
    which the computer starts from, and the rest for Bunny Linux.

==> Encrypting the disk (LUKS2)
    Encrypts the Bunny Linux partition with your password, then unlocks
    it so the install can write to it.
```

To see the exact commands and files instead, add `--verbose`:
`bunny install --dry-run --verbose`.

## 6. Install

```sh
bunny install
```

bunny asks a few things, one at a time:

1. **Which disk.** It lists the disks with their size and model, and leaves
   out ones it can't use (the USB stick, for one) with the reason. With only
   one disk, it picks that one.
2. **What will be erased, and what you'll get.** Every partition on the disk
   (like "NTFS (Windows)"), then the new layout: a small EFI partition, and
   the rest encrypted.
3. **Confirm by typing the disk's name** (like `nvme0n1`). Anything else
   stops here, and nothing is changed.
4. **Your user name**, in lowercase (like `you`).
5. **A name for this computer** (like `laptop`).
6. **Your time zone**, like `America/Toronto` or `Europe/London` (Enter for
   UTC).
7. **Your keyboard layout**, like `us`, `uk`, `de` or `fr` (Enter for `us`).
8. **A password**, typed twice. It's for your account **and** for unlocking
   the disk each time the computer starts, so pick one you'll remember. If
   it's lost, the disk can't be read. A long one is safest: a few words
   together is easy to remember and hard to guess. If it's under 8
   characters, bunny says so; type it again to keep it anyway, or press
   Enter to choose another.

Then it works for 15 to 30 minutes, depending on your internet. You'll see
each step: partitioning, encrypting, downloading Gentoo's base system,
installing packages, the boot menu, your account, and last, applying your
config. It ends with:

```
✓ Bunny Linux is installed on /dev/nvme0n1.
```

If a step fails, bunny says which and why, puts the disk back to a state
where you can simply run `bunny install` again, and changes nothing else.
The most common cause is the network dropping.

Useful options (all of them are in
[bunny install, in detail](../reference/install.md)):

| Option | What it does |
|---|---|
| `--swap 8G` | A swapfile (8 GB here). |
| `--hibernate` | Set up hibernation (a swapfile as big as your memory). |
| `--no-encrypt` | Don't encrypt the disk. Anyone with the computer can then read it. |
| `--edit` | Change the config before it's applied. |
| `--config github:you/my-system` | Install from your own config instead. |

## 7. Restart

```sh
reboot
```

Take out the USB stick when the screen goes dark. Then:

1. A short boot menu ("Bunny Linux"), which starts by itself.
2. **Type your password** to unlock the disk.
3. Log in with your user name and the same password.

## 8. Check that everything's in order

```sh
bunny status
```

```
✓ Config        the system matches your config
✓ Versions      all … packages match bunny.lock
…
```

Your system is described by `/etc/bunny/bunny.toml`, and it's already
applied. Have a look:

```sh
less /etc/bunny/bunny.toml
```

Next: [What to do first](after-installing.md).
