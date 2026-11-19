#!/bin/sh
# The Bunny Linux USB's welcome, shown when it starts. Type `welcome` to see
# it again (and to check the connection again).
#
# The network can take a few seconds after login (DHCP, Wi-Fi), so the
# connection check tries for up to WELCOME_WAIT seconds (15 by default)
# before saying anything. It checks two things separately:
#   - an address: a reply from a well-known address (no names involved)
#   - names (DNS): looking up the Gentoo server bunny install downloads from
# so "online, but names don't work" gets its own message.

wait_for="${WELCOME_WAIT:-15}"
name="distfiles.gentoo.org"
addresses="1.1.1.1 9.9.9.9 8.8.8.8"

# A reply from any of the addresses (one try each, 1 second).
address_works() {
    for a in $addresses; do
        ping -c 1 -W 1 "$a" >/dev/null 2>&1 && return 0
    done
    return 1
}

# The name looks up (capped at 2 seconds, so a DNS server that never answers
# doesn't stall the welcome).
names_work() {
    timeout 2 getent hosts "$name" >/dev/null 2>&1
}

# A default route means a network is connected, even if it doesn't reach the
# internet (IPv4 or IPv6).
has_route() {
    awk 'NR > 1 && $2 == "00000000" { found = 1 } END { exit !found }' /proc/net/route 2>/dev/null \
        || awk '$1 == "00000000000000000000000000000000" && $2 == "00" && $10 != "lo" { found = 1 } END { exit !found }' /proc/net/ipv6_route 2>/dev/null
}

cat <<'EOF'

  Bunny Linux  ·  live USB

  This is a small Bunny Linux system running from the USB stick. Nothing on
  this computer has been changed.

  To install Bunny Linux:     bunny install
  To see what it would do:    bunny install --dry-run
  All the options:            bunny install --help

  The Guide (the install chapter walks through every step):
    https://bunnyos-dev.github.io/bunnyos/

EOF

# On a terminal, show that it's checking, and clear the line afterwards.
checking=""
if [ -t 1 ]; then
    checking="yes"
    printf '  Checking the connection…'
fi
start="$(date +%s)"
address="no"
names="no"
while :; do
    address="no"
    names="no"
    if names_work; then
        names="yes"
        break
    fi
    address_works && address="yes"
    [ $(($(date +%s) - start)) -ge "$wait_for" ] && break
    sleep 1
done
[ -n "$checking" ] && printf '\r\033[K'

if [ "$names" = "yes" ]; then
    # Names work, so downloads will (even where pings are blocked).
    echo "  ✓ You're online."
    again="see this again"
elif [ "$address" = "yes" ]; then
    cat <<EOF
  ⚠ Online, but names don't work. The internet answers, but looking up
    $name doesn't (DNS), so downloads would fail.
      Often it's only a few seconds late: wait a moment and check again.
      If it stays this way, use a public name server:
        echo 'nameserver 9.9.9.9' >> /etc/resolv.conf
EOF
    again="check again"
else
    if has_route; then
        echo "  ✗ Connected to a network, but it doesn't reach the internet. If it needs"
        echo "    a sign-in page (hotels, trains), use another network:"
    else
        echo "  ✗ Not online yet. bunny install downloads the system, so connect first:"
    fi
    cat <<'EOF'
      Ethernet:  plug in the cable; it connects by itself in a few seconds.
      Wi-Fi:     iwctl station wlan0 scan
                 iwctl station wlan0 get-networks
                 iwctl station wlan0 connect "Network name"
                 (iwctl device list shows the name if it isn't wlan0)
EOF
    again="check again"
fi
echo
echo "  Type welcome to $again."
echo
