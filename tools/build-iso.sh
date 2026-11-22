#!/bin/sh
# build-iso.sh: build the Bunny Linux live USB image (an ISO you write to a
# USB stick, or boot in the VM).
#
#   doas tools/build-iso.sh <bunny binary> [output.iso]
#   doas env PREVIEW=1 tools/build-iso.sh ...   (a preview image, for testing)
#
# A preview image records every bunny command (developer mode, without
# sending anything), and bunny install copies its recording into the new
# system so `bunny report` can send it from there. Release images don't.
#
# Needs root (it unpacks a system and runs commands inside it with chroot),
# a Linux computer, and: wget, tar, xz, sha512sum, mksquashfs
# (squashfs-tools), grub-mkrescue with GRUB's x86_64-efi files, xorriso and
# mtools. On Gentoo: sys-fs/squashfs-tools dev-libs/libisoburn sys-fs/mtools,
# and sys-boot/grub with GRUB_PLATFORMS="efi-64". CI builds it on Ubuntu
# (.github/workflows/iso.yml).
#
# What's in the image (docs/ROADMAP.md, "The live USB image"):
#   Gentoo's newest OpenRC base system (stage3), Gentoo's prebuilt kernel and
#   firmware, the disk tools bunny install uses (cryptsetup, btrfs-progs,
#   dosfstools, util-linux), networking (dhcpcd for Ethernet, iwd for Wi-Fi),
#   chrony, git, nano, and bunny. It boots (UEFI, GRUB) straight to a root
#   terminal with a welcome: what this is, `bunny install`, the Guide's URL,
#   and how to get online.
#
# Packages come from Gentoo's binary host, so nothing is compiled. A
# bootable image of a few hundred MB; the work folder needs about 6 GB.

set -eu

MIRROR=https://distfiles.gentoo.org/releases/amd64/autobuilds
LABEL=BUNNY
here="$(cd "$(dirname "$0")" && pwd)"

say()  { printf '\n==> %s\n' "$*"; }
fail() { printf '\n✗ %s\n' "$*" >&2; exit 1; }

