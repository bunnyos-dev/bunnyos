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
| `tools/bunny-vm.sh ssh` | Log in from your terminal (copy and paste work here) |
| `tools/bunny-vm.sh put <file>` | Copy a file into the VM's home folder |
| `tools/bunny-vm.sh restore clean-install` | Go back to the clean install (VM off) |
| `tools/bunny-vm.sh snap <name>` / `snaps` | Save / list VM snapshots (VM off) |
| `tools/bunny-vm.sh reset-keys` | Wipe the VM's Secure Boot keys |

To test a build of bunny: `cargo build --release`,
`tools/bunny-vm.sh put target/release/bunny`, then run `./bunny …` in the VM.

## A second VM shaped like a hand-made install

Many people already run Gentoo with an encrypted disk, the boot partition at
`/boot`, and btrfs without Bunny Linux's subvolumes. To test moving such a
system to the Bunny Linux layout, there's a second, separate VM for it. It has
its own disk and snapshots, and SSH on port 2223, so both VMs can exist side
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

## Tips

- **Keyboard stuck in the VM window?** Press **Ctrl+Alt+G** to release it.
- **Black window?** Start with `GL=0 tools/bunny-vm.sh run`.
- **"Connection refused" from ssh:** the VM isn't running or hasn't finished
  booting yet.
- **Paste commands that ask questions one at a time**, or the next pasted line
  becomes the answer.
