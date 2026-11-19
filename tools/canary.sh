#!/bin/sh
# The weekly canary check: runs bunny inside a fresh Gentoo (stage3 + the
# latest package tree) to catch changes in Gentoo that break bunny, before
# users hit them. Run by .github/workflows/canary.yml; usable by hand too:
#
#   docker run --rm --volumes-from portage -v "$PWD:/src:ro" \
#       gentoo/stage3 sh /src/tools/canary.sh /src/target/release/bunny
#
# Each check prints PASS or FAIL with a plain explanation. Exits non-zero
# if any check failed.

bunny="${1:-bunny}"
work="$(mktemp -d)"
failed=0

pass() { echo "PASS  $1"; }
fail() {
    echo "FAIL  $1"
    [ -n "$2" ] && echo "      $2"
    failed=1
}
section() { echo; echo "== $1"; }

section "Versions"
"$bunny" --version || fail "bunny runs" "the binary didn't start in this Gentoo"
emerge --version | head -1

section "Reading the system"
if "$bunny" import -o "$work/bunny.toml" >"$work/import.txt" 2>&1; then
    pass "bunny import writes a config"
else
    fail "bunny import writes a config" "$(tail -3 "$work/import.txt")"
fi
if "$bunny" check "$work/bunny.toml" >"$work/check.txt" 2>&1; then
    pass "bunny check accepts the imported config"
else
    fail "bunny check accepts the imported config" "$(tail -3 "$work/check.txt")"
fi
if "$bunny" plan "$work/bunny.toml" >"$work/plan.txt" 2>&1; then
    pass "bunny plan runs"
else
    fail "bunny plan runs" "$(tail -3 "$work/plan.txt")"
fi
if "$bunny" status "$work/bunny.toml" >"$work/status.txt" 2>&1; then
    pass "bunny status runs"
else
    fail "bunny status runs" "$(tail -3 "$work/status.txt")"
fi

section "The package tree"
if "$bunny" search tmux 2>&1 | grep -q "app-misc/tmux"; then
    pass "bunny search finds app-misc/tmux (package metadata format unchanged)"
else
    fail "bunny search finds app-misc/tmux" "the repository's metadata cache may have changed format"
fi

section "Reading emerge's output"
# The install list: bunny reads lines like "[ebuild  N    ] app-misc/tmux-3.5::gentoo".
emerge --pretend --update --deep --newuse --color=n app-misc/tmux >"$work/pretend.txt" 2>&1
if grep -Eq '^\[(ebuild|binary) [^]]*\] app-misc/tmux-[0-9]' "$work/pretend.txt"; then
    pass "emerge's install list still looks the way bunny reads it"
else
    fail "emerge's install list still looks the way bunny reads it" "$(grep -m3 'tmux' "$work/pretend.txt")"
fi

