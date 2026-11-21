#!/bin/sh
# convert-layout.sh: move an existing Gentoo on btrfs to the Bunny Linux layout.
#
# For systems where / is the top of the btrfs volume (no @ subvolume), like
# many hand-made installs. It makes the subvolumes bunny's undo needs:
#
#   @          /                  (a copy of the system as it is now)
#   @home      /home              @log       /var/log
#   @cache     /var/cache         @build     /var/tmp/portage
#   @snapshots /.snapshots        @swap      /swap (if there's a swapfile)
#
# Nothing is moved or deleted: the new subvolumes are btrfs copies (reflinks)
# that take seconds and almost no space, and the old system stays where it is.
# A boot menu entry, "Before the Bunny Linux layout", starts it if anything is
# wrong. Only `--finish`, run later from the new layout, deletes the old copy.
#
# Usage (as root):
#   convert-layout.sh              show the plan; change nothing
#   convert-layout.sh --apply      do it (asks you to type "convert")
#   convert-layout.sh --finish     after rebooting into the new layout and
#                                  checking it: delete the old copy (asks you
#                                  to type "delete")
#   --root DIR                     work on DIR instead of / (for testing on a
#                                  disk image; leaves the boot loader alone)

set -eu

ROOT=/
MODE=plan
while [ $# -gt 0 ]; do
    case "$1" in
        --apply)  MODE=apply ;;
        --finish) MODE=finish ;;
        --root)   shift; ROOT="${1:?--root needs a folder}" ;;
        -h|--help) sed -n '2,27p' "$0"; exit 0 ;;
        *) printf '✗ Unknown option: %s (see --help)\n' "$1" >&2; exit 1 ;;
    esac
    shift
done
ROOT="${ROOT%/}"
REAL=0; [ -z "$ROOT" ] && REAL=1
SUBVOLS="@home:/home @log:/var/log @cache:/var/cache @build:/var/tmp/portage @snapshots:/.snapshots"
OLD_ENTRY_BEGIN="### BEGIN convert-layout: the system before the Bunny Linux layout"
OLD_ENTRY_END="### END convert-layout"

say()  { printf '\n\033[1;35m==> %s\033[0m\n' "$*"; }
step() { printf '  • %s\n' "$*"; }
fail() { printf '\n\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || fail "Run this as root (doas sh $0 …)."
command -v btrfs >/dev/null || fail "btrfs isn't installed (sys-fs/btrfs-progs)."

# ------------------------------------------------------------ what's here ---
FSTYPE="$(findmnt -no FSTYPE --target "${ROOT:-/}")"
[ "$FSTYPE" = btrfs ] || fail "${ROOT:-/} isn't on btrfs (it's $FSTYPE). The Bunny Linux layout needs btrfs."
OPTS="$(findmnt -no OPTIONS --target "${ROOT:-/}")"
SUBVOL="$(printf '%s' "$OPTS" | tr ',' '\n' | sed -n 's/^subvol=//p')"
DEVICE="$(findmnt -no SOURCE --target "${ROOT:-/}" | sed 's/\[.*//')"
UUID="$(findmnt -no UUID --target "${ROOT:-/}")"
# The options the root is mounted with, minus the ones that pick a subvolume.
BASE_OPTS="$(printf '%s' "$OPTS" | tr ',' '\n' | grep -Ev '^(subvol|subvolid)=' | paste -sd, -)"

if [ "$MODE" = finish ]; then
    [ "$SUBVOL" = /@ ] || fail "--finish runs from the new layout. / is subvol=$SUBVOL now: reboot into the new layout first."
else
    case "$SUBVOL" in
        /) ;;
        /@) fail "This system already uses the Bunny Linux layout (/ is @). Nothing to do." ;;
        *) fail "/ is the subvolume $SUBVOL, not the top of the volume. This script only converts systems with / at the top." ;;
    esac
fi

# /boot must be its own filesystem (usually the EFI partition), as on an
# encrypted install. Then GRUB and the kernels live outside the btrfs volume
# and keep working as they are. With /boot inside /, GRUB reads its files
# from the top of the volume, which the conversion leaves behind, so GRUB
# itself would have to be reinstalled: not handled here yet.
if [ "$MODE" != finish ] && [ "$REAL" = 1 ]; then
    BOOT_UUID="$(findmnt -no UUID --mountpoint /boot 2>/dev/null || true)"
    [ -n "$BOOT_UUID" ] && [ "$BOOT_UUID" != "$UUID" ] || fail "/boot is part of / here, not its own partition. This script only converts
  systems where /boot is a separate partition (like the EFI partition on an
  encrypted install), because otherwise GRUB has to be reinstalled too.
  Nothing was changed."
