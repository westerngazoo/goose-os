# GooseOS RISC-V Boot Code
#
# OpenSBI jumps here in S-mode with:
#   a0 = hart ID
#   a1 = device tree blob (DTB) pointer
#
# Platform-aware: the linker script defines _boot_hart_id as an
# absolute symbol (0 for QEMU, 1 for VF2). We store it in a nearby
# data word so we can load it PC-relative (avoids relocation overflow).

.section .text.boot
.global _start

_start:
    # --- Park non-boot harts ---
    # Load boot hart ID from embedded data (see bottom of file)
    la      t0, _boot_hart_data
    ld      t0, 0(t0)
    bne     a0, t0, _park

    # --- Zero the .bss section ---
    la      t0, _bss_start
    la      t1, _bss_end
_zero_bss:
    beq     t0, t1, _bss_done
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       _zero_bss
_bss_done:

    # --- Set up stack ---
    la      sp, _stack_top

    # --- Jump to Rust ---
    # a0 = hart_id, a1 = dtb_addr (preserved from OpenSBI)
    call    kmain

    # kmain should never return
    j       _park

# --- Secondary hart parking loop ---
_park:
    wfi
    j       _park

# --- Boot configuration data ---
# Stored here so `la` can reach it PC-relative.
# The .dword uses an absolute (R_RISCV_64) relocation — no range limit.
.align 3
_boot_hart_data:
    .dword _boot_hart_id
