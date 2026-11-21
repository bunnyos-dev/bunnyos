#!/bin/bash
# Bunny Linux live USB: the part of tools/build-iso.sh that runs inside the
# image (chroot). Installs the packages, sets up the USB's start-up, and
# leaves a kernel and startup image in /boot for build-iso.sh to take out.
set -eu
# shellcheck source=/dev/null
. /etc/profile

say() { printf '\n==> %s\n' "$*"; }

say "Getting the package list"
emerge-webrsync -q
command -v getuto >/dev/null && getuto

cat >> /etc/portage/make.conf <<'EOF'

# Bunny Linux live USB: prebuilt packages only, checked against Gentoo's keys.
FEATURES="getbinpkg binpkg-request-signature -news"
ACCEPT_LICENSE="-* @FREE @BINARY-REDISTRIBUTABLE"
EOF
mkdir -p /etc/portage/package.use
cat > /etc/portage/package.use/bunny-live <<'EOF'
# The startup image is made below with dracut's live-USB module, so the
# kernel package doesn't need to make its own.
sys-kernel/gentoo-kernel-bin -initramfs
sys-kernel/installkernel -dracut -grub
EOF

say "Installing the packages"
emerge -q --noreplace --usepkg --getbinpkg \
    sys-kernel/gentoo-kernel-bin \
    sys-kernel/linux-firmware \
    sys-kernel/dracut \
    sys-fs/cryptsetup \
    sys-fs/btrfs-progs \
    sys-fs/dosfstools \
    sys-apps/gptfdisk \
    net-misc/dhcpcd \
    net-wireless/iwd \
    net-misc/chrony \
    dev-vcs/git

say "Start-up: log in as root on the screen, with the welcome"
# A live USB has no password: the person at the keyboard owns it.
passwd -d root >/dev/null
sed -i 's|^c1:12345:respawn:/sbin/agetty .*|c1:12345:respawn:/sbin/agetty --autologin root --noclear 38400 tty1 linux|' /etc/inittab
grep -q -- '--autologin root' /etc/inittab || { echo "Couldn't set up the automatic login." >&2; exit 1; }
echo 'hostname="bunny-live"' > /etc/conf.d/hostname
# The live system's root is in memory; an fstab with something in it keeps
# OpenRC from warning about an empty one at start-up.
echo 'tmpfs  /tmp  tmpfs  defaults,nosuid,nodev  0 0' >> /etc/fstab
echo "UTC" > /etc/timezone
emerge -q --config sys-libs/timezone-data
rc-update add dhcpcd default
rc-update add iwd default
# Quiet start-up: only warnings and errors.
sed -i 's/^#\?rc_verbose=.*/rc_verbose=NO/' /etc/rc.conf

say "The kernel and the live startup image"
kver="$(find /lib/modules -mindepth 1 -maxdepth 1 -printf '%f\n' | sort -V | tail -1)"
[ -n "$kver" ] || { echo "No kernel was installed." >&2; exit 1; }
kernel=""
for candidate in "/boot/vmlinuz-$kver" "/lib/modules/$kver/vmlinuz" "/usr/src/linux-$kver/arch/x86/boot/bzImage"; do
    if [ -f "$candidate" ]; then kernel="$candidate"; break; fi
done
[ -n "$kernel" ] || { echo "Couldn't find the kernel for $kver." >&2; exit 1; }
cp "$kernel" /boot/vmlinuz-live
dracut --force --no-hostonly --kver "$kver" \
    --add "dmsquash-live" --omit "plymouth" \
    --add-drivers "squashfs overlay loop iso9660 sr_mod usb_storage uas nvme virtio_blk virtio_scsi" \
    /boot/initramfs-live.img
rm -f /boot/vmlinuz-"$kver" /boot/initramfs-"$kver".img /boot/System.map-"$kver" /boot/config-"$kver"

say "Making it smaller"
# Things a live USB never needs: the package tree and downloads (bunny
# install fetches its own inside the new system), documentation, and
# Portage's caches.
rm -rf /var/db/repos/gentoo /var/cache/distfiles/* /var/cache/binpkgs/* \
    /usr/share/doc/* /usr/share/man/* /usr/share/info/* /usr/share/gtk-doc \
    /var/tmp/portage/* /var/log/emerge*.log /usr/src/linux-*
