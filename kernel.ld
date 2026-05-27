OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(x86_64)
ENTRY(kernel_main)

PHDRS {
    requests PT_LOAD FLAGS(4);
    text     PT_LOAD FLAGS(5);
    rodata   PT_LOAD FLAGS(4);
    data     PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0xFFFFFFFF80000000;
    . += SIZEOF_HEADERS;
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .limine_requests : {
        __kernel_start = .;
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :requests

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .text : {
        *(.text .text.*)
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .data : {
        *(.data .data.*)
    } :data

    .bss : {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        __bss_end = .;
    } :data

    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
