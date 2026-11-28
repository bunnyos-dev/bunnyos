#!/bin/sh
# bunny-vm.sh: Bunny Linux test VM (UEFI + Secure Boot + TPM 2.0 + KVM + 3D)
#
# Usage:
#   ./bunny-vm.sh run [file.iso]   start the VM (boots the ISO if given;
#                                  ISO=file.iso does the same)
#   ./bunny-vm.sh stop             shut the running VM down (asks for its password;
#                                  if it can't log in, asks you to power off
#                                  from the VM window)
#   ./bunny-vm.sh snap <name>      save a snapshot (VM must be off)
#   ./bunny-vm.sh restore <name>   go back to a snapshot (VM must be off)
#   ./bunny-vm.sh snaps            list snapshots
#   ./bunny-vm.sh reset-keys       wipe Secure Boot keys (back to Setup Mode)
#   ./bunny-vm.sh ssh [user]       SSH into the running VM (default: the VM's user)
#   ./bunny-vm.sh put <file>...    Copy files into the running VM (to the user's
#                                  home), one after another
#   ./bunny-vm.sh forget-key       forget the VM's SSH key (after reinstalling it)
#   ./bunny-vm.sh port             show the VM's SSH port
#
# Options (environment variables):
#   MEM=8G CPUS=4 DISK_SIZE=60G GL=1   (GL=0 if 3D graphics won't start)
#   VM=laptop   a second, separate VM (its own disk, snapshots and SSH
#               port), for testing with the laptop's disk layout:
#               VM=laptop ./bunny-vm.sh run
#   PORT=2230   use this SSH port instead (just this once)
#   VM_USER=test    the VM's user for ssh, put and stop. Remembered for this
#                   VM (in $VM_DIR/user) once given, so later commands
#                   don't need it. Without one: "you".
#   HOME_DISK=20G   also attach a second disk of this size (made once, kept
#                   in the VM's folder), for testing /home on another disk
#
# SSH ports: the main VM uses 2222. Each named VM gets its own port the
# first time it's used, the lowest from 2223 up that no other named VM has,
# kept in $VM_DIR/port, so several VMs can run at once. Each VM also keeps
# its SSH key in its own file ($VM_DIR/known_hosts), so SSH never mixes up
# two VMs ("REMOTE HOST IDENTIFICATION HAS CHANGED").

set -eu

VM="${VM:-}"
if [ -n "$VM" ]; then
    case "$VM" in *[!a-z0-9-]*) echo "✗ VM names use a-z, 0-9 and -"; exit 1 ;; esac
    VM_DIR="${VM_DIR:-$HOME/bunny-vm-$VM}"
    NAME="bunny-$VM"
else
    VM_DIR="${VM_DIR:-$HOME/bunny-vm}"
    NAME="bunnyos-test"
    PORT="${PORT:-2222}"
fi
DISK="$VM_DIR/disk.qcow2"
VARS="$VM_DIR/OVMF_VARS.fd"
TPM_DIR="$VM_DIR/tpm"
MEM="${MEM:-8G}"
CPUS="${CPUS:-4}"
DISK_SIZE="${DISK_SIZE:-60G}"
GL="${GL:-1}"
USER_FILE="$VM_DIR/user"
KNOWN_HOSTS="$VM_DIR/known_hosts"
HOME_DISK_FILE="$VM_DIR/home.qcow2"

# Prints the SSH port this named VM keeps in $VM_DIR/port. A VM without one
# gets the lowest port from 2223 up that no other VM's port file names (and
# that nothing is listening on, when ss is there to check). It's saved once
# the VM's folder exists, so it stays the same from then on.
named_vm_port() {
    if [ -f "$VM_DIR/port" ]; then
        read -r saved < "$VM_DIR/port" || true
        case "$saved" in
            ''|*[!0-9]*) ;;
            *) echo "$saved"; return ;;
        esac
    fi
    taken=" "
    for file in "$HOME"/bunny-vm-*/port; do
        [ -f "$file" ] && [ "$file" != "$VM_DIR/port" ] || continue
        read -r other < "$file" || true
        taken="$taken$other "
    done
    candidate=2223
    while :; do
        case "$taken" in
            *" $candidate "*) ;;
            *)
                if ! command -v ss >/dev/null 2>&1 \
                    || ! ss -Hltn "sport = :$candidate" 2>/dev/null | grep -q .; then
                    break
                fi
                ;;
        esac
        candidate=$((candidate + 1))
    done
    if [ -d "$VM_DIR" ]; then
        echo "$candidate" > "$VM_DIR/port"
    fi
    echo "$candidate"
}

# The VM's user: VM_USER if given (and remembered in $VM_DIR/user for next
# time), else the remembered one, else "you".
if [ -n "${VM_USER:-}" ]; then
    case "$VM_USER" in
        *[!a-z0-9_-]*|-*) echo "✗ VM_USER should be a plain user name, like test"; exit 1 ;;
    esac
    saved_user=""
    if [ -f "$USER_FILE" ]; then
        read -r saved_user < "$USER_FILE" || true
    fi
    if [ "$saved_user" != "$VM_USER" ]; then
        mkdir -p "$VM_DIR"
        echo "$VM_USER" > "$USER_FILE"
        echo "→ Remembering $VM_USER as the ${VM:-main} VM's user (in $USER_FILE)"
    fi
