#!/bin/sh
# The Bunny Linux USB's welcome, shown when it starts. Type `welcome` to see
# it again.

online="no"
if ping -c 1 -W 2 distfiles.gentoo.org >/dev/null 2>&1; then
    online="yes"
fi

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

if [ "$online" = "yes" ]; then
    echo "  ✓ You're online."
else
    cat <<'EOF'
  ✗ Not online yet. bunny install downloads the system, so connect first:
      Ethernet:  plug in the cable; it connects by itself in a few seconds.
      Wi-Fi:     iwctl station wlan0 scan
                 iwctl station wlan0 get-networks
                 iwctl station wlan0 connect "Network name"
                 (iwctl device list shows the name if it isn't wlan0)
    Then check:  ping -c 1 gentoo.org
EOF
fi
if [ -f /etc/bunny-preview.toml ]; then
    cat <<'EOF'

  This is a preview USB, for testing: bunny records every command here.
  Nothing is sent from the USB. bunny install copies its recording into the
  new system; there, `bunny report` shows what it sends and asks first.
EOF
fi
echo
echo "  Type welcome to see this again."
echo
