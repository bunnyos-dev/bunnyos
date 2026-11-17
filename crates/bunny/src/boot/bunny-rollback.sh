#!/bin/sh
# BunnyOS: undo a change from the boot menu.
#
# Runs before the system is mounted. Does nothing unless the kernel command
# line has bunny.rollback=<generation> (only the boot menu's "undo" entry
# adds it). Then it does what `bunny rollback` does: saves the current system
# as a generation, moves it aside, and makes a copy of <generation> the new
# system. If any step fails, the current system is left (or put back) as it
# was and the boot continues normally.
#
# This file is sourced by dracut, so it must never call `exit`.

# shellcheck disable=SC1091  # provided by dracut inside the startup image
type getarg >/dev/null 2>&1 || . /lib/dracut-lib.sh

bunny_rollback() {
    bunny_target="$(getarg bunny.rollback=)"
    [ -n "$bunny_target" ] || return 0
    case "$bunny_target" in
        bunny-*) ;;
        *) warn "bunny: \"$bunny_target\" isn't a generation name; booting normally"; return 0 ;;
    esac
    case "$bunny_target" in
        */* | *..*) warn "bunny: \"$bunny_target\" isn't a generation name; booting normally"; return 0 ;;
    esac

    # shellcheck disable=SC2154  # $root is set by dracut ("block:/dev/...")
    bunny_dev="${root#block:}"
    bunny_top=/run/bunny-top
    mkdir -p "$bunny_top"
    if ! mount -t btrfs -o subvolid=5 "$bunny_dev" "$bunny_top"; then
        warn "bunny: couldn't open the disk to undo the change; booting normally"
        return 0
    fi

    if [ ! -d "$bunny_top/@snapshots/$bunny_target" ]; then
        warn "bunny: generation $bunny_target is gone; booting normally"
        umount "$bunny_top"
        return 0
    fi

    bunny_stamp="$(date -u +%Y%m%d-%H%M%S)"
    bunny_before="bunny-$bunny_stamp-before-rollback"
    info "bunny: undoing the change, going back to $bunny_target"

    if ! btrfs subvolume snapshot -r "$bunny_top/@" "$bunny_top/@snapshots/$bunny_before" >/dev/null; then
        warn "bunny: couldn't save the current system first, so nothing was undone"
        umount "$bunny_top"
        return 0
    fi
    if ! mv "$bunny_top/@" "$bunny_top/@replaced-$bunny_stamp"; then
        warn "bunny: couldn't move the current system aside, so nothing was undone"
        umount "$bunny_top"
        return 0
    fi
    if ! btrfs subvolume snapshot "$bunny_top/@snapshots/$bunny_target" "$bunny_top/@" >/dev/null; then
        mv "$bunny_top/@replaced-$bunny_stamp" "$bunny_top/@"
        warn "bunny: couldn't restore $bunny_target; the current system was put back"
        umount "$bunny_top"
        return 0
    fi

    # Notes, so `bunny rollback list` can say what happened.
    printf 'kind = "rollback"\nsummary = ""\nto = "%s"\n' "$bunny_target" \
        > "$bunny_top/@snapshots/$bunny_before.toml"
    printf 'last = "rollback"\nat = "%s"\nto = "%s"\n' "$bunny_stamp" "$bunny_target" \
        > "$bunny_top/@snapshots/bunny-state.toml"

    umount "$bunny_top"
    info "bunny: done; starting the restored system"
}

bunny_rollback