# bunny's Portage helper should give the same install list as emerge's text.
"$bunny" helper plan app-misc/tmux >"$work/helper.json" 2>"$work/helper-err.txt"
helper_names="$(grep -o '"name": "[^"]*"' "$work/helper.json" | sed 's/"name": "//; s/"$//' | sort)"
emerge_names="$(sed -n 's/^\[\(ebuild\|binary\)[^]]*\] \([^ ]*\)-[0-9].*/\2/p' "$work/pretend.txt" | sort)"
if [ -n "$helper_names" ] && [ "$helper_names" = "$emerge_names" ]; then
    pass "bunny's Portage helper agrees with emerge ($(echo "$helper_names" | wc -l) packages)"
else
    fail "bunny's Portage helper agrees with emerge" "helper: $(tr '\n' ' ' < "$work/helper.json" | cut -c1-300) | $(tail -3 "$work/helper-err.txt" | tr '\n' ' ')"
    echo "      emerge: $(echo "$emerge_names" | tr '\n' ' ')"
fi

# The same for what depclean would remove (on a fresh container, likely
# nothing, but both must agree and the helper must answer).
emerge --depclean --pretend --color=n >"$work/depclean.txt" 2>&1
"$bunny" helper depclean >"$work/helper-depclean.json" 2>"$work/helper-depclean-err.txt"
helper_gone="$(grep -o '"name": "[^"]*", "version": "[^"]*"' "$work/helper-depclean.json" | sed 's/"name": "//; s/", "version": "/-/; s/"$//' | sort)"
emerge_gone="$(sed -n 's/^All selected packages: //p' "$work/depclean.txt" | tr ' ' '\n' | sed 's/^=//' | grep . | sort)"
if grep -q '"ok": true' "$work/helper-depclean.json" && [ "$helper_gone" = "$emerge_gone" ]; then
    pass "bunny's Portage helper agrees with emerge's depclean ($(echo "$helper_gone" | grep -c .) packages)"
else
    fail "bunny's Portage helper agrees with emerge's depclean" "helper: $(tr '\n' ' ' < "$work/helper-depclean.json" | cut -c1-300) | $(tail -3 "$work/helper-depclean-err.txt" | tr '\n' ' ')"
    echo "      emerge: $(echo "$emerge_gone" | tr '\n' ' ')"
fi

# A missing package: bunny should explain it, not say it doesn't recognise it.
emerge --pretend --color=n app-misc/bunny-canary-no-such-package >"$work/missing.txt" 2>&1
if "$bunny" explain "$work/missing.txt" 2>&1 | grep -qi "no package"; then
    pass "bunny explains a misspelled package"
else
    fail "bunny explains a misspelled package" "$("$bunny" explain "$work/missing.txt" 2>&1 | head -3)"
fi

# A real refusal: mpv can't be installed on a bare stage3 (no display flags).
# Whatever emerge says, bunny should recognise it.
emerge --pretend --color=n media-video/mpv >"$work/mpv.txt" 2>&1
if [ "$(grep -c . "$work/mpv.txt")" -gt 0 ] && ! grep -Eq '^\[(ebuild|binary)' "$work/mpv.txt"; then
    if "$bunny" explain "$work/mpv.txt" 2>&1 | grep -q "doesn't recognise"; then
        fail "bunny explains why emerge refuses mpv" "emerge's message changed; see the mpv output below"
        sed 's/^/      | /' "$work/mpv.txt" | tail -40
    else
        pass "bunny explains why emerge refuses mpv"
    fi
else
    pass "emerge would install mpv here (nothing to explain this week)"
fi

section "Security advisories"
# bunny status reads `glsa-check --nocolor --list affected` (gentoolkit),
# lines like "202501-03 [N] Title ( cat/pkg )".
if emerge --quiet --getbinpkg app-portage/gentoolkit >"$work/gentoolkit.txt" 2>&1 \
    || emerge --quiet app-portage/gentoolkit >>"$work/gentoolkit.txt" 2>&1; then
    glsa-check --nocolor --list all >"$work/glsa.txt" 2>"$work/glsa-err.txt"
    echo "      glsa-check's first lines:"
    grep -E '^[0-9]{6}-[0-9]{2} ' "$work/glsa.txt" | head -3 | sed 's/^/      | /'
    if grep -Eq '^[0-9]{6}-[0-9]{2} \[[ANU ]\] .+' "$work/glsa.txt"; then
        pass "glsa-check's list still looks the way bunny reads it"
    else
        fail "glsa-check's list still looks the way bunny reads it" "$(head -5 "$work/glsa.txt") $(tail -2 "$work/glsa-err.txt")"
    fi
else
    echo "SKIP  couldn't install gentoolkit here: $(tail -1 "$work/gentoolkit.txt")"
fi

echo
if [ "$failed" -eq 0 ]; then
    echo "All checks passed."
else
    echo "Some checks failed: Gentoo changed something bunny relies on."
fi
exit "$failed"