fi

# Folders from the layout that are already their own filesystem (another
# disk, or tmpfs for /var/tmp/portage): left exactly as they are.
separate() {
    other="$(findmnt -no UUID --mountpoint "${ROOT}$1" 2>/dev/null || true)"
    fs="$(findmnt -no FSTYPE --mountpoint "${ROOT}$1" 2>/dev/null || true)"
    [ -n "$fs" ] && [ "$other" != "$UUID" ]
}
KEPT=""
WANTED=""
for pair in $SUBVOLS; do
    if separate "${pair#*:}"; then KEPT="$KEPT ${pair#*:}"; else WANTED="$WANTED $pair"; fi
done
SUBVOLS="${WANTED# }"

# Nested subvolumes inside / (they aren't part of a snapshot of /).
# Ones at a layout folder (like /home as a subvolume) are copied into that
# folder's @ subvolume instead, so they're left out here.
nested() {
    btrfs subvolume list -o "${ROOT:-/}" | sed 's/.* path //' | grep -v '^@' | while read -r sv; do
        layout=0
        for pair in $SUBVOLS; do [ "/$sv" = "${pair#*:}" ] && layout=1; done
        [ "$layout" = 1 ] || echo "$sv"
    done
    true
}

# Swapfiles on this filesystem: in use now, or listed in /etc/fstab. A
# swapfile can't stay in @: btrfs won't snapshot a subvolume with an active
# swapfile, which would break undo.
swapfiles() {
    {
        if [ "$REAL" = 1 ]; then
            awk 'NR > 1 && $2 == "file" { print $1 }' /proc/swaps
        fi
        awk '!/^[[:space:]]*#/ && $3 == "swap" && $1 ~ /^\// { print $1 }' "${ROOT}/etc/fstab" 2>/dev/null
    } | sort -u | while read -r f; do
        [ -f "${ROOT}$f" ] || continue
        [ "$REAL" = 0 ] || [ "$(findmnt -no UUID --target "$f" 2>/dev/null)" = "$UUID" ] || continue
        echo "$f"
    done
    true
}

# Whether a swapfile is in use right now.
swap_active() {
    [ "$REAL" = 1 ] && awk -v f="$1" 'NR > 1 && $1 == f { found = 1 } END { exit !found }' /proc/swaps
}

# ------------------------------------------------------------------ finish --
if [ "$MODE" = finish ]; then
    TOP=/run/convert-layout-top
    mkdir -p "$TOP"
    mountpoint -q "$TOP" || mount -o subvolid=5 "$DEVICE" "$TOP"
    say "The old system's files at the top of the volume"
    LEFT="$(cd "$TOP" && for f in * .[!.]*; do [ -e "$f" ] || continue; case "$f" in @*) ;; *) echo "$f";; esac; done)"
    if [ -z "$LEFT" ]; then
        echo "  Nothing left to delete."
        umount "$TOP"; exit 0
    fi
    printf '%s\n' "$LEFT" | sed 's/^/  /'
    echo
    echo "These are the old system's files. The new layout doesn't use them, and"
    echo "after this, \"Before the Bunny Linux layout\" can't start any more."
    printf 'Type delete to remove them: '
    read -r answer
    [ "$answer" = delete ] || { umount "$TOP"; fail "Nothing was deleted."; }
    # Nested subvolumes first, inside-out, then everything else.
    btrfs subvolume list -o "$TOP" | sed 's/.* path //' | grep -v '^@' | sort -r | while read -r sv; do
        btrfs subvolume delete "${TOP:?}/$sv" >/dev/null
    done
    printf '%s\n' "$LEFT" | while read -r f; do
        [ -n "$f" ] && rm -rf "${TOP:?}/${f:?}"
    done
    if [ "$REAL" = 1 ] && mountpoint -q /boot && [ -f /boot/grub/custom.cfg ]; then
        sed -i "/^$OLD_ENTRY_BEGIN\$/,/^$OLD_ENTRY_END\$/d" /boot/grub/custom.cfg
    fi
    umount "$TOP"
    printf '\n\033[1;32m✓ The old copy is gone; the space is free again.\033[0m\n'
    exit 0
