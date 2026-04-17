/* GooseOS userspace linker script
 *
 * Places code at USER_VA_START (0x50000000).
 * Kernel provides stack at 0x7FFF_0000 — do NOT allocate stack here.
 * All sections must be within [0x50000000, 0x80000000).
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    USER_RAM (rwx) : ORIGIN = 0x50000000, LENGTH = 0x2FFF0000
}

SECTIONS
{
    /* Code — _start MUST be first at 0x50000000 */
    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    } > USER_RAM

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } > USER_RAM

    .data : ALIGN(4K) {
        __global_pointer$ = . + 0x800;
        *(.data .data.*)
        *(.sdata .sdata.*)
    } > USER_RAM

    .bss : ALIGN(4K) {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _bss_end = .;
    } > USER_RAM

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.init_array)
        *(.fini_array)
    }
}
