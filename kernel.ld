OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(x86_64)
ENTRY(kernel_main)

KERNEL_VIRT_BASE = 0xFFFFFFFF80000000;

PHDRS {
    requests PT_LOAD FLAGS(4);
    text     PT_LOAD FLAGS(5);
    rodata   PT_LOAD FLAGS(4);
    data     PT_LOAD FLAGS(6);
}

SECTIONS {
    . = KERNEL_VIRT_BASE + SIZEOF_HEADERS;

    .limine_requests : ALIGN(8) {
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :requests

    .text : ALIGN(4096) {
        *(.text .text.*)
    } :text

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    } :rodata

    .data : ALIGN(4096) {
        *(.data .data.*)
    } :data

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        __bss_end = .;
    } :data

    __kernel_start = KERNEL_VIRT_BASE + SIZEOF_HEADERS;
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