else
    VM_USER=you
    if [ -f "$USER_FILE" ]; then
        read -r saved_user < "$USER_FILE" || true
        case "${saved_user:-}" in
            ''|*[!a-z0-9_-]*|-*) ;;
            *) VM_USER="$saved_user" ;;
        esac
    fi
fi

PORT_GIVEN="${PORT:+yes}"
if [ -n "$VM" ]; then
    PORT="${PORT:-$(named_vm_port)}"
else
    PORT="${PORT:-2222}"
fi

# Find a matching Secure Boot firmware pair (code + vars must come from the same set)
find_firmware() {
    for pair in \
        "/usr/share/edk2/OvmfX64/OVMF_CODE.secboot.4m.fd /usr/share/edk2/OvmfX64/OVMF_VARS.4m.fd" \
        "/usr/share/edk2/OvmfX64/OVMF_CODE.secboot.fd /usr/share/edk2/OvmfX64/OVMF_VARS.fd" \
        "/usr/share/edk2-ovmf/OVMF_CODE.secboot.fd /usr/share/edk2-ovmf/OVMF_VARS.fd" \
        "/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd /usr/share/edk2/ovmf/OVMF_VARS.fd" \
        "/usr/share/qemu/edk2-x86_64-secure-code.fd /usr/share/qemu/edk2-i386-vars.fd"
    do
        # shellcheck disable=SC2086  # two paths, split on purpose
        set -- $pair
        if [ -f "$1" ] && [ -f "$2" ]; then
            CODE="$1"; VARS_TEMPLATE="$2"; return 0
        fi
    done
    echo "✗ Couldn't find Secure Boot UEFI firmware."
    echo "  Run this and send me the output:"
    echo "  ls -R /usr/share/edk2* /usr/share/qemu 2>/dev/null | grep -i '\.fd'"
    exit 1
}

setup() {
    find_firmware
    mkdir -p "$VM_DIR" "$TPM_DIR"
    # A new named VM keeps the port it was just given (not a PORT=… one).
    if [ -n "$VM" ] && [ -z "$PORT_GIVEN" ] && [ ! -f "$VM_DIR/port" ]; then
        echo "$PORT" > "$VM_DIR/port"
    fi
    if [ ! -f "$DISK" ]; then
        echo "→ Creating $DISK_SIZE virtual disk"
        qemu-img create -f qcow2 "$DISK" "$DISK_SIZE" >/dev/null
    fi
    if [ -n "${HOME_DISK:-}" ] && [ ! -f "$HOME_DISK_FILE" ]; then
        echo "→ Creating a second $HOME_DISK virtual disk (for /home)"
        qemu-img create -f qcow2 "$HOME_DISK_FILE" "$HOME_DISK" >/dev/null
    fi
    if [ ! -f "$VARS" ]; then
        echo "→ Creating fresh UEFI settings (Secure Boot in Setup Mode)"
        cp "$VARS_TEMPLATE" "$VARS"
    fi
}

check_off() {
    if pgrep -f "qemu-system-x86_64 -name $NAME " >/dev/null; then
        echo "✗ The VM is running. Shut it down first."
        exit 1
    fi
}

run() {
    setup
    [ -e /dev/kvm ] || { echo "✗ /dev/kvm missing: KVM isn't enabled in your kernel."; exit 1; }

    # Software TPM 2.0 (exits automatically when the VM shuts down)
    rm -f "$TPM_DIR/sock"
    swtpm socket --tpm2 --tpmstate dir="$TPM_DIR" \
        --ctrl type=unixio,path="$TPM_DIR/sock" --terminate &
    sleep 1

    if [ "$GL" = "1" ]; then
        GFX="-device virtio-vga-gl -display gtk,gl=on"
    else
        GFX="-device virtio-vga -display gtk"
    fi

    CDROM=""
    if [ -n "${1:-}" ]; then
        [ -f "$1" ] || { echo "✗ ISO not found: $1"; exit 1; }
        CDROM="-cdrom $1"
    fi
    SECOND=""
    if [ -n "${HOME_DISK:-}" ]; then
        SECOND="-drive file=$HOME_DISK_FILE,if=virtio,format=qcow2"
    fi

    echo "→ Starting the ${VM:-main} Bunny Linux test VM ($CPUS CPUs, $MEM RAM, SSH on port $PORT)"
    # shellcheck disable=SC2086
    exec qemu-system-x86_64 -name "$NAME" \
        -enable-kvm -cpu host -smp "$CPUS" -m "$MEM" \
        -machine q35,smm=on \
        -global driver=cfi.pflash01,property=secure,value=on \
        -drive if=pflash,format=raw,unit=0,file="$CODE",readonly=on \
        -drive if=pflash,format=raw,unit=1,file="$VARS" \
        -drive file="$DISK",if=virtio,format=qcow2 \
        $SECOND \
        -chardev socket,id=chrtpm,path="$TPM_DIR/sock" \
        -tpmdev emulator,id=tpm0,chardev=chrtpm \
        -device tpm-tis,tpmdev=tpm0 \
        $GFX \
        -nic user,model=virtio-net-pci,hostfwd=tcp::$PORT-:22 \
        -device virtio-rng-pci \
        -usb -device usb-tablet \
        $CDROM
}

