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
    # ┌─────────────────────────────────────────────────────────┐
    # │ THE SP SWAP — the most important instruction in the OS  │
    # │                                                         │
    # │ csrrw does: tmp = sscratch; sscratch = sp; sp = tmp     │
    # │ It's an atomic register swap between sp and sscratch.   │
    # │                                                         │
    # │ From U-mode (sscratch held kernel_sp):                  │
    # │   BEFORE: sp = user_sp,   sscratch = kernel_sp          │
    # │   AFTER:  sp = kernel_sp, sscratch = user_sp            │
    # │   → We now have a valid kernel stack. User sp is safe   │
    # │     in sscratch, we'll save it to the TrapFrame later.  │
    # │                                                         │
    # │ From S-mode (sscratch was 0, our convention):            │
    # │   BEFORE: sp = kernel_sp, sscratch = 0                  │
    # │   AFTER:  sp = 0,         sscratch = kernel_sp          │
    # │   → sp is 0 (garbage). We detect this below and recover │
    # │     kernel_sp from sscratch.                             │
    # └─────────────────────────────────────────────────────────┘
    csrrw   sp, sscratch, sp

    # sp != 0 → came from U-mode, sp already holds kernel_sp, jump to save
    # sp == 0 → came from S-mode, need to recover kernel_sp from sscratch
    bnez    sp, _trap_save

    # S-mode recovery: sscratch has our kernel_sp (we swapped it there above)
    csrr    sp, sscratch
    # Now: sp = kernel_sp, sscratch = kernel_sp (both same — that's fine)

_trap_save:
    # --- Allocate trap frame (272 bytes) ---
    addi    sp, sp, -272

    # --- Save all general-purpose registers ---
    sd      x1,   0x00(sp)     # ra

    # Save the original sp that was in use BEFORE the trap.
    # After the csrrw swap at entry:
    #   From U-mode: sscratch = user_sp    (the user's stack pointer)
    #   From S-mode: sscratch = kernel_sp  (same as current sp)
    # Either way, sscratch holds the "original sp" we need to preserve.
    csrr    t0, sscratch
    sd      t0,   0x08(sp)     # frame.sp = original sp (user or kernel)

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

    # Mark that we're now executing in S-mode: sscratch = 0.
    # This is our convention — if a NESTED trap occurs while we're
    # in the kernel, the entry code sees sscratch=0 → sp=0 → S-mode path.
    # Without this, a nested trap would think sscratch holds a kernel_sp
    # and corrupt the stack.
    csrw    sscratch, zero

    # --- Call Rust trap handler ---
    # a0 = pointer to TrapFrame on the kernel stack.
    # The handler may OVERWRITE this frame (context switch) — when it
    # returns, the frame might contain a DIFFERENT process's registers.
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
    # ┌─────────────────────────────────────────────────────────┐
    # │ RETURNING TO U-MODE — reverse the sp swap               │
    # │                                                         │
    # │ Goal: sp = user_sp, sscratch = kernel_sp                │
    # │ (So the NEXT trap from U-mode can swap them again.)     │
    # │                                                         │
    # │ Strategy:                                                │
    # │   1. Restore all GPRs except sp and t0 (need as temps)  │
    # │   2. Load user_sp from frame into sscratch              │
    # │   3. Restore t0 from frame                              │
    # │   4. Deallocate frame (sp = kernel stack top)           │
    # │   5. csrrw sp, sscratch, sp                             │
    # │      → sp = user_sp, sscratch = kernel_sp              │
    # │   6. sret → jump to sepc in U-mode                     │
    # └─────────────────────────────────────────────────────────┘

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

    # --- The reverse sp swap (3 steps) ---

    # Step 1: Stash user_sp into sscratch (temporary holding)
    # State: sp = kernel_sp (frame base), sscratch = 0
    ld      t0,   0x08(sp)     # t0 = user_sp (saved in frame at entry)
    csrw    sscratch, t0       # sscratch = user_sp

    # Step 2: Restore the last temp register and free the frame
    ld      x5,   0x20(sp)     # restore t0 (x5) — all GPRs now restored
    addi    sp, sp, 272        # deallocate frame → sp = kernel stack top

    # Step 3: THE REVERSE SWAP
    #   BEFORE: sp = kernel_sp (stack top), sscratch = user_sp
    #   AFTER:  sp = user_sp,              sscratch = kernel_sp
    #   → User gets their stack back. Kernel sp is saved in sscratch
    #     for the next trap entry.
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
