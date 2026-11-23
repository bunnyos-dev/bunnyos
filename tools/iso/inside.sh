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
    dev-vcs/git \
    dev-libs/libinput \
    x11-libs/libxkbcommon \
    media-libs/fontconfig \
    media-fonts/dejavu

say "Start-up: log in as root on the screen, with the welcome"
# A live USB has no password: the person at the keyboard owns it.
passwd -d root >/dev/null
sed -i 's|^c1:12345:respawn:/sbin/agetty .*|c1:12345:respawn:/sbin/agetty --autologin root --noclear 38400 tty1 linux|' /etc/inittab
grep -q -- '--autologin root' /etc/inittab || { echo "Couldn't set up the automatic login." >&2; exit 1; }
# Every start of the USB is a first login (its records are in memory), so
# PAM's lastlog module said "Welcome to your new account!" above the welcome
# each time. That's Gentoo's pambase (`pam_lastlog.so never` in
# system-login); on the USB, "Last login" means nothing either, so it goes.
sed -i '/^session[[:space:]].*pam_lastlog\.so/d' /etc/pam.d/system-login
if grep -q 'pam_lastlog' /etc/pam.d/system-login; then
    echo "Couldn't turn off the \"new account\" message." >&2; exit 1
fi
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

say "A console font that can draw ✓ ✗ ⚠"
# The console's usual font has no ✓ or ✗ (bunny's ✓ showed as "v"). kbd's
# eurlatgr has both (sharing the √ and × shapes) and ·, but no ⚠, so the
# copy made here also draws ⚠ with its "!".
font_src=/usr/share/consolefonts/eurlatgr.psfu.gz
font=/usr/local/share/consolefonts/bunny.psfu
[ -f "$font_src" ] || font_src="${font_src%.gz}"
[ -f "$font_src" ] || { echo "kbd's eurlatgr console font is missing." >&2; exit 1; }
mkdir -p "${font%/*}"
zcat -f "$font_src" > /tmp/bunny-font.psfu
psfgettable /tmp/bunny-font.psfu /tmp/bunny-font.txt
printf '0x021\tU+26A0\n' >> /tmp/bunny-font.txt
psfaddtable /tmp/bunny-font.psfu /tmp/bunny-font.txt "$font"
rm -f /tmp/bunny-font.psfu /tmp/bunny-font.txt
for mark in 2713 2717 26a0 00b7; do
    psfgettable "$font" - | grep -qi "U+$mark" \
        || { echo "The console font can't draw U+$mark." >&2; exit 1; }
done
sed -i "s|^#\?consolefont=.*|consolefont=\"$font\"|" /etc/conf.d/consolefont
grep -q "^consolefont=\"$font\"" /etc/conf.d/consolefont \
    || { echo "Couldn't set the console font." >&2; exit 1; }
rc-update add consolefont boot

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

# Making it smaller (removing what the USB doesn't need) is
# tools/iso/smaller.sh, which build-iso.sh runs right after this script.
