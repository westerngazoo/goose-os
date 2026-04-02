# GooseOS RISC-V Boot Code
#
# OpenSBI jumps here in S-mode with:
#   a0 = hart ID
#   a1 = device tree blob (DTB) pointer
#
# We park all harts except 0, zero BSS, set up the stack,
# then jump into Rust.

.section .text.boot
.global _start

_start:
    # --- Park non-boot harts ---
    # Only hart 0 continues; others spin in wfi loop.
    # VisionFive 2 has 4 U74 cores — this handles them all.
    bnez    a0, _park

    # --- Zero the .bss section ---
    # Rust expects zero-initialized statics to actually be zero.
    la      t0, _bss_start
    la      t1, _bss_end
_zero_bss:
    beq     t0, t1, _bss_done
    sd      zero, 0(t0)         # store doubleword (8 bytes)
    addi    t0, t0, 8
    j       _zero_bss
_bss_done:

    # --- Set up stack ---
    # Stack grows downward from top of RAM.
    la      sp, _stack_top

    # --- Jump to Rust ---
    # a0 = hart_id, a1 = dtb_addr (preserved from OpenSBI)
    call    kmain

    # kmain should never return, but if it does:
    j       _park

# --- Secondary hart parking loop ---
_park:
    wfi
    j       _park
