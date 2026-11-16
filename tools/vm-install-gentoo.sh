#!/bin/sh
# vm-install-gentoo.sh: install a Gentoo test system inside the BunnyOS VM.
#
# Run this INSIDE the VM, from the Gentoo minimal install ISO, as root.
# It ERASES /dev/vda and refuses to run anywhere but a QEMU virtual machine.
#
# What you get (the layout from docs/DESIGN.md, without encryption for now):
#   /dev/vda1  2 GB   FAT32  EFI partition, mounted at /efi
#   /dev/vda2  rest   btrfs  subvolumes @ @home @log @cache @build @snapshots
#   OpenRC, gentoo-kernel-bin, GRUB, dhcpcd, sshd, doas, git, Rust
#   Packages come from Gentoo's binary host, so very little is compiled.

set -eu

DISK=/dev/vda
EFI_PART=${DISK}1
ROOT_PART=${DISK}2
MNT=/mnt/gentoo
MIRROR=https://distfiles.gentoo.org/releases/amd64/autobuilds
BTRFS_OPTS=compress=zstd:1,noatime

say()  { printf '\n\033[1;35m==> %s\033[0m\n' "$*"; }
fail() { printf '\n\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------- safety ----
[ "$(id -u)" = 0 ] || fail "Run this as root (the live ISO logs you in as root)."
vendor="$(cat /sys/class/dmi/id/sys_vendor /sys/class/dmi/id/product_name 2>/dev/null || true)"
case "$vendor" in
    *QEMU*|*"Standard PC"*) ;;
    *) fail "This doesn't look like the QEMU VM ($vendor). Refusing to erase anything." ;;
esac
[ -b "$DISK" ] || fail "$DISK not found. Is this the BunnyOS VM?"
[ -d /sys/firmware/efi ] || fail "The VM didn't boot in UEFI mode. Start it with tools/bunny-vm.sh."
ping -c1 -W3 distfiles.gentoo.org >/dev/null 2>&1 || fail "No internet in the VM. Check the network, then try again."

echo
echo "This will ERASE $DISK ($(lsblk -dno SIZE "$DISK")) inside the VM and install Gentoo."
printf 'Type yes to continue: '
read -r answer
[ "$answer" = yes ] || fail "Cancelled. Nothing was changed."

printf 'Password for root and for the user "you": '
stty -echo; read -r PASSWORD; stty echo; echo
printf 'Again: '
stty -echo; read -r PASSWORD2; stty echo; echo
[ -n "$PASSWORD" ] && [ "$PASSWORD" = "$PASSWORD2" ] || fail "Passwords were empty or didn't match."

# ------------------------------------------------------------- partition ----
say "Partitioning $DISK"
umount -R "$MNT" 2>/dev/null || true
wipefs -af "$DISK" >/dev/null
sfdisk --quiet "$DISK" <<EOF
label: gpt
size=2GiB, type=U, name=EFI
type=L, name=BunnyOS
EOF
sleep 1

say "Creating filesystems"
mkfs.vfat -F 32 -n EFI "$EFI_PART" >/dev/null
mkfs.btrfs -f -L bunnyos "$ROOT_PART" >/dev/null

say "Creating btrfs subvolumes"
mkdir -p "$MNT"
mount "$ROOT_PART" "$MNT"
for sv in @ @home @log @cache @build @snapshots; do
    btrfs subvolume create "$MNT/$sv" >/dev/null
done
umount "$MNT"

mount -o "$BTRFS_OPTS,subvol=@" "$ROOT_PART" "$MNT"
mkdir -p "$MNT/home" "$MNT/var/log" "$MNT/var/cache" "$MNT/var/tmp/portage" "$MNT/.snapshots" "$MNT/efi"
mount -o "$BTRFS_OPTS,subvol=@home"      "$ROOT_PART" "$MNT/home"
mount -o "$BTRFS_OPTS,subvol=@log"       "$ROOT_PART" "$MNT/var/log"
mount -o "$BTRFS_OPTS,subvol=@cache"     "$ROOT_PART" "$MNT/var/cache"
mount -o "$BTRFS_OPTS,subvol=@build"     "$ROOT_PART" "$MNT/var/tmp/portage"
mount -o "$BTRFS_OPTS,subvol=@snapshots" "$ROOT_PART" "$MNT/.snapshots"
mount "$EFI_PART" "$MNT/efi"

# ---------------------------------------------------------------- stage3 ----
say "Setting the clock"
chronyd -q 'server pool.ntp.org iburst' >/dev/null 2>&1 || true

say "Downloading the latest OpenRC stage3"
cd "$MNT"
STAGE3="$(wget -qO- "$MIRROR/latest-stage3-amd64-openrc.txt" | grep -o '^[^ #]*stage3-amd64-openrc-[^ ]*\.tar\.xz' | head -1)"
[ -n "$STAGE3" ] || fail "Couldn't find the latest stage3 on the Gentoo mirror."
wget -q --show-progress "$MIRROR/$STAGE3" "$MIRROR/$STAGE3.DIGESTS"
FILE="$(basename "$STAGE3")"

