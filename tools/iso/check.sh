#!/bin/bash
# Bunny Linux live USB: checks, inside the finished image (chroot), that
# making it smaller didn't break anything the USB needs. tools/build-iso.sh
# runs it last.
#
# 1. Every program and library left has the libraries it needs.
# 2. The commands bunny install, the welcome and getting online use are
#    there and start.
# 3. The kernel's modules, network and graphics firmware are there.
set -u

problems=0
problem() { printf '✗ %s\n' "$*" >&2; problems=$((problems + 1)); }

echo "Checking that every program and library has the libraries it needs"
while IFS= read -r -d '' file; do
    head -c 4 "$file" 2>/dev/null | grep -q 'ELF' || continue
    missing="$(ldd "$file" 2>/dev/null | grep 'not found' || true)"
    [ -z "$missing" ] || problem "$file needs a library that isn't there: $(echo "$missing" | awk '{print $1}' | tr '\n' ' ')"
done < <(find /bin /sbin /usr/bin /usr/sbin /usr/local/bin /usr/local/sbin \
    /usr/lib64 /usr/libexec /lib64 -xdev -type f \( -perm -u+x -o -name '*.so*' \) -print0 2>/dev/null)

echo "Checking the commands the USB uses"
# bunny install (on the USB itself; the rest runs inside the new system),
# the welcome, the console, and getting online.
for cmd in bunny bash sh chronyd wget sha512sum tar xz sfdisk wipefs lsblk \
    blkid findmnt mkfs.vfat mkfs.btrfs btrfs cryptsetup mount umount chroot \
    udevadm cp git nano dhcpcd iwd iwctl ip ping loadkeys setfont agetty \
    openrc rc-service modprobe; do
    command -v "$cmd" >/dev/null 2>&1 \
        || [ -x "/usr/libexec/$cmd" ] || [ -x "/usr/libexec/iwd/$cmd" ] \
        || problem "$cmd is missing."
done
for cmd in "bunny --version" "cryptsetup --version" "mkfs.btrfs --version" \
    "btrfs --version" "git --version" "wget --version" "tar --version" \
    "xz --version" "sfdisk --version" "lsblk --version" "dhcpcd --version" \
    "chronyd -v" "nano --version"; do
    $cmd >/dev/null 2>&1 || problem "\"$cmd\" doesn't run."
done

echo "Checking the kernel's modules and firmware"
kver="$(find /lib/modules -mindepth 1 -maxdepth 1 -printf '%f\n' | sort -V | tail -1)"
for module in iwlwifi rtw88_8822ce rtw89_8852ce r8169 mt7921e ath11k_pci ath10k_pci \
    brcmfmac e1000e igc amdgpu i915 nouveau btrfs dm_crypt squashfs overlay; do
    modinfo -k "$kver" "$module" >/dev/null 2>&1 || problem "The kernel module $module is missing."
done
for dir in iwlwifi* rtw88 rtw89 rtl_nic mediatek ath10k ath11k brcm amdgpu i915 nvidia; do
    # shellcheck disable=SC2086  # a pattern on purpose (iwlwifi-*)
    ls -d /lib/firmware/$dir >/dev/null 2>&1 || problem "Firmware /lib/firmware/$dir is missing."
done

if [ "$problems" -gt 0 ]; then
    echo "✗ $problems problem(s) in the finished image (above)." >&2
    exit 1
fi
echo "✓ The image has everything the USB needs."
