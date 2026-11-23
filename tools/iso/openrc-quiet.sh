#!/bin/sh
# Bunny Linux live USB: starts OpenRC (inittab runs this instead of
# /sbin/openrc). The boot menu's normal entries have "quiet" on the kernel
# line, and then OpenRC's start-up lines ("* Starting dbus ... [ ok ]") are
# left out: only errors show. "Show start-up messages" has no "quiet", so
# everything shows as usual.
#
# dracut has mounted /proc by now; mount it if it hasn't, so the kernel line
# can be read (OpenRC's sysinit finds it mounted and carries on).
[ -r /proc/cmdline ] || mount -t proc proc /proc 2>/dev/null
case " $(cat /proc/cmdline 2>/dev/null) " in
    *" quiet "*)
        # OpenRC's own switch for --quiet: no "Starting …" or warnings,
        # errors still show.
        EINFO_QUIET=YES
        export EINFO_QUIET
        ;;
esac
exec /sbin/openrc "$@"