say "Verifying the download"
grep -A1 'SHA512 HASH' "$FILE.DIGESTS" | grep " $FILE\$" | head -1 > "$FILE.sha512"
[ -s "$FILE.sha512" ] || fail "Couldn't read the checksum file."
sha512sum -c "$FILE.sha512" >/dev/null || fail "The stage3 download is corrupted. Run the script again."

say "Unpacking"
tar xpf "$FILE" --xattrs-include='*.*' --numeric-owner -C "$MNT"
rm -f "$FILE" "$FILE.DIGESTS" "$FILE.sha512"

# ----------------------------------------------------------- portage setup --
say "Configuring Portage"
cat >> "$MNT/etc/portage/make.conf" <<EOF

# Added by vm-install-gentoo.sh
MAKEOPTS="-j$(nproc)"
FEATURES="getbinpkg binpkg-request-signature"
ACCEPT_LICENSE="-* @FREE @BINARY-REDISTRIBUTABLE"
GRUB_PLATFORMS="efi-64"
EOF
mkdir -p "$MNT/etc/portage/package.use"
echo "sys-kernel/installkernel dracut grub" > "$MNT/etc/portage/package.use/installkernel"

cp -L /etc/resolv.conf "$MNT/etc/"
mount --types proc /proc "$MNT/proc"
mount --rbind /sys "$MNT/sys" && mount --make-rslave "$MNT/sys"
mount --rbind /dev "$MNT/dev" && mount --make-rslave "$MNT/dev"
mount --bind /run "$MNT/run" && mount --make-slave "$MNT/run"

ROOT_UUID="$(blkid -s UUID -o value "$ROOT_PART")"
EFI_UUID="$(blkid -s UUID -o value "$EFI_PART")"

cat > "$MNT/etc/fstab" <<EOF
# <fs>                                     <mount>           <type> <options>                          <dump> <pass>
UUID=$ROOT_UUID  /                 btrfs  $BTRFS_OPTS,subvol=@           0 0
UUID=$ROOT_UUID  /home             btrfs  $BTRFS_OPTS,subvol=@home       0 0
UUID=$ROOT_UUID  /var/log          btrfs  $BTRFS_OPTS,subvol=@log        0 0
UUID=$ROOT_UUID  /var/cache        btrfs  $BTRFS_OPTS,subvol=@cache      0 0
UUID=$ROOT_UUID  /var/tmp/portage  btrfs  $BTRFS_OPTS,subvol=@build      0 0
UUID=$ROOT_UUID  /.snapshots       btrfs  $BTRFS_OPTS,subvol=@snapshots  0 0
UUID=$EFI_UUID                             /efi              vfat   umask=0077                         0 2
EOF

# ---------------------------------------------------------------- chroot ----
say "Installing the system (mostly prebuilt packages, this takes a while)"
cat > "$MNT/root/inside.sh" <<'INSIDE'
set -eu
. /etc/profile
emerge-webrsync -q
command -v getuto >/dev/null && getuto
echo "UTC" > /etc/timezone
emerge -q --config sys-libs/timezone-data
echo "en_US.UTF-8 UTF-8" > /etc/locale.gen
locale-gen -q
eselect locale set en_US.utf8 >/dev/null
. /etc/profile

emerge -q --noreplace sys-kernel/installkernel sys-kernel/gentoo-kernel-bin \
    sys-boot/grub sys-fs/btrfs-progs sys-fs/dosfstools \
    net-misc/dhcpcd app-admin/sysklogd app-admin/doas \
    dev-vcs/git dev-lang/rust-bin

echo 'hostname="bunny-vm"' > /etc/conf.d/hostname
rc-update add dhcpcd default
rc-update add sshd default
rc-update add sysklogd default

echo 'GRUB_CMDLINE_LINUX="rootflags=subvol=@"' >> /etc/default/grub
grub-install --target=x86_64-efi --efi-directory=/efi --removable
grub-mkconfig -o /boot/grub/grub.cfg

useradd -m -G wheel,users -s /bin/bash you
echo "permit :wheel" > /etc/doas.conf
chmod 600 /etc/doas.conf
INSIDE

chroot "$MNT" /bin/bash /root/inside.sh
printf 'root:%s\nyou:%s\n' "$PASSWORD" "$PASSWORD" | chroot "$MNT" chpasswd
rm -f "$MNT/root/inside.sh"

say "Unmounting"
cd /
umount -l "$MNT/dev" "$MNT/sys" "$MNT/proc" "$MNT/run" 2>/dev/null || true
umount -R "$MNT"

printf '\n\033[1;32m✓ Gentoo is installed.\033[0m\n'
echo "Next:"
echo "  1. Run: poweroff"
echo "  2. On your desktop, start the VM without the ISO: tools/bunny-vm.sh run"
echo "  3. Log in as \"you\". Then save a snapshot after shutting down:"
echo "     tools/bunny-vm.sh snap clean-install"
