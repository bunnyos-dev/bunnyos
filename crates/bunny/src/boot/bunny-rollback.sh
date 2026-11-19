#!/bin/sh
# Bunny Linux: undo a change from the boot menu.
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

    # Already undone to this generation (the entry was picked again, or GRUB
    # remembered it): nothing to do. Undoing twice would only add clutter.
    if [ -f "$bunny_top/@snapshots/bunny-state.toml" ] && \
        grep -q '^last = "rollback"' "$bunny_top/@snapshots/bunny-state.toml" && \
        grep -q "^to = \"$bunny_target\"" "$bunny_top/@snapshots/bunny-state.toml"; then
        info "bunny: this change was already undone; starting normally"
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

    # The config that matched that generation goes back too, so config and
    # system agree. Only a config under /etc (one in /home isn't rolled back).
    # The same goes for this computer's host file.
    bunny_applied="$bunny_top/@/var/lib/bunny/applied"
    if [ -f "$bunny_applied/config-path" ] && [ -f "$bunny_applied/bunny.toml" ]; then
        read -r bunny_config < "$bunny_applied/config-path"
        case "$bunny_config" in
            *..*) ;;
            /etc/*.toml)
                cp "$bunny_applied/bunny.toml" "$bunny_top/@$bunny_config" || \
                    warn "bunny: couldn't put $bunny_config back as it was"
                if [ -f "$bunny_applied/bunny.lock" ]; then
                    cp "$bunny_applied/bunny.lock" "$bunny_top/@${bunny_config%.toml}.lock" || true
                fi
                ;;
        esac
    fi
    # This computer's host file (hosts/<name>.toml), as applied with it.
    if [ -f "$bunny_applied/host-path" ] && [ -f "$bunny_applied/host.toml" ]; then
        read -r bunny_host < "$bunny_applied/host-path"
        case "$bunny_host" in
            *..*) ;;
            /etc/*.toml)
                if [ -d "$bunny_top/@${bunny_host%/*}" ]; then
                    cp "$bunny_applied/host.toml" "$bunny_top/@$bunny_host" || \
                        warn "bunny: couldn't put $bunny_host back as it was"
                else
                    warn "bunny: couldn't put $bunny_host back (its folder is gone)"
                fi
                ;;
        esac
    fi

    # The restored system brings back its own, older boot menu, whose undo
    # entry would go one change further back. Remove it: there's no "last
    # change" to undo until the next switch.
    bunny_cfg="$bunny_top/@/boot/grub/custom.cfg"
    if [ -f "$bunny_cfg" ]; then
        bunny_skip=0
        : > "$bunny_cfg.bunny-new"
        while IFS= read -r bunny_line || [ -n "$bunny_line" ]; do
            case "$bunny_line" in
                "### BEGIN bunny: undo entry"*) bunny_skip=1; continue ;;
                "### END bunny") bunny_skip=0; continue ;;
            esac
            [ "$bunny_skip" = 1 ] || printf '%s\n' "$bunny_line" >> "$bunny_cfg.bunny-new"
        done < "$bunny_cfg"
        mv "$bunny_cfg.bunny-new" "$bunny_cfg" || rm -f "$bunny_cfg.bunny-new"
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
