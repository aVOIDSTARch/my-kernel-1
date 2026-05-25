KERNEL  := target/x86_64-crusty_os/debug/kernel
ISO     := my-kernel.iso
LIMINE  := ../limine  # adjust to wherever you cloned limine

.PHONY: all run clean iso

all: iso

$(KERNEL):
	cargo build

iso: $(KERNEL)
	mkdir -p iso_root/boot/limine
	cp $(KERNEL) iso_root/boot/kernel.elf
	cp $(LIMINE)/limine-bios.sys \
	   $(LIMINE)/limine-bios-cd.bin \
	   $(LIMINE)/limine-uefi-cd.bin \
	   iso_root/boot/limine/
	cat > iso_root/boot/limine/limine.conf << 'EOF'
	TIMEOUT=0
	VERBOSE=yes

	/My Kernel
	    PROTOCOL=limine
	    KERNEL_PATH=boot:///boot/kernel.elf
	EOF
	xorriso -as mkisofs \
		-b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image \
		--protective-msdos-label \
		iso_root -o $(ISO)
	$(LIMINE)/limine bios-install $(ISO)

run: iso
	qemu-system-x86_64 \
		-cdrom $(ISO) \
		-m 512M \
		-serial stdio \
		-no-reboot \
		-no-shutdown \
		-d int,cpu_reset \
		-D qemu.log

run-kvm: iso
	qemu-system-x86_64 \
		-cdrom $(ISO) \
		-m 512M \
		-enable-kvm \
		-cpu host \
		-serial stdio \
		-no-reboot \
		-no-shutdown

clean:
	cargo clean
	rm -rf iso_root $(ISO) qemu.log
