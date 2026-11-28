#!/bin/sh
# Run a freshly built bunny's read-only commands on the test machine, as the
# unprivileged runner user. Nothing on the machine changes: anything bunny
# writes goes to a scratch folder.
#
#   tools/test-box/try-bunny.sh <path to bunny>
set -u
bunny=$1
chmod +x "$bunny"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
run() { echo; echo "\$ bunny $*"; "$bunny" "$@" 2>&1; echo "(exit $?)"; }

run --version
run hardware
run status
run diff
# The folder doesn't exist yet: import should create it.
run import -o "$scratch/new/folder/bunny.toml"
ls -la "$scratch/new/folder" 2>&1
grep -n -A6 '^\[hardware\]\|^\[system\]' "$scratch/new/folder/bunny.toml" 2>&1
run check "$scratch/new/folder/bunny.toml"
run plan "$scratch/new/folder/bunny.toml"
# "Did you mean": a typo, and a name that lives in another category.
printf 'packages = ["www-client/firefx", "app-misc/tree"]\n' > "$scratch/typo.toml"
run check "$scratch/typo.toml"
run search firefx
# bunny add on a fake root that sees the real package tree (with --root,
# bunny only prints commands). Names that aren't packages stop it early.
mkdir -p "$scratch/root/var/db" "$scratch/root/etc"
ln -s /var/db/repos "$scratch/root/var/db/repos"
cp -r /etc/portage "$scratch/root/etc/portage" 2>/dev/null
cp /etc/bunny/bunny.toml "$scratch/bunny.toml"
for name in app-misc/tree firefx; do
	echo; echo "\$ bunny add $name (fake root)"
	"$bunny" add --root "$scratch/root" --config "$scratch/bunny.toml" "$name" </dev/null 2>&1
	echo "(exit $?)"
done
