#!/bin/bash
# Bunny Linux: the part of `bunny install` that runs inside the new system
# (chroot). bunny writes /root/bunny-install.env next to it, with:
#   HOST, USERNAME, TIMEZONE, KEYMAP, CMDLINE, PACKAGES, SERVICES, BOOT_SERVICES
# Packages come from Gentoo's binary host, so very little is compiled.
set -eu
. /etc/profile
. /root/bunny-install.env

say() { printf '\n==> %s\n' "$*"; }

say "Getting the package list"
emerge-webrsync -q
# The keys that check prebuilt packages are signed by Gentoo.
command -v getuto >/dev/null && getuto

say "Language, time zone and keyboard"
echo "$TIMEZONE" > /etc/timezone
# Current Gentoo reads the zone from this link (/etc/timezone alone no
# longer sets it).
ln -sf "../usr/share/zoneinfo/$TIMEZONE" /etc/localtime
echo "en_US.UTF-8 UTF-8" > /etc/locale.gen
locale-gen -q
eselect locale set en_US.utf8 >/dev/null
. /etc/profile
sed -i "s/^keymap=.*/keymap=\"$KEYMAP\"/" /etc/conf.d/keymaps

say "A console font that can draw ✓ ✗ ⚠"
# The console's usual font has no ✓ or ✗ (bunny's ✓ showed as "v"). kbd's
# eurlatgr has both (sharing the √ and × shapes) and ·, but no ⚠, so the
# copy made here also draws ⚠ with its "!". The live USB does the same.
font_src=/usr/share/consolefonts/eurlatgr.psfu.gz
font=/usr/local/share/consolefonts/bunny.psfu
[ -f "$font_src" ] || font_src="${font_src%.gz}"
if [ -f "$font_src" ]; then
    mkdir -p "${font%/*}"
    zcat -f "$font_src" > /tmp/bunny-font.psfu
    psfgettable /tmp/bunny-font.psfu /tmp/bunny-font.txt
    printf '0x021\tU+26A0\n' >> /tmp/bunny-font.txt
    psfaddtable /tmp/bunny-font.psfu /tmp/bunny-font.txt "$font"
    rm -f /tmp/bunny-font.psfu /tmp/bunny-font.txt
    sed -i "s|^#\?consolefont=.*|consolefont=\"$font\"|" /etc/conf.d/consolefont
    rc-update add consolefont boot
else
    # Not worth stopping the install for: the marks show as v and x.
    echo "Note: kbd's eurlatgr font is missing, so the console keeps its usual font."
fi

say "Installing the system (prebuilt packages; this takes a while)"
# shellcheck disable=SC2086  # a list of package names
emerge -q --noreplace $PACKAGES

# Gentoo builds its stage3 with a few USE flags turned off that its profile
# turns on (releng's package.use: curl without http2/http3/quic, libb2,
# gettext and portage-utils without openmp, because the stage1 compiler
# can't do it yet). Bring those in line now, from the prebuilt packages,
# with the same options bunny switch uses, so the first switch has nothing
# to rebuild.
emerge -q --update --deep --newuse @world

# Gentoo's announcements so far are older than this system: mark them read,
# so bunny only mentions new ones.
eselect news read --quiet all >/dev/null 2>&1 || true

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