fi

# -------------------------------------------------------------------- plan --
NESTED="$(nested)"
SWAPS="$(swapfiles)"
for sv in @ $SUBVOLS @swap; do
    name="${sv%%:*}"
    [ ! -e "${ROOT}/$name" ] || fail "${ROOT:-}/$name already exists, maybe from an unfinished attempt. Look at it, delete it if it's leftover (btrfs subvolume delete ${ROOT:-}/$name), and run this again."
done

say "Plan: ${ROOT:-/} (btrfs on $DEVICE) → the Bunny Linux layout"
step "Copy the whole system as it is now into a new subvolume, @ (btrfs snapshot: instant)."
for pair in $SUBVOLS; do
    sv="${pair%%:*}"; dir="${pair#*:}"
    if [ "$sv" = @snapshots ]; then
        step "Create $sv for $dir, where bunny keeps saved versions."
    elif [ "$sv" = @build ]; then
        step "Create $sv for $dir (compile scratch space; starts empty)."
    else
        step "Create $sv for $dir, and copy $dir into it (reflinks: instant, no extra space)."
    fi
done
if [ -n "$NESTED" ]; then
    printf '%s\n' "$NESTED" | while read -r sv; do
        step "/$sv is a subvolume inside /, so it isn't in the copy: copy its files into @ as a normal folder."
    done
fi
for dir in $KEPT; do
    step "$dir is already its own filesystem ($(findmnt -no FSTYPE --mountpoint "${ROOT}$dir")), so it stays as it is."
done
if [ -n "$SWAPS" ]; then
    printf '%s\n' "$SWAPS" | while read -r f; do
        size="$(stat -c %s "${ROOT}$f")"
        step "Swap: move $f ($((size / 1024 / 1024)) MB) to its own subvolume, @swap (/swap/swapfile), so it can't block saved versions."
    done
    step "Add a small boot service, swapfiles, that turns swap on once /swap is mounted."
fi
step "Write @'s /etc/fstab for the new layout (other lines, like /boot or network shares, stay)."
if [ "$REAL" = 1 ]; then
    step "Boot loader: add rootflags=subvol=@ to GRUB_CMDLINE_LINUX and run grub-mkconfig."
    step "Add a boot menu entry, \"Before the Bunny Linux layout\", that starts the old system."
else
    step "(Testing with --root: the boot loader is left alone.)"
fi
echo
echo "Nothing is moved or deleted: the old system stays as it is until you run --finish."

if [ "$MODE" = plan ]; then
    echo
    echo "This was only the plan; nothing changed. To do it: $0 --apply"
    exit 0
fi

# ------------------------------------------------------------------- apply --
echo
printf 'Type convert to go ahead: '
read -r answer
[ "$answer" = convert ] || fail "Cancelled. Nothing was changed."

# If anything fails from here on, say where things stand.
BOOT_SWITCHED=0
stopped() {
    if [ "$BOOT_SWITCHED" = 1 ]; then
        printf '\n\033[1;31m✗ Stopped after the boot menu was switched to the new layout.\033[0m\n' >&2
        printf 'Don'"'"'t delete @: the computer starts from it now. Restart and check it works;\n' >&2
        printf 'only the "Before the Bunny Linux layout" entry may be missing.\n' >&2
        return
    fi
    made=""
    for sv in @ $SUBVOLS @swap; do
        name="${sv%%:*}"
        [ -e "${ROOT}/$name" ] && made="$made ${ROOT}/$name"
    done
    printf '\n\033[1;31m✗ Stopped partway.\033[0m Your old system is untouched and still starts as before.\n' >&2
    if [ -n "$made" ]; then
        printf 'To try again, first delete what was made so far:\n  btrfs subvolume delete%s\n' "$made" >&2
    fi
    if [ -n "$SWAPS" ]; then
        printf 'Swap was turned off; it comes back on at the next restart (or: swapon -a).\n' >&2
    fi
}
trap 'status=$?; [ "$status" = 0 ] || stopped' EXIT

SWAP_SIZE=0
for f in $SWAPS; do
    SWAP_SIZE="$(stat -c %s "${ROOT}$f")"
    if swap_active "$f"; then
        say "Turning off swap ($f)"
        swapoff "$f"
    fi
done

