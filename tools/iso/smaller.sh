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

say "Making it smaller: translations, headers and caches"
# bunny and the USB speak English; glibc's locale.alias stays. Headers,
# static libraries and Portage's caches are only for building and
# installing packages, which happens inside the new system.
before="$(mb /usr/share/locale /usr/share/i18n /usr/include /var/cache/edb)"
find /usr/share/locale -mindepth 1 -maxdepth 1 ! -name locale.alias -exec rm -rf {} +
rm -rf /usr/share/i18n /usr/include /var/cache/edb/* /usr/share/gcc-data/*/*/info \
    /usr/share/gcc-data/*/*/man /usr/share/binutils-data/*/*/info \
    /usr/share/binutils-data/*/*/man
find /usr/lib /usr/lib64 -xdev \( -name '*.a' -o -name '*.la' \) -type f -delete
echo "Removed about $before MB (and the static libraries)."

say "Making it smaller: things stored twice"
# The prebuilt packages Portage downloaded (it keeps them after installing
# them), and the kernel's leftovers next to its modules: the USB starts the
# kernel build-iso.sh put on the ISO (boot/vmlinuz).
before="$(mb /var/cache/binhost /lib/modules/*/vmlinuz /lib/modules/*/System.map /lib/modules/*/config)"
rm -rf /var/cache/binhost/*
rm -f /lib/modules/*/vmlinuz /lib/modules/*/System.map /lib/modules/*/config
find /lib/modules -mindepth 2 -maxdepth 2 \( -name build -o -name source \) -xtype l -delete
echo "Removed about $before MB."

# Removes installed packages the way Portage would (every file and link
# its record lists, then its empty folders and the record itself), without
# Portage, so Portage and Python can go too. Takes patterns like
# dev-python/* or sys-apps/portage; ones that aren't installed are skipped.
drop_packages() {
    local pattern vdb line path dir
    for pattern in "$@"; do
        for vdb in /var/db/pkg/$pattern-[0-9]*/; do
            [ -f "$vdb/CONTENTS" ] || continue
            local dirs=()
            while IFS= read -r line; do
                case "$line" in
                    "obj "*) path="${line#obj }"; rm -f -- "${path% * *}" ;;
                    "sym "*) path="${line#sym }"; rm -f -- "${path%% -> *}" ;;
                    "dir "*) dirs+=("${line#dir }") ;;
                esac
            done < "$vdb/CONTENTS"
            if [ "${#dirs[@]}" -gt 0 ]; then
                printf '%s\n' "${dirs[@]}" | sort -r | while IFS= read -r dir; do
                    rmdir -- "$dir" 2>/dev/null || true
                done
            fi
            rm -rf -- "$vdb"
            path="${vdb#/var/db/pkg/}"
            echo "  removed ${path%/}"
        done
    done
}

say "Making it smaller: build tools, Portage, Python and Perl"
# The new system compiles and installs packages itself (it's Gentoo's full
# stage3), so the USB needs none of this. The compiler's runtime libraries
# (libstdc++, libgcc_s, which programs here use) stay; the compiler goes.
before="$(mb /usr)"
drop_packages \
    sys-devel/binutils sys-devel/bison sys-devel/flex sys-devel/m4 sys-devel/patch \
    dev-build/autoconf dev-build/autoconf-wrapper dev-build/autoconf-archive \
    dev-build/automake dev-build/automake-wrapper dev-build/libtool dev-build/make \
    dev-build/meson dev-build/meson-format-array dev-build/ninja dev-build/cmake \
    dev-build/gtk-doc-am dev-util/pkgconf dev-util/gperf dev-util/re2c \
    app-text/docbook-xml-dtd app-text/docbook-xsl-stylesheets \
    app-text/docbook-xsl-ns-stylesheets app-text/docbook-sgml-dtd \
    app-text/sgml-common app-text/build-docbook-catalog app-text/xmlto \
    app-text/po4a app-text/opensp dev-util/pahole \
    sys-apps/texinfo sys-apps/groff sys-apps/man-db sys-apps/man-pages \
    media-fonts/urw-fonts \
    'dev-perl/*' 'perl-core/*' dev-lang/perl \
    'app-portage/*' sys-apps/portage 'dev-python/*' dev-lang/python \
    dev-lang/python-exec dev-lang/python-exec-conf
rm -rf /usr/libexec/gcc /usr/x86_64-pc-linux-gnu/gcc-bin
for dir in /usr/lib/gcc/*/*/; do
    rm -rf "$dir"include "$dir"include-fixed "$dir"plugin "$dir"finclude "$dir"install-tools
    find "$dir" -maxdepth 1 -type f ! -name '*.so*' -delete
done
rm -rf /usr/share/gcc-data/*/*/python /usr/lib/python3* /usr/lib/python-exec
# Links left pointing at removed programs (cc, gcc, ld, python…).
find /usr/bin /usr/sbin /usr/local/bin -xtype l -delete
ldconfig
echo "Removed about $(( before - $(mb /usr) )) MB."
