# GooseOS Trap Vector
#
# All traps (interrupts + exceptions) land here.
# We save every register to a TrapFrame on the kernel stack,
# call the Rust dispatcher, then restore and sret.
#
# Supports traps from BOTH S-mode and U-mode:
#
#   Convention:
#     sscratch = 0         when executing in S-mode
#     sscratch = kernel sp  when executing in U-mode
#
#   On entry from U-mode:
#     csrrw swaps sp(user) with sscratch(kernel_sp)
#     → sp = kernel_sp, sscratch = user_sp
#
#   On entry from S-mode:
#     csrrw swaps sp(kernel) with sscratch(0)
#     → sp = 0, sscratch = kernel_sp
#     We detect sp=0 and recover kernel_sp from sscratch.
#
# TrapFrame layout: 31 GPRs + sstatus + sepc = 33 fields * 8 = 264 bytes.
# Rounded to 272 (0x110) for 16-byte alignment.

.section .text
.balign 4
.global _trap_vector

_trap_vector:
    # --- Swap sp and sscratch ---
    csrrw   sp, sscratch, sp

    # If sp == 0, we came from S-mode (sscratch was 0)
    bnez    sp, _trap_save

    # From S-mode: kernel sp is now in sscratch, get it back
    csrr    sp, sscratch

_trap_save:
    # --- Allocate trap frame (272 bytes) ---
    addi    sp, sp, -272

    # --- Save all general-purpose registers ---
    sd      x1,   0x00(sp)     # ra

    # Save original sp: sscratch now holds whichever sp was swapped in.
    # From U-mode: sscratch = user_sp.  From S-mode: sscratch = kernel_sp.
    # Both are the "original sp" we want to preserve.
    csrr    t0, sscratch
    sd      t0,   0x08(sp)     # original sp (user or kernel)

    sd      x3,   0x10(sp)     # gp
    sd      x4,   0x18(sp)     # tp
    sd      x5,   0x20(sp)     # t0
    sd      x6,   0x28(sp)     # t1
    sd      x7,   0x30(sp)     # t2
    sd      x8,   0x38(sp)     # s0/fp
    sd      x9,   0x40(sp)     # s1
    sd      x10,  0x48(sp)     # a0
    sd      x11,  0x50(sp)     # a1
    sd      x12,  0x58(sp)     # a2
    sd      x13,  0x60(sp)     # a3
    sd      x14,  0x68(sp)     # a4
    sd      x15,  0x70(sp)     # a5
    sd      x16,  0x78(sp)     # a6
    sd      x17,  0x80(sp)     # a7
    sd      x18,  0x88(sp)     # s2
    sd      x19,  0x90(sp)     # s3
    sd      x20,  0x98(sp)     # s4
    sd      x21,  0xA0(sp)     # s5
    sd      x22,  0xA8(sp)     # s6
    sd      x23,  0xB0(sp)     # s7
    sd      x24,  0xB8(sp)     # s8
    sd      x25,  0xC0(sp)     # s9
    sd      x26,  0xC8(sp)     # s10
    sd      x27,  0xD0(sp)     # s11
    sd      x28,  0xD8(sp)     # t3
    sd      x29,  0xE0(sp)     # t4
    sd      x30,  0xE8(sp)     # t5
    sd      x31,  0xF0(sp)     # t6

    # Save CSRs
    csrr    t0, sstatus
    sd      t0,   0xF8(sp)     # sstatus
    csrr    t0, sepc
    sd      t0,   0x100(sp)    # sepc

    # Mark that we're in S-mode now: sscratch = 0
    csrw    sscratch, zero

    # --- Call Rust trap handler ---
    mv      a0, sp
    call    trap_handler

    # --- Restore CSRs ---
    ld      t0, 0x100(sp)
    csrw    sepc, t0
    ld      t0, 0xF8(sp)
    csrw    sstatus, t0

    # --- Check if returning to U-mode or S-mode ---
    # sstatus.SPP (bit 8): 1 = return to S-mode, 0 = return to U-mode
    andi    t0, t0, 0x100
    bnez    t0, _trap_restore_smode