say "Copying the system into @"
sync
btrfs subvolume snapshot "${ROOT:-/}" "${ROOT}/@" >/dev/null
NEW="${ROOT}/@"

for pair in $SUBVOLS; do
    sv="${pair%%:*}"; dir="${pair#*:}"
    say "Creating $sv ($dir)"
    btrfs subvolume create "${ROOT}/$sv" >/dev/null
    # If the folder was a subvolume, the copy of / has a read-only
    # placeholder there: replace it with a normal folder.
    rmdir "${NEW:?}$dir" 2>/dev/null || true
    mkdir -p "${NEW:?}$dir"
    if [ -d "${ROOT}$dir" ]; then
        chown --reference="${ROOT}$dir" "${NEW:?}$dir"
        chmod --reference="${ROOT}$dir" "${NEW:?}$dir"
    fi
    case "$sv" in
        @snapshots|@build) ;;
        *)
            if [ -d "${ROOT}$dir" ]; then
                cp -a --reflink=always "${ROOT}$dir/." "${ROOT}/$sv/"
            fi
            ;;
    esac
    # In @, the folder becomes an empty mount point for the subvolume.
    find "${NEW:?}$dir" -mindepth 1 -delete
done

if [ -n "$NESTED" ]; then
    printf '%s\n' "$NESTED" | while read -r sv; do
        say "Copying /$sv into @ as a normal folder"
        # The copy of / has a read-only placeholder where the subvolume was.
        rmdir "${NEW:?}/${sv:?}" 2>/dev/null || true
        mkdir -p "${NEW:?}/$sv"
        chown --reference="${ROOT}/$sv" "${NEW:?}/$sv"
        chmod --reference="${ROOT}/$sv" "${NEW:?}/$sv"
        cp -a --reflink=always "${ROOT}/$sv/." "${NEW:?}/$sv/"
    done
fi

SWAP_LINE=""
if [ "$SWAP_SIZE" -gt 0 ]; then
    say "Making the new swapfile in @swap"
    btrfs subvolume create "${ROOT}/@swap" >/dev/null
    btrfs filesystem mkswapfile --size "$SWAP_SIZE" "${ROOT}/@swap/swapfile" >/dev/null
    mkdir -p "${NEW:?}/swap"
    for f in $SWAPS; do rm -f "${NEW:?}${f:?}"; done
    SWAP_LINE="/swap/swapfile  none  swap  defaults  0 0"

    # OpenRC's swap service runs before localmount, so /swap isn't mounted
    # yet and the swapfile is skipped without a word. This small service
    # runs swapon again once everything in fstab is mounted.
    mkdir -p "${NEW:?}/etc/init.d"
    cat > "${NEW:?}/etc/init.d/swapfiles" <<'EOF'
#!/sbin/openrc-run
# Added by Bunny Linux's convert-layout.sh.
# Turns on swapfiles that live on their own subvolume (/swap/swapfile).
# OpenRC's own swap service runs before those are mounted.

description="Turn on swapfiles once all filesystems are mounted"

depend() {
	need localmount
	after swap
	keyword -docker -podman -jail -lxc -openvz -prefix -systemd-nspawn -vserver -wsl
}

start() {
	ebegin "Activating swapfiles"
	swapon -a >/dev/null 2>&1
	eend 0
}
EOF
    chmod 755 "${NEW:?}/etc/init.d/swapfiles"
    mkdir -p "${NEW:?}/etc/runlevels/boot"
    ln -sf /etc/init.d/swapfiles "${NEW:?}/etc/runlevels/boot/swapfiles"
fi

