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
        _text_start = .;
        KEEP(*(.text.boot))
        *(.text .text.*)
        _text_end = .;
    } > RAM

    .rodata : ALIGN(4K) {
        _rodata_start = .;
        *(.rodata .rodata.*)
        _rodata_end = .;
    } > RAM

    .data : ALIGN(4K) {
        _data_start = .;
        *(.data .data.*)
        _data_end = .;
    } > RAM

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
     * VisionFive 2: hart 0 is the S7 monitor core (no MMU!).
     * Hart 1 is the first U74 application core = boot hart. */
    _boot_hart_id = 1;

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
