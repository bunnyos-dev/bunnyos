# Testing Bunny Linux in a virtual machine

The repository has two scripts that give you a safe place to try bunny:

- `tools/bunny-vm.sh` runs a QEMU virtual machine with UEFI, Secure Boot (in
  setup mode), a TPM 2.0 chip and 3D graphics, with snapshots so you can
  start fresh any time.
- `tools/vm-install-gentoo.sh` runs *inside* the VM and installs Gentoo with
  [the Bunny Linux disk layout](disk-layout.md), OpenRC, doas and Rust, mostly from
  prebuilt packages.

## 1. Install QEMU (on your computer)

```sh
echo 'app-emulation/qemu gtk opengl virgl QEMU_SOFTMMU_TARGETS: x86_64' | doas tee /etc/portage/package.use/qemu
echo 'net-libs/gnutls pkcs11 tools' | doas tee /etc/portage/package.use/swtpm
doas emerge -av app-emulation/qemu app-crypt/swtpm sys-firmware/edk2-bin
doas usermod -aG kvm $USER
```

Log out and back in. Check `ls -l /dev/kvm` exists.

## Trying the Bunny Linux live USB

To try the installer the way a new user would, boot the live USB image
(from a [release](https://github.com/bunnyos-dev/bunnyos/releases/latest), or
built with `tools/build-iso.sh`) in a fresh VM:

```sh
VM=fresh tools/bunny-vm.sh run ~/Downloads/bunny-linux-live-*.iso
```

(`ISO=… tools/bunny-vm.sh run` does the same.) Then follow
[Install Bunny Linux](../start/installing.md) from step 4. Afterwards, start
it without the ISO: `VM=fresh tools/bunny-vm.sh run`.

The rest of this page sets up a plain Gentoo in the VM instead, for trying
bunny on an existing install.

## 2. Install Gentoo in the VM

Download the amd64 **Minimal Installation CD** from
[gentoo.org/downloads](https://www.gentoo.org/downloads/), then:

```sh
cd bunnyos
tools/bunny-vm.sh run ~/Downloads/install-amd64-minimal-*.iso
```

In a **second terminal**, share the repository with the VM:

```sh
cd bunnyos && python3 -m http.server 8000
```

In the **VM window**, once it shows `livecd ~ #`:

```sh
wget http://10.0.2.2:8000/tools/vm-install-gentoo.sh
sh vm-install-gentoo.sh
```

(`10.0.2.2` is how the VM reaches your computer. Use `http`, not `https`.)
Type `yes`, choose a password for `root` and the user `you`, and wait. When it
says **"✓ Gentoo is installed"**, run `poweroff` and stop the web server
(Ctrl+C).

## 3. Save a clean starting point

```sh
tools/bunny-vm.sh run
```

Log in once to check it works, `doas poweroff`, then:

```sh
tools/bunny-vm.sh snap clean-install
```

## Everyday use

| Command | What it does |
|---|---|
| `tools/bunny-vm.sh run` | Start the VM |
| `tools/bunny-vm.sh stop` | Shut the VM down (asks for your password in the VM) |
| `tools/bunny-vm.sh ssh` | Log in from your terminal (copy and paste work here) |
| `tools/bunny-vm.sh put <file>` | Copy a file into the VM's home folder |
| `tools/bunny-vm.sh restore clean-install` | Go back to the clean install (VM off) |
| `tools/bunny-vm.sh snap <name>` / `snaps` | Save / list VM snapshots (VM off) |
| `tools/bunny-vm.sh reset-keys` | Wipe the VM's Secure Boot keys |
| `tools/bunny-vm.sh forget-key` | Forget the VM's SSH key, after you reinstall it |

The VM's user is `you` unless you say otherwise: `VM_USER=bunny tools/bunny-vm.sh ssh`
(also for `put` and `stop`).

To test `/home` on a second disk, start the VM with `HOME_DISK=24G` (any
size of 20 GB or more): it attaches a second disk, made the first time and
kept in the VM's folder.

To test a build of bunny: `cargo build --release`,
`tools/bunny-vm.sh put target/release/bunny`, then run `./bunny …` in the VM.

## A second VM shaped like a hand-made install

Many people already run Gentoo with an encrypted disk, the boot partition at
`/boot`, and btrfs without Bunny Linux's subvolumes. To test moving such a
system to the Bunny Linux layout, there's a second, separate VM for it. It has
its own disk, snapshots and SSH port, so both VMs can exist (and run) side
by side.

```sh
VM=laptop tools/bunny-vm.sh run ~/Downloads/install-amd64-minimal-*.iso
```

With the web server running as above, in the **VM window**:

```sh
wget http://10.0.2.2:8000/tools/vm-install-gentoo.sh
LAYOUT=laptop sh vm-install-gentoo.sh
```

What it makes:

| | |
|---|---|
| `/dev/vda1`, 1 GB | the EFI partition, mounted at **`/boot`** (kernels live here) |
| `/dev/vda2` | **LUKS2 encrypted**, unlocked with the same password at boot |
| inside it | btrfs with the root at the top of the volume (no `@`), `/srv` as a subvolume, a 2 GB `/swapfile` |

Then, as before: `poweroff`, start it without the ISO, **type the password
in the VM window** to unlock the disk, log in, `doas poweroff`, and save a
starting point:

```sh
VM=laptop tools/bunny-vm.sh run
VM=laptop tools/bunny-vm.sh snap laptop-before
```

Every command takes `VM=laptop` the same way: `VM=laptop tools/bunny-vm.sh ssh`,
`VM=laptop tools/bunny-vm.sh put target/release/bunny`, and so on.

## Several VMs at once

Each VM has its own SSH port, so you can run two (or more) at the same time,
like a fresh install next to an older one:

- The main VM (no `VM=`) always uses **2222**.
- A named VM (`VM=laptop`, `VM=fresh`, …) gets its own port the first time
  it's used: the lowest from **2223** up that no other named VM has. It's
  kept in its folder (`~/bunny-vm-<name>/port`), so it stays the same.
- The port is shown when the VM starts ("SSH on port 2224"), and
  `VM=laptop tools/bunny-vm.sh port` prints it. `ssh`, `put` and `stop`
  use it for you.
- To use another port just once: `PORT=2230 VM=laptop tools/bunny-vm.sh run`
  (and the same `PORT=` for its `ssh`). To change it for good, edit the
  `port` file while the VM is off.

VMs made before this had no `port` file; each gets one the next time it's
used, in the order you use them. SSH may ask once to accept its key on the
new port, then remembers it.

## Tips

- **Keyboard stuck in the VM window?** Press **Ctrl+Alt+G** to release it.
- **Black window?** Start with `GL=0 tools/bunny-vm.sh run`.
- **"Connection refused" from ssh:** the VM isn't running or hasn't finished
  booting yet.
- **"REMOTE HOST IDENTIFICATION HAS CHANGED" or "Host key verification
  failed":** each VM remembers its own SSH key (in `~/bunny-vm-<name>/known_hosts`,
  or `~/bunny-vm/known_hosts` for the main one), so two VMs never mix up
  each other's keys. If you see it anyway, that VM was reinstalled and
  has a new key: run `VM=<name> tools/bunny-vm.sh forget-key` and connect
  again. (Older versions of this script kept the keys in your own
  `~/.ssh/known_hosts`; the old `[localhost]:2222` and `[localhost]:2223`
  lines there aren't used any more.)
- **Paste commands that ask questions one at a time**, or the next pasted line
  becomes the answer.

## Trying the graphical installer

The graphical installer (`crates/bunny-installer`) runs on your desktop too,
in a window, with two pretend disks. It only does dry runs there, so nothing
is changed:

```sh
cargo build -p bunny -p bunny-installer
```
```sh
target/debug/bunny-installer --demo --bunny target/debug/bunny
```

`--screenshots <folder>` draws every screen to PNG files instead, with no
window. The installer is a thin window over `bunny install`: it builds the
command (`--yes`, `--user`, `--password-stdin` and the rest), and bunny
checks the answers, shows the preview and does the work.