[ $# -ge 1 ] || fail "Usage: $0 <bunny binary> [output.iso]"
bunny="$1"
out="${2:-bunny-linux-live.iso}"
preview="${PREVIEW:-0}"
[ "$(id -u)" = 0 ] || fail "Run this as root (doas $0 …): it unpacks a system and uses chroot."
[ -x "$bunny" ] || fail "$bunny isn't a bunny binary you can run. Build one first (cargo build --release)."
"$bunny" --version >/dev/null 2>&1 \
    || fail "$bunny doesn't run here. Build a static one (the release workflow's musl build)."
for tool in wget tar xz sha512sum mksquashfs grub-mkrescue xorriso mformat; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is missing. See the top of this script for what to install."
done
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac

work="${WORK:-$(mktemp -d /var/tmp/bunny-iso.XXXXXX)}"
root="$work/root"
iso="$work/iso"
mkdir -p "$root" "$iso/boot/grub" "$iso/LiveOS"

mounted=""
cleanup() {
    for dir in $mounted; do
        umount --recursive --lazy "$root/$dir" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

# ------------------------------------------------------------ base system ---
say "Downloading Gentoo's newest base system (stage3)"
listing="$(wget -qO- "$MIRROR/latest-stage3-amd64-openrc.txt")" \
    || fail "Couldn't reach Gentoo's mirror ($MIRROR)."
stage3="$(printf '%s\n' "$listing" | grep -v '^#' | awk '{print $1}' \
    | grep 'stage3-amd64-openrc-.*\.tar\.xz$' | head -1)"
[ -n "$stage3" ] || fail "Couldn't find the newest stage3 in the mirror's listing."
file="${stage3##*/}"
(
    cd "$work"
    wget -q "$MIRROR/$stage3" "$MIRROR/$stage3.DIGESTS"
    grep -A1 'SHA512 HASH' "$file.DIGESTS" | grep " $file\$" | head -1 > "$file.sha512"
    [ -s "$file.sha512" ] || fail "Couldn't read the checksum file."
    sha512sum -c "$file.sha512" >/dev/null || fail "The stage3 download is damaged. Run this again."
)

say "Unpacking"
tar xpf "$work/$file" --xattrs-include='*.*' --numeric-owner -C "$root"
rm -f "$work/$file" "$work/$file.DIGESTS" "$work/$file.sha512"

# ---------------------------------------------------- inside the image -----
say "Adding bunny and the USB's own files"
install -D -m 0755 "$bunny" "$root/usr/local/bin/bunny"
install -D -m 0755 "$here/iso/welcome.sh" "$root/usr/local/bin/welcome"
install -D -m 0755 "$here/iso/openrc-quiet.sh" "$root/usr/local/sbin/bunny-openrc"
install -D -m 0644 "$here/iso/bash_profile" "$root/root/.bash_profile"
install -D -m 0755 "$here/iso/inside.sh" "$root/root/build-inside.sh"
if [ "$preview" = 1 ]; then
    # Read by bunny (dev::PREVIEW): record every command on this USB.
    cat > "$root/etc/bunny-preview.toml" <<'EOF'
# Bunny Linux preview USB: bunny records every command here (developer
# mode). Nothing is sent from the USB; bunny install copies its recording
# into the new system, where `bunny report` shows it and asks before sending.
[dev]
report = true
EOF
fi

cp --dereference /etc/resolv.conf "$root/etc/"
mount --types proc /proc "$root/proc"; mounted="proc $mounted"
mount --rbind /sys "$root/sys"; mount --make-rslave "$root/sys"; mounted="sys $mounted"
mount --rbind /dev "$root/dev"; mount --make-rslave "$root/dev"; mounted="dev $mounted"
mount --bind /run "$root/run"; mount --make-slave "$root/run"; mounted="run $mounted"

say "Installing the USB's packages inside it (prebuilt; takes a while)"
chroot "$root" /bin/bash /root/build-inside.sh \
    || fail "Building inside the image failed (the message is above)."
rm -f "$root/root/build-inside.sh"
cleanup
mounted=""
: > "$root/etc/resolv.conf"

# ------------------------------------------------------------- the image ---
say "Making the image"
mv "$root/boot/vmlinuz-live" "$iso/boot/vmlinuz"
mv "$root/boot/initramfs-live.img" "$iso/boot/initramfs.img"
cp "$here/iso/grub.cfg" "$iso/boot/grub/grub.cfg"
version="$("$bunny" --version | awk '{print $NF}')"
kind="live USB"
[ "$preview" = 1 ] && kind="live USB (preview)"
printf 'Bunny Linux %s, bunny %s, built %s from %s\n' \
    "$kind" "$version" "$(date -u +%Y-%m-%d)" "$file" > "$iso/bunny-live.txt"
cp "$iso/bunny-live.txt" "$root/etc/bunny-live"

mksquashfs "$root" "$iso/LiveOS/squashfs.img" -comp zstd -Xcompression-level 19 \
    -noappend -quiet -e boot/grub
grub-mkrescue --output="$out" "$iso" -- -volid "$LABEL" >/dev/null 2>&1 \
    || grub-mkrescue --output="$out" "$iso" -- -volid "$LABEL"
(cd "$(dirname "$out")" && sha256sum "$(basename "$out")" > "$(basename "$out").sha256")

size="$(du -h "$out" | cut -f1)"
say "Done: $out ($size)"
echo "  Its SHA256 is in $out.sha256."
echo "  Try it:        tools/bunny-vm.sh run $out"
echo "  Write a USB:   doas dd if=$out of=/dev/sdX bs=4M status=progress oflag=sync"
echo "  (The work folder $work can be deleted.)"