say "Writing @'s /etc/fstab"
FSTAB="${NEW:?}/etc/fstab"
cp "$FSTAB" "$FSTAB.before-bunny-layout"
SWAPS_FOR_AWK="$(printf '%s ' $SWAPS)"
awk -v uuid="$UUID" -v opts="$BASE_OPTS" -v subvols="$SUBVOLS" -v swapline="$SWAP_LINE" \
    -v oldswaps="$SWAPS_FOR_AWK" -v swapdone=0 '
    BEGIN {
        n = split(oldswaps, os, " ")
        m = split(subvols, sv, " ")
        for (i = 1; i <= m; i++) { split(sv[i], p, ":"); replaced[p[2]] = 1 }
        if (swapline != "") replaced["/swap"] = 1
    }
    /^[[:space:]]*#/ || NF < 2 { print; next }
    # An old line for a folder that now gets its @ subvolume (like a /home
    # subvolume mounted on its own): replaced by the new line.
    ($2 in replaced) { print "# (replaced by the Bunny Linux layout) " $0; next }
    $2 == "/" {
        printf "UUID=%s  /  btrfs  %s,subvol=@  0 0\n", uuid, opts
        m = split(subvols, sv, " ")
        for (i = 1; i <= m; i++) {
            split(sv[i], p, ":")
            printf "UUID=%s  %s  btrfs  %s,subvol=%s  0 0\n", uuid, p[2], opts, p[1]
        }
        if (swapline != "") {
            printf "UUID=%s  /swap  btrfs  %s,subvol=@swap  0 0\n", uuid, opts
        }
        next
    }
    {
        for (i = 1; i <= n; i++) if ($1 == os[i] && $3 == "swap") {
            if (!swapdone) { print swapline; swapdone = 1 }
            next
        }
        print
    }
    END { if (swapline != "" && !swapdone) print swapline }
' "$FSTAB.before-bunny-layout" > "$FSTAB"

if [ "$REAL" = 1 ]; then
    say "Boot loader"
    for grubconf in /etc/default/grub "${NEW:?}/etc/default/grub"; do
        if grep -q 'rootflags=subvol=@' "$grubconf"; then continue; fi
        if grep -q '^GRUB_CMDLINE_LINUX=' "$grubconf"; then
            sed -i 's/^GRUB_CMDLINE_LINUX=\(["'"'"']\)/GRUB_CMDLINE_LINUX=\1rootflags=subvol=@ /' "$grubconf"
        else
            echo 'GRUB_CMDLINE_LINUX="rootflags=subvol=@"' >> "$grubconf"
        fi
        grep -q 'rootflags=subvol=@' "$grubconf" || fail "Couldn't add rootflags=subvol=@ to $grubconf. Add it to GRUB_CMDLINE_LINUX by hand, then run grub-mkconfig -o /boot/grub/grub.cfg."
    done
    grub-mkconfig -o /boot/grub/grub.cfg >/dev/null 2>&1 || fail "grub-mkconfig failed. Your old system is untouched; run it by hand to see why."
    BOOT_SWITCHED=1

    # "Before the Bunny Linux layout": the newest kernel, with the old root.
    KERNEL="$(ls -t /boot/vmlinuz-* 2>/dev/null | grep -v '\.old$' | head -1)"
    VERSION="${KERNEL#/boot/vmlinuz-}"
    ESP_UUID="$BOOT_UUID"
    CMDLINE="$(sed 's/BOOT_IMAGE=[^ ]* //; s/rootflags=[^ ]* \{0,1\}//' /proc/cmdline)"
    if [ -n "$KERNEL" ] && [ -f "/boot/initramfs-$VERSION.img" ] && [ -n "$ESP_UUID" ]; then
        CUSTOM=/boot/grub/custom.cfg
        touch "$CUSTOM"
        sed -i "/^$OLD_ENTRY_BEGIN\$/,/^$OLD_ENTRY_END\$/d" "$CUSTOM"
        cat >> "$CUSTOM" <<EOF
$OLD_ENTRY_BEGIN
menuentry 'Before the Bunny Linux layout' --class bunny {
    search --no-floppy --fs-uuid --set=root $ESP_UUID
    linux /vmlinuz-$VERSION $CMDLINE rootflags=subvolid=5
    initrd /initramfs-$VERSION.img
}
$OLD_ENTRY_END
EOF
        [ -x /etc/grub.d/41_custom ] || echo "  Note: /etc/grub.d/41_custom isn't there, so the extra entry won't show."
    else
        echo "  Note: couldn't find the kernel on /boot, so there's no \"Before the Bunny Linux layout\" entry."
    fi
fi

trap - EXIT
printf '\n\033[1;32m✓ The Bunny Linux layout is ready.\033[0m\n'
if [ "$REAL" = 1 ]; then
    echo "Next:"
    echo "  1. Reboot. It starts from @ by itself."
    echo "  2. Check:  findmnt -no OPTIONS /   (should include subvol=/@)"
    echo "  3. If anything's wrong: hold Esc while starting, pick \"Before the Bunny Linux layout\"."
    echo "  4. Once you're happy: $0 --finish  (deletes the old copy)"
fi
