#!/usr/bin/env bash
# Called by cargo test with the path to the compiled test kernel binary.
set -e

KERNEL_BIN="$1"
ISO="target/test.iso"
LIMINE="../../rust-os/limine"
ISO_ROOT="target/test-iso-root"

# Build a bootable ISO from the test binary.
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine"
cp "$KERNEL_BIN" "$ISO_ROOT/boot/kernel.elf"
cp "$LIMINE/limine-bios.sys" \
   "$LIMINE/limine-bios-cd.bin" \
   "$LIMINE/limine-uefi-cd.bin" \
   "$ISO_ROOT/boot/limine/"

cat > "$ISO_ROOT/boot/limine/limine.conf" << 'EOF'
timeout: 0
verbose: no

/Test Kernel
    protocol: limine
    path: boot():/boot/kernel.elf
    kaslr: no
EOF

xorriso -as mkisofs \
    -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --efi-boot boot/limine/limine-uefi-cd.bin \
    -efi-boot-part --efi-boot-image \
    --protective-msdos-label \
    "$ISO_ROOT" -o "$ISO" 2>/dev/null

"$LIMINE/limine" bios-install "$ISO" 2>/dev/null

# Run in QEMU.
# isa-debug-exit: writing 0x10 → QEMU exit code 33 (success)
#                 writing 0x11 → QEMU exit code 35 (failure)
qemu-system-x86_64 \
    -cdrom "$ISO" \
    -m 512M \
    -serial stdio \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -display none \
    -no-reboot \
    -no-shutdown || true
    -action shutdown=poweroff
QEMU_EXIT=$?

if [ $QEMU_EXIT -eq 33 ]; then
    exit 0
else
    echo "QEMU exited with code $QEMU_EXIT (expected 33 for success)" >&2
    exit 1
fi
