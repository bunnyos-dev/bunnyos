#!/bin/sh
# Nightly health check for the home test machine (a real Bunny Linux install).
#
# Read-only: it looks at the system and changes nothing, so it runs as the
# unprivileged `runner` user. Each check prints one plain-English line.
# "problem" lines make the job fail; "note" lines are only information.
#
#   tools/test-box/nightly-check.sh
#
# Exit status: 0 when nothing is wrong, 1 when at least one problem was found.

problems=0

ok()      { printf '✓ %s\n' "$1"; }
note()    { printf '· %s\n' "$1"; }
problem() {
	printf '✗ %s\n' "$1"
	[ -n "$2" ] && printf '    %s\n' "$2"
	problems=$((problems + 1))
}

echo "Bunny Linux nightly check on $(hostname)"
echo

# Which system is this, and how long has it been up.
note "$(grep '^PRETTY_NAME=' /etc/os-release | cut -d= -f2 | tr -d '"'), kernel $(uname -r)"
note "running since $(uptime -s 2>/dev/null || echo 'unknown')"

# Services: anything OpenRC started that has since died.
crashed=$(rc-status --crashed 2>/dev/null | awk '{print $1}' | tr '\n' ' ')
if ! command -v rc-status >/dev/null 2>&1; then
	problem "rc-status isn't here, so this doesn't look like an OpenRC system"
elif [ -n "$crashed" ]; then
	problem "these services crashed: $crashed" \
		"Look at the machine with: rc-status, and the service's log in /var/log"
else
	ok "no crashed services"
fi

# Network: the runner can only pick up jobs if this works, so check DNS too.
if getent hosts github.com >/dev/null 2>&1; then
	ok "network and DNS work"
else
	problem "can't look up github.com" "Check Wi-Fi with: iwctl station list"
fi

# Disk space on / (btrfs snapshots and builds fill it up over time).
used=$(df --output=pcent / | tail -n 1 | tr -dc '0-9')
if [ -z "$used" ]; then
	problem "couldn't read how full the disk is"
elif [ "$used" -ge 90 ]; then
	problem "the disk is ${used}% full" "Free space with: sudo bunny clean"
else
	ok "the disk is ${used}% full"
fi

# Undo: snapshots must exist for rollbacks to work.
if [ -d /.snapshots ]; then
	ok "the snapshot folder for undo is there"
else
	problem "there's no /.snapshots, so changes can't be undone"
fi

# Memory and swap, just for the record.
note "memory: $(free -h | awk '/^Mem:/ {print $3 " used of " $2}'), swap: $(free -h | awk '/^Swap:/ {print $2}')"

echo
if [ "$problems" -eq 0 ]; then
	echo "Nothing wrong found."
	exit 0
fi
echo "$problems problem(s) found; each line marked ✗ says what to look at."
exit 1
