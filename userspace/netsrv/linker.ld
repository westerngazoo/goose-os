/* GooseOS userspace linker script — netsrv
 *
 * Same layout as userspace/hello/linker.ld. The kernel loads user
 * binaries at USER_VA_START (0x50000000) regardless of which program
 * it is; the scheduler + page table isolate them.
 *
 * See docs/unsafe-audit.md INV-3 for why these addresses are fixed.
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