# ssh and scp with this VM's own known_hosts file. A new key is saved the
# first time; a different one later is refused (see forget-key). The quotes
# inside the option keep a folder name with spaces in one piece.
vm_ssh() {
    mkdir -p "$VM_DIR"
    ssh -p "$PORT" -o "UserKnownHostsFile=\"$KNOWN_HOSTS\"" \
        -o StrictHostKeyChecking=accept-new "$@"
}
vm_scp() {
    mkdir -p "$VM_DIR"
    scp -P "$PORT" -o "UserKnownHostsFile=\"$KNOWN_HOSTS\"" \
        -o StrictHostKeyChecking=accept-new "$@"
}

# Runs ssh or scp; if it fails because of the key, says what to do.
with_key_hint() {
    status=0
    "$@" || status=$?
    if [ "$status" = 255 ]; then
        echo "✗ Couldn't connect to the ${VM:-main} VM."
        echo "  If SSH said the host key changed, the VM was reinstalled (or is"
        echo "  running a different system). Forget the old key and try again:"
        echo "  ${VM:+VM=$VM }$0 forget-key"
    fi
    return "$status"
}

forget_key() {
    if [ -f "$KNOWN_HOSTS" ]; then
        ssh-keygen -R "[localhost]:$PORT" -f "$KNOWN_HOSTS" >/dev/null 2>&1 || true
        rm -f "$KNOWN_HOSTS.old"
    fi
    echo "✓ Forgot the ${VM:-main} VM's SSH key. The next ssh saves the new one."
}

# Whether this VM is running.
vm_running() {
    pgrep -f "qemu-system-x86_64 -name $NAME " >/dev/null
}

# Waits up to $1 seconds for the VM to be off. Returns 1 if it's still on.
wait_off() {
    i=0
    while vm_running; do
        i=$((i + 1))
        [ "$i" -le "$1" ] || return 1
        sleep 1
    done
}

stop() {
    if ! vm_running; then
        echo "✓ The VM is already off."
        return 0
    fi
    echo "→ Shutting the VM down as $VM_USER (it may ask for $VM_USER's password in the VM)"
    TERM=xterm-256color vm_ssh -o ConnectTimeout=5 -t "$VM_USER@localhost" \
        'if command -v doas >/dev/null; then doas poweroff; else sudo poweroff; fi' || true
    # ssh's exit status can't tell "logged in and powered off" (the
    # connection drops) from "couldn't log in", so look at the VM instead.
    if ! wait_off 15; then
        echo "  Couldn't shut it down over SSH (is sshd running in the VM, and is"
        echo "  $VM_USER its user? VM_USER=<name> sets it). In the VM window, run:"
        echo "  sudo poweroff     (or doas poweroff)"
        echo "→ Waiting for the VM to turn off…"
        if ! wait_off 180; then
            echo "✗ The VM is still running. Close its window, or shut it down from inside."
            exit 1
        fi
    fi
    echo "✓ The VM is off."
}

# Copies each file into the VM's home folder, one scp at a time, saying
# which went over. Stops at the first that fails.
put() {
    [ $# -gt 0 ] || { echo "✗ Name at least one file to copy."; exit 1; }
    for file in "$@"; do
        [ -e "$file" ] || { echo "✗ $file doesn't exist, so nothing was copied."; exit 1; }
    done
    for file in "$@"; do
        with_key_hint vm_scp "$file" "$VM_USER@localhost:" || exit $?
        echo "✓ Copied $file to $VM_USER's home folder in the ${VM:-main} VM."
    done
}

case "${1:-run}" in
    run)        shift 2>/dev/null || true; run "${1:-${ISO:-}}" ;;
    stop)       stop ;;
    snap)       check_off; qemu-img snapshot -c "${2:?name needed}" "$DISK"; echo "✓ Saved snapshot: $2" ;;
    restore)    check_off; qemu-img snapshot -a "${2:?name needed}" "$DISK"; echo "✓ Restored: $2" ;;
    snaps)      qemu-img snapshot -l "$DISK" ;;
    reset-keys) check_off; find_firmware; cp "$VARS_TEMPLATE" "$VARS"; echo "✓ Secure Boot keys wiped (Setup Mode)" ;;
    put)        shift; put "$@" ;;
    # TERM: the VM may not know your terminal (e.g. kitty), so use a common one
    ssh)        TERM=xterm-256color with_key_hint vm_ssh "${2:-$VM_USER}@localhost" ;;
    forget-key) forget_key ;;
    port)       echo "$PORT" ;;
    *)          sed -n '2,36p' "$0" ;;
esac
