#!/bin/bash
# Bunny Linux: the part of `bunny install` that runs inside the new system
# (chroot). bunny writes /root/bunny-install.env next to it, with:
#   HOST, USERNAME, CMDLINE, PACKAGES, SERVICES, BOOT_SERVICES
# Packages come from Gentoo's binary host, so very little is compiled.
set -eu
. /etc/profile
. /root/bunny-install.env

say() { printf '\n==> %s\n' "$*"; }

say "Getting the package list"
emerge-webrsync -q
# The keys that check prebuilt packages are signed by Gentoo.
command -v getuto >/dev/null && getuto

say "Language and time zone"
echo "UTC" > /etc/timezone
emerge -q --config sys-libs/timezone-data
echo "en_US.UTF-8 UTF-8" > /etc/locale.gen
locale-gen -q
eselect locale set en_US.utf8 >/dev/null
. /etc/profile

say "Installing the system (prebuilt packages; this takes a while)"
# shellcheck disable=SC2086  # a list of package names
emerge -q --noreplace $PACKAGES

say "Host name, services and the boot menu"
echo "hostname=\"$HOST\"" > /etc/conf.d/hostname
for service in $SERVICES; do rc-update add "$service" default; done
for service in $BOOT_SERVICES; do rc-update add "$service" boot; done
# The /home disk: unlocked at boot with its key file.
if [ -f /root/bunny-dmcrypt.conf ]; then
    cat /root/bunny-dmcrypt.conf >> /etc/conf.d/dmcrypt
fi

{
    echo
    echo "# Added by bunny install"
    echo "GRUB_DISTRIBUTOR=\"Bunny Linux\""
    echo "GRUB_CMDLINE_LINUX=\"$CMDLINE\""
} >> /etc/default/grub
grub-install --target=x86_64-efi --efi-directory=/boot --bootloader-id="Bunny Linux"
# Also at the fallback path, for computers that forget their boot entries.
grub-install --target=x86_64-efi --efi-directory=/boot --removable
# The kernel was installed before cryptsetup and the dracut settings were in
# place: rebuild its startup image so it can unlock and mount the disk.
emerge -q --config sys-kernel/gentoo-kernel-bin
grub-mkconfig -o /boot/grub/grub.cfg

say "Your account"
useradd -m -G wheel,users,audio,video,input -s /bin/bash "$USERNAME"
# bunny wrote /etc/sudoers.d/bunny (admin rights for wheel); check it now
# that sudo is installed.
visudo -cf /etc/sudoers.d/bunny >/dev/null
