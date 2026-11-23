#!/bin/bash
# Bunny Linux live USB: removes what the USB never needs, inside the image
# (chroot), after tools/iso/inside.sh has installed everything. Then
# tools/iso/check.sh checks nothing the USB uses went with it.
#
# What the USB must keep: starting on UEFI and BIOS, Ethernet and Wi-Fi
# (drivers and firmware), graphics (drivers and firmware, so the screen
# works), and everything `bunny install` runs on the USB itself: the disk
# tools, wget, tar and xz, chrony, git and bunny. The install's own work
# (emerge, GRUB, dracut, the first switch) runs inside the new system,
# which is Gentoo's full stage3 with its own compiler, Portage and Python.
set -eu

say() { printf '\n==> %s\n' "$*"; }

# MB used by the given paths (for the log).
mb() { du -scm "$@" 2>/dev/null | tail -1 | cut -f1; }

say "Checking the kernel can read an xz-packed image"
# build-iso.sh packs the system with xz; a kernel without it couldn't start
# the USB. Read now, before the kernel's sources go.
kver="$(find /lib/modules -mindepth 1 -maxdepth 1 -printf '%f\n' | sort -V | tail -1)"
config=""
for candidate in "/usr/src/linux-$kver/.config" "/lib/modules/$kver/build/.config" \
    "/lib/modules/$kver/config" /usr/src/linux-*/.config; do
    if [ -f "$candidate" ]; then config="$candidate"; break; fi
done
if [ -n "$config" ]; then
    grep -q '^CONFIG_SQUASHFS_XZ=y' "$config" \
        || { echo "The kernel ($kver) can't read xz-packed squashfs images." >&2; exit 1; }
    echo "✓ It can ($config)."
else
    echo "Note: couldn't find the kernel's settings; the boot check will tell."
fi

say "Making it smaller: the package tree, downloads and documentation"
# bunny install fetches its own package tree inside the new system.
rm -rf /var/db/repos/gentoo /var/cache/distfiles/* /var/cache/binpkgs/* \
    /usr/share/doc/* /usr/share/man/* /usr/share/info/* /usr/share/gtk-doc \
    /var/tmp/portage/* /var/log/emerge*.log /usr/src/linux-*
