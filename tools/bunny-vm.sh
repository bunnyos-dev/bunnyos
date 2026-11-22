#!/bin/sh
# bunny-vm.sh: Bunny Linux test VM (UEFI + Secure Boot + TPM 2.0 + KVM + 3D)
#
# Usage:
#   ./bunny-vm.sh run [file.iso]   start the VM (boots the ISO if given;
#                                  ISO=file.iso does the same)
#   ./bunny-vm.sh stop             shut the running VM down (asks for its password)
#   ./bunny-vm.sh snap <name>      save a snapshot (VM must be off)
#   ./bunny-vm.sh restore <name>   go back to a snapshot (VM must be off)
#   ./bunny-vm.sh snaps            list snapshots
#   ./bunny-vm.sh reset-keys       wipe Secure Boot keys (back to Setup Mode)
#   ./bunny-vm.sh ssh [user]       SSH into the running VM (default user "you")
#   ./bunny-vm.sh put <file>...    Copy files into the running VM (to the user's home)
#   ./bunny-vm.sh forget-key       forget the VM's SSH key (after reinstalling it)
#   ./bunny-vm.sh port             show the VM's SSH port
#
# Options (environment variables):
#   MEM=8G CPUS=4 DISK_SIZE=60G GL=1   (GL=0 if 3D graphics won't start)
#   VM=laptop   a second, separate VM (its own disk, snapshots and SSH
#               port), for testing with the laptop's disk layout:
#               VM=laptop ./bunny-vm.sh run
#   PORT=2230   use this SSH port instead (just this once)
#   VM_USER=bunny   the VM's user for ssh, put and stop (default "you")
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
VM_USER="${VM_USER:-you}"
KNOWN_HOSTS="$VM_DIR/known_hosts"

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

    echo "→ Starting the ${VM:-main} Bunny Linux test VM ($CPUS CPUs, $MEM RAM, SSH on port $PORT)"
    # shellcheck disable=SC2086
    exec qemu-system-x86_64 -name "$NAME" \
        -enable-kvm -cpu host -smp "$CPUS" -m "$MEM" \
        -machine q35,smm=on \
        -global driver=cfi.pflash01,property=secure,value=on \
        -drive if=pflash,format=raw,unit=0,file="$CODE",readonly=on \
        -drive if=pflash,format=raw,unit=1,file="$VARS" \
        -drive file="$DISK",if=virtio,format=qcow2 \
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

stop() {
    if ! pgrep -f "qemu-system-x86_64 -name $NAME " >/dev/null; then
        echo "✓ The VM is already off."
        return 0
    fi
    echo "→ Shutting the VM down (it may ask for your password in the VM)"
    TERM=xterm-256color vm_ssh -t "$VM_USER@localhost" \
        'if command -v doas >/dev/null; then doas poweroff; else sudo poweroff; fi' || true
    i=0
    while pgrep -f "qemu-system-x86_64 -name $NAME " >/dev/null; do
        i=$((i + 1))
        if [ "$i" -gt 60 ]; then
            echo "✗ The VM is still running after a minute. Close its window, or shut it down from inside."
            exit 1
        fi
        sleep 1
    done
    echo "✓ The VM is off."
}

case "${1:-run}" in
    run)        shift 2>/dev/null || true; run "${1:-${ISO:-}}" ;;
    stop)       stop ;;
    snap)       check_off; qemu-img snapshot -c "${2:?name needed}" "$DISK"; echo "✓ Saved snapshot: $2" ;;
    restore)    check_off; qemu-img snapshot -a "${2:?name needed}" "$DISK"; echo "✓ Restored: $2" ;;
    snaps)      qemu-img snapshot -l "$DISK" ;;
    reset-keys) check_off; find_firmware; cp "$VARS_TEMPLATE" "$VARS"; echo "✓ Secure Boot keys wiped (Setup Mode)" ;;
    put)        shift; [ $# -gt 0 ] || { echo "✗ Name at least one file to copy."; exit 1; }
                with_key_hint vm_scp "$@" "$VM_USER@localhost:" ;;
    # TERM: the VM may not know your terminal (e.g. kitty), so use a common one
    ssh)        TERM=xterm-256color with_key_hint vm_ssh "${2:-$VM_USER}@localhost" ;;
    forget-key) forget_key ;;
    port)       echo "$PORT" ;;
    *)          sed -n '2,30p' "$0" ;;
esac