_trap_restore_umode:
    # Returning to U-mode:
    #   Need sp = user_sp, sscratch = kernel_sp (for next trap)

    # Restore all GPRs except sp (x2) and t0 (x5) — need them as temps
    ld      x1,   0x00(sp)     # ra
    # skip x2 (sp)
    ld      x3,   0x10(sp)     # gp
    ld      x4,   0x18(sp)     # tp
    # skip x5 (t0) — restore below
    ld      x6,   0x28(sp)     # t1
    ld      x7,   0x30(sp)     # t2
    ld      x8,   0x38(sp)     # s0/fp
    ld      x9,   0x40(sp)     # s1
    ld      x10,  0x48(sp)     # a0
    ld      x11,  0x50(sp)     # a1
    ld      x12,  0x58(sp)     # a2
    ld      x13,  0x60(sp)     # a3
    ld      x14,  0x68(sp)     # a4
    ld      x15,  0x70(sp)     # a5
    ld      x16,  0x78(sp)     # a6
    ld      x17,  0x80(sp)     # a7
    ld      x18,  0x88(sp)     # s2
    ld      x19,  0x90(sp)     # s3
    ld      x20,  0x98(sp)     # s4
    ld      x21,  0xA0(sp)     # s5
    ld      x22,  0xA8(sp)     # s6
    ld      x23,  0xB0(sp)     # s7
    ld      x24,  0xB8(sp)     # s8
    ld      x25,  0xC0(sp)     # s9
    ld      x26,  0xC8(sp)     # s10
    ld      x27,  0xD0(sp)     # s11
    ld      x28,  0xD8(sp)     # t3
    ld      x29,  0xE0(sp)     # t4
    ld      x30,  0xE8(sp)     # t5
    ld      x31,  0xF0(sp)     # t6

    # Put user_sp into sscratch temporarily
    ld      t0,   0x08(sp)     # user sp from frame
    csrw    sscratch, t0       # sscratch = user_sp

    # Restore t0 (x5) from frame
    ld      x5,   0x20(sp)     # t0

    # Deallocate trap frame: sp = kernel stack top
    addi    sp, sp, 272

    # Swap: sp = user_sp (from sscratch), sscratch = kernel_sp
    csrrw   sp, sscratch, sp

    sret

_trap_restore_smode:
    # Returning to S-mode:
    #   sscratch is already 0 (set above after saving context)
    #   Just restore all GPRs and deallocate frame

    ld      x1,   0x00(sp)     # ra
    # skip x2 (sp) — recover from frame deallocation
    ld      x3,   0x10(sp)     # gp
    ld      x4,   0x18(sp)     # tp
    ld      x5,   0x20(sp)     # t0
    ld      x6,   0x28(sp)     # t1
    ld      x7,   0x30(sp)     # t2
    ld      x8,   0x38(sp)     # s0/fp
    ld      x9,   0x40(sp)     # s1
    ld      x10,  0x48(sp)     # a0
    ld      x11,  0x50(sp)     # a1
    ld      x12,  0x58(sp)     # a2
    ld      x13,  0x60(sp)     # a3
    ld      x14,  0x68(sp)     # a4
    ld      x15,  0x70(sp)     # a5
    ld      x16,  0x78(sp)     # a6
    ld      x17,  0x80(sp)     # a7
    ld      x18,  0x88(sp)     # s2
    ld      x19,  0x90(sp)     # s3
    ld      x20,  0x98(sp)     # s4
    ld      x21,  0xA0(sp)     # s5
    ld      x22,  0xA8(sp)     # s6
    ld      x23,  0xB0(sp)     # s7
    ld      x24,  0xB8(sp)     # s8
    ld      x25,  0xC0(sp)     # s9
    ld      x26,  0xC8(sp)     # s10
    ld      x27,  0xD0(sp)     # s11
    ld      x28,  0xD8(sp)     # t3
    ld      x29,  0xE0(sp)     # t4
    ld      x30,  0xE8(sp)     # t5
    ld      x31,  0xF0(sp)     # t6

    # Deallocate trap frame — sp back to original kernel sp
    addi    sp, sp, 272

    sret
