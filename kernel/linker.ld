OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    /*
     * OpenSBI occupies 0x80000000 - 0x801FFFFF
     * Our kernel loads at 0x80200000
     * QEMU virt machine has 128MB RAM by default
     */
    RAM (rwx) : ORIGIN = 0x80200000, LENGTH = 126M
}

SECTIONS
{
    /* Code — _start MUST be first at 0x80200000 */
    .text : {
        _text_start = .;
        KEEP(*(.text.boot))    /* boot.S entry point first */
        *(.text .text.*)
        _text_end = .;
    } > RAM

    /* Read-only data (string literals, etc.) */
    .rodata : ALIGN(4K) {
        _rodata_start = .;
        *(.rodata .rodata.*)
        _rodata_end = .;
    } > RAM

    /* Initialized mutable data */
    .data : ALIGN(4K) {
        _data_start = .;
        *(.data .data.*)
        _data_end = .;
    } > RAM

    /* Zero-initialized data — boot.S will zero this */
    .bss : ALIGN(4K) {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _bss_end = .;
    } > RAM

    /* End of kernel image — free memory starts here (page-aligned) */
    . = ALIGN(4K);
    _end = .;

    /* Kernel stack: 64KB at top of RAM, grows downward.
     * Page allocator must NOT allocate above _heap_end. */
    _stack_size = 64K;
    _memory_end = ORIGIN(RAM) + LENGTH(RAM);
    _stack_top = _memory_end;
    _heap_end = _memory_end - _stack_size;

    /* Boot hart ID — boot.S compares a0 against this.
     * QEMU virt: hart 0 is the boot hart. */
    _boot_hart_id = 0;

    /* Discard unneeded sections */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
