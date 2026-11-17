# Testing BunnyOS in a virtual machine

The repository has two scripts that give you a safe place to try bunny:

- `tools/bunny-vm.sh` runs a QEMU virtual machine with UEFI, Secure Boot (in
  setup mode), a TPM 2.0 chip and 3D graphics, with snapshots so you can
  start fresh any time.
- `tools/vm-install-gentoo.sh` runs *inside* the VM and installs Gentoo with
  [the BunnyOS disk layout](disk-layout.md), OpenRC, doas and Rust, mostly from
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

## Tips

- **Keyboard stuck in the VM window?** Press **Ctrl+Alt+G** to release it.
- **Black window?** Start with `GL=0 tools/bunny-vm.sh run`.
- **"Connection refused" from ssh:** the VM isn't running or hasn't finished
  booting yet.
- **Paste commands that ask questions one at a time**, or the next pasted line
  becomes the answer.
