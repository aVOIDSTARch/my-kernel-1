#!/usr/bin/env bash
# Runner for `cargo run` — builds a bootable ISO and launches QEMU interactively.
set -e

KERNEL_BIN="$1"
ISO="target/run.iso"
LIMINE="../../rust-os/limine"
ISO_ROOT="target/run-iso-root"

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
graphics: no

/My Kernel
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

qemu-system-x86_64 \
    -cdrom "$ISO" \
    -m 512M \
    -serial stdio \
    -no-reboot \
    -no-shutdown \
    -d int,cpu_reset \
    -D qemu.log
