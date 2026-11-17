#!/bin/sh
# Prints one version's section of CHANGELOG.md, for the release notes.
#   tools/release-notes.sh 0.1.0       -> the "## [0.1.0] - <date>" section
#   tools/release-notes.sh Unreleased  -> the "## [Unreleased]" section
# Fails (with a plain message) if there's no such section, so a release
# can't go out without notes.

version="$1"
changelog="${2:-CHANGELOG.md}"

if [ -z "$version" ]; then
    echo "Usage: $0 <version> [changelog]" >&2
    exit 2
fi

notes="$(awk -v v="$version" '
    /^## \[/ {
        if (found) exit
        if (index($0, "## [" v "]") == 1) { found = 1; next }
    }
    found { print }
' "$changelog")"

if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
    echo "CHANGELOG.md has no section for $version (a heading like \"## [$version] - <date>\")." >&2
    echo "Add one (see docs/RELEASING.md), then release again." >&2
    exit 1
fi
printf '%s\n' "$notes"
