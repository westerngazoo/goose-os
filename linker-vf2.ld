/* Linker script for StarFive VisionFive 2 (JH7110)
 *
 * Key differences from QEMU virt:
 *   - RAM starts at 0x40000000 (not 0x80000000)
 *   - Kernel entry at 0x40200000 (OpenSBI convention: DRAM_BASE + 2M)
 *   - OpenSBI lives at 0x40000000 - 0x401FFFFF (do NOT overwrite)
 */

OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    RAM (rwx) : ORIGIN = 0x40200000, LENGTH = 126M
}

SECTIONS
{
    /* Code — _start MUST be first at 0x40200000 */
    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    } > RAM

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    } > RAM

    .data : ALIGN(4K) {
        *(.data .data.*)
    } > RAM

    .bss : ALIGN(4K) {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _bss_end = .;
    } > RAM

    _stack_top = ORIGIN(RAM) + LENGTH(RAM);

    /* Boot hart ID — boot.S compares a0 against this.
     * VisionFive 2: hart 0 is the S7 monitor core (no MMU!).
     * Hart 1 is the first U74 application core = boot hart. */
    _boot_hart_id = 1;

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
