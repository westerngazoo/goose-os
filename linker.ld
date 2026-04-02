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
        KEEP(*(.text.boot))    /* boot.S entry point first */
        *(.text .text.*)
    } > RAM

    /* Read-only data (string literals, etc.) */
    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    } > RAM

    /* Initialized mutable data */
    .data : ALIGN(4K) {
        *(.data .data.*)
    } > RAM

    /* Zero-initialized data — boot.S will zero this */
    .bss : ALIGN(4K) {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _bss_end = .;
    } > RAM

    /* Stack lives at the top of RAM, grows downward */
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);

    /* Boot hart ID — boot.S compares a0 against this.
     * QEMU virt: hart 0 is the boot hart. */
    _boot_hart_id = 0;

    /* Discard unneeded sections */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
