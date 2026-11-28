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
# GURU (where Hyprland, Mango and Quickshell live now), cloned into the
# scratch folder and added only to the copy's repos.conf.
guru() {
	git clone -q --depth 1 https://github.com/gentoo-mirror/guru "$lab/guru"
	mkdir -p "$lab/etc/portage/repos.conf"
	printf '[guru]\nlocation = %s\nmasters = gentoo\nauto-sync = no\n' "$lab/guru" > "$lab/etc/portage/repos.conf/lab-guru.conf"
	cat "$lab/guru/profiles/repo_name" "$lab/guru/metadata/layout.conf" 2>&1; emerge --info 2>&1 | grep -iE -A3 "guru|^Repositories"
	portageq get_repos / 2>&1
}
em() { echo; echo "\$ emerge $*"; emerge --color=n --ask=n "$@" 2>&1; echo "(exit $?)"; }
put() { mkdir -p "$(dirname "$lab/etc/portage/$1")"; printf '%s\n' "$2" >> "$lab/etc/portage/$1"; }

echo "profile: $profile"
grep -E '^(USE|FEATURES|EMERGE_DEFAULT_OPTS|BINPKG|ACCEPT)' "$lab/etc/portage/make.conf"
ls "$lab/etc/portage/binrepos.conf" 2>/dev/null && cat "$lab/etc/portage/binrepos.conf"/* 2>/dev/null

case "${1:-baseline}" in
baseline)
	em --pretend --verbose --update --deep --newuse @world | tail -15
	;;
desktop)
	# What [desktop] wayland = true writes, then a desktop on top.
	put package.use/lab 'sys-auth/pambase elogind'
	put package.use/lab 'sys-auth/seatd elogind'
	guru
	# GURU's packages are testing-only: accept them, as `bunny add` would.
	put package.accept_keywords/lab '*/*::guru ~amd64'
	em --pretend --verbose --autounmask=y sys-auth/elogind sys-apps/dbus \
		gui-wm/mangowm gui-apps/quickshell x11-terms/kitty xfce-base/thunar
	;;
profile-desktop|minimal-loop)
	guru
	put package.use/lab 'sys-auth/pambase elogind'
	put package.use/lab 'sys-auth/seatd elogind'
	put package.accept_keywords/lab '*/*::guru ~amd64'
	want="sys-auth/elogind sys-apps/dbus gui-wm/mangowm gui-apps/quickshell x11-terms/kitty xfce-base/thunar"
	if [ "$1" = profile-desktop ]; then
		rm "$lab/etc/portage/make.profile"
		ln -s /var/db/repos/gentoo/profiles/default/linux/amd64/23.0/desktop "$lab/etc/portage/make.profile"
	fi
	# emerge's own fix-up loop: accept every change it suggests, in the
	# copy only, and try again, up to 12 rounds. Shows what each round asks.
	round=1
	while [ $round -le 12 ]; do
		echo; echo "######## round $round"
		# shellcheck disable=SC2086
		out=$(CONFIG_PROTECT="-*" emerge --color=n --ask=n --pretend --verbose \
			--autounmask=y --autounmask-write=y --autounmask-continue=n $want 2>&1)
		status=$?
		echo "$out" | grep -E '^(=|>=|<|~)?[a-z0-9-]+/[^ ]+ [-a-zA-Z0-9_ ]+$|USE changes|keyword|REQUIRED_USE|any-of|conflict|slot|!!!|required by' | head -40
		if echo "$out" | grep -q '^\[ebuild\|^\[binary'; then
			if [ $status -eq 0 ]; then
				echo "== resolved: $(echo "$out" | grep -c '^\[binary') prebuilt, $(echo "$out" | grep -c '^\[ebuild') to compile"
				echo "$out" | grep '^\[ebuild' | head -40
				break
			fi
		fi
		round=$((round + 1))
	done
	echo; echo "== what the fix-ups wrote:"
	cat "$lab"/etc/portage/package.use/* "$lab"/etc/portage/package.accept_keywords/* 2>/dev/null | grep -v '^#' | sort -u
	;;
binhost)
	# Which flags Gentoo's prebuilt packages were built with (its index).
	curl -fsSL -o "$lab/Packages" https://distfiles.gentoo.org/releases/amd64/binpackages/23.0/x86-64/Packages
	python3 - "$lab/Packages" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
blocks = text.split("\n\n")
print("header:", "; ".join(l for l in blocks[0].splitlines() if l.split(":")[0] in ("PROFILE","USE","ARCH","PACKAGES","TIMESTAMP")))
want = ["xfce-base/libxfce4ui","x11-libs/gtk+","gui-libs/gtk","dev-qt/qtbase","dev-qt/qtdeclarative",
        "media-libs/mesa","x11-libs/cairo","media-libs/freetype","gui-libs/wlroots",
        "x11-libs/libxkbcommon","net-wireless/wpa_supplicant","media-video/ffmpeg","dev-qt/qtwayland"]
for w in want:
    print("==", w)
    for b in blocks[1:]:
        f = dict(l.split(": ",1) for l in b.splitlines() if ": " in l)
        cpv = f.get("CPV","")
        if re.match(re.escape(w)+r"-\d", cpv):
            print("  ", cpv, "| USE:", f.get("USE",""))
PY
	;;
search)
	guru
	for p in hyprland mangowm quickshell sway; do
		echo "== $p"; ls -d /var/db/repos/*/*/"$p" "$lab"/guru/*/"$p" 2>/dev/null
	done
	echo "== tree date: $(cat /var/db/repos/gentoo/metadata/timestamp.chk 2>/dev/null)"
	;;
esac
