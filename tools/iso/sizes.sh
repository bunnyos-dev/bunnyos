#!/bin/sh
# sizes.sh: what takes the space in the live USB's system, as Markdown
# tables (tools/build-iso.sh writes them next to the ISO, and CI shows them
# on the run's page).
#
#   tools/iso/sizes.sh <the image's root folder>
#
# Sizes are before compression, in MB. Read-only: it only looks.

set -eu

root="${1:?Usage: $0 <root folder>}"
[ -d "$root/usr" ] || { echo "$root doesn't look like a system (no usr folder)." >&2; exit 1; }

# du's MB for each folder given (relative to the root), largest first,
# as table rows. Folders that don't exist are left out.
rows() {
    for dir in "$@"; do
        [ -e "$root/$dir" ] && [ ! -L "$root/$dir" ] || continue
        printf '%s\t%s\n' "$(du -sxm "$root/$dir" | cut -f1)" "$dir"
    done | sort -rn | awk -F'\t' '{ printf "| `%s` | %s |\n", $2, $1 }'
}

# The biggest folders directly inside one folder.
biggest_in() {
    dir="$1"; count="$2"
    [ -d "$root/$dir" ] || return 0
    echo
    echo "**Biggest in \`/$dir\`**"
    echo
    echo "| Folder | MB |"
    echo "|---|---:|"
    # shellcheck disable=SC2046  # folder names from find, no spaces
    rows $(cd "$root" && find "$dir" -mindepth 1 -maxdepth 1 -type d | sort) | head -n "$count"
}

modules="$(cd "$root" && find lib/modules usr/lib/modules -mindepth 1 -maxdepth 1 -type d 2>/dev/null | head -1)"
firmware=lib/firmware
[ -L "$root/lib" ] && firmware=usr/lib/firmware

echo "### The live system, by folder (before compression)"
echo
echo "| Folder | MB |"
echo "|---|---:|"
# shellcheck disable=SC2046
rows $(cd "$root" && find . -mindepth 1 -maxdepth 1 -type d | sed 's|^\./||' | sort)
total="$(du -sxm "$root" | cut -f1)"
echo "| **All** | **$total** |"

biggest_in usr 10
biggest_in usr/lib 25
biggest_in usr/libexec 10
biggest_in usr/share 20
biggest_in "$firmware" 40
biggest_in "$firmware/intel" 15
[ -n "$modules" ] && biggest_in "$modules/kernel" 15
[ -n "$modules" ] && biggest_in "$modules/kernel/drivers" 30

echo
echo "**Biggest packages** (as Portage recorded them when installing)"
echo
echo "| Package | MB |"
echo "|---|---:|"
for pkg in "$root"/var/db/pkg/*/*; do
    [ -f "$pkg/SIZE" ] || continue
    printf '%s\t%s\n' "$(cat "$pkg/SIZE")" "${pkg#"$root"/var/db/pkg/}"
done | sort -rn | head -40 | awk -F'\t' '{ printf "| %s | %.1f |\n", $2, $1 / 1048576 }'

echo
echo "**Biggest files**"
echo
echo "| File | MB |"
echo "|---|---:|"
(cd "$root" && find . -xdev -type f -size +4M -printf '%s\t%P\n') \
    | sort -rn | head -30 | awk -F'\t' '{ printf "| `/%s` | %.1f |\n", $2, $1 / 1048576 }'

if [ -n "$modules" ]; then
    echo
    plain="$(find "$root/$modules" -name '*.ko' | wc -l)"
    packed="$(find "$root/$modules" -name '*.ko.*' | wc -l)"
    echo "Kernel modules: $plain uncompressed, $packed compressed."
fi
packed="$(find "$root/$firmware" -type f \( -name '*.xz' -o -name '*.zst' \) | wc -l)"
all="$(find "$root/$firmware" -type f | wc -l)"
echo "Firmware files: $all, of which $packed compressed."
