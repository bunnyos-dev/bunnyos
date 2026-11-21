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
#   ./bunny-vm.sh ssh [user]       SSH into the running VM (port 2222, default user "you")
#   ./bunny-vm.sh put <file>...    Copy files into the running VM (to /home/you)
#
# Options (environment variables):
#   MEM=8G CPUS=4 DISK_SIZE=60G GL=1   (GL=0 if 3D graphics won't start)
#   VM=laptop   a second, separate VM (its own disk, snapshots and SSH port
#               2223), for testing with the laptop's disk layout:
#               VM=laptop ./bunny-vm.sh run

set -eu

VM="${VM:-}"
if [ -n "$VM" ]; then
    case "$VM" in *[!a-z0-9-]*) echo "✗ VM names use a-z, 0-9 and -"; exit 1 ;; esac
    VM_DIR="${VM_DIR:-$HOME/bunny-vm-$VM}"
    NAME="bunny-$VM"
    PORT="${PORT:-2223}"
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

# Find a matching Secure Boot firmware pair (code + vars must come from the same set)
find_firmware() {
    for pair in \
        "/usr/share/edk2/OvmfX64/OVMF_CODE.secboot.4m.fd /usr/share/edk2/OvmfX64/OVMF_VARS.4m.fd" \
        "/usr/share/edk2/OvmfX64/OVMF_CODE.secboot.fd /usr/share/edk2/OvmfX64/OVMF_VARS.fd" \
        "/usr/share/edk2-ovmf/OVMF_CODE.secboot.fd /usr/share/edk2-ovmf/OVMF_VARS.fd" \
        "/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd /usr/share/edk2/ovmf/OVMF_VARS.fd" \
        "/usr/share/qemu/edk2-x86_64-secure-code.fd /usr/share/qemu/edk2-i386-vars.fd"
    do
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

stop() {
    if ! pgrep -f "qemu-system-x86_64 -name $NAME " >/dev/null; then
        echo "✓ The VM is already off."
        return 0
    fi
    echo "→ Shutting the VM down (it may ask for your password in the VM)"
    TERM=xterm-256color ssh -t -p "$PORT" -o StrictHostKeyChecking=no you@localhost \
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
                exec scp -P "$PORT" -o StrictHostKeyChecking=no "$@" you@localhost: ;;
    # TERM: the VM may not know your terminal (e.g. kitty), so use a common one
    ssh)        TERM=xterm-256color exec ssh -p "$PORT" -o StrictHostKeyChecking=no "${2:-you}"@localhost ;;
    *)          sed -n '2,18p' "$0" ;;
esac
