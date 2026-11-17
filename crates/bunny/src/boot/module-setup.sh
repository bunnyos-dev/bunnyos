#!/bin/bash
# BunnyOS: "Undo" from the boot menu. Installed by bunny ([boot] undo_entry).
# Adds a small step to the startup image (initramfs) that does nothing unless
# the boot menu passed bunny.rollback=<generation>.

check() {
    return 0
}

depends() {
    echo btrfs
}

install() {
    inst_multiple btrfs mv cp rm grep mkdir date printf umount mount
    # shellcheck disable=SC2154  # $moddir is set by dracut
    inst_hook pre-mount 90 "$moddir/bunny-rollback.sh"
}
