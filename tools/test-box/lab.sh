#!/bin/sh
# Read-only experiments with Portage on the test machine, as the unprivileged
# runner user: a copy of /etc/portage (PORTAGE_CONFIGROOT) that can be changed
# freely, prebuilt-package downloads into a scratch folder, and only
# `emerge --pretend`. Nothing on the machine changes.
#
#   tools/test-box/lab.sh <experiment>
set -u
lab=$(mktemp -d)
trap 'rm -rf "$lab"' EXIT
mkdir -p "$lab/etc" "$lab/binpkgs" "$lab/tmp"
cp -a /etc/portage "$lab/etc/portage" 2>/dev/null || cp -r /etc/portage "$lab/etc/portage"
# The profile link is relative; point the copy at the real profile.
profile=$(realpath /etc/portage/make.profile)
rm -rf "$lab/etc/portage/make.profile"
ln -s "$profile" "$lab/etc/portage/make.profile"
# Prebuilt-package indexes go to a scratch folder too.
for f in "$lab"/etc/portage/binrepos.conf/*.conf; do
	[ -f "$f" ] && sed -i "s|^location *=.*|location = $lab/binhost|" "$f"
done
# Scratch places for what emerge --pretend writes.
cat >> "$lab/etc/portage/make.conf" <<EOT

# lab: scratch folders
PKGDIR="$lab/binpkgs"
PORTAGE_TMPDIR="$lab/tmp"
EOT
export PORTAGE_CONFIGROOT="$lab"
em() { echo; echo "\$ emerge $*"; emerge --color=n --ask=n "$@" 2>&1; echo "(exit $?)"; }
put() { mkdir -p "$(dirname "$lab/etc/portage/$1")"; printf '%s\n' "$2" >> "$lab/etc/portage/$1"; }

echo "profile: $profile"
grep -E '^(USE|FEATURES|EMERGE_DEFAULT_OPTS|BINPKG|ACCEPT)' "$lab/etc/portage/make.conf"
ls "$lab/etc/portage/binrepos.conf" 2>/dev/null && cat "$lab/etc/portage/binrepos.conf"/* 2>/dev/null

case "${1:-baseline}" in
baseline)
	em --pretend --verbose --update --deep --newuse @world | tail -15
	;;
hyprland)
	put package.accept_keywords/lab 'gui-apps/quickshell ~amd64'
	em --pretend --verbose --autounmask=y gui-wm/hyprland gui-apps/quickshell
	;;
desktop)
	# What [desktop] wayland = true writes, then a desktop on top.
	put package.use/lab 'sys-auth/pambase elogind'
	put package.use/lab 'sys-auth/seatd elogind'
	put package.accept_keywords/lab 'gui-apps/quickshell ~amd64'
	em --pretend --verbose --autounmask=y sys-auth/elogind sys-apps/dbus \
		gui-wm/sway gui-apps/quickshell x11-terms/kitty xfce-base/thunar
	;;
search)
	for p in hyprland mangowm quickshell sway; do
		echo "== $p"; ls -d /var/db/repos/*/*/"$p" 2>/dev/null
	done
	echo "== tree date: $(cat /var/db/repos/gentoo/metadata/timestamp.chk 2>/dev/null)"
	;;
esac
