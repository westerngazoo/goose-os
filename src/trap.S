# GooseOS Trap Vector
#
# All traps (interrupts + exceptions) land here.
# We save every register to a TrapFrame on the stack,
# call the Rust dispatcher, then restore and sret.
#
# Entered from S-mode with interrupts disabled
# (hardware clears sstatus.SIE on trap entry).

.section .text
.balign 4
.global _trap_vector

_trap_vector:
    # --- Allocate trap frame (272 bytes, 16-byte aligned) ---
    # 31 GPRs + sstatus + sepc = 33 fields * 8 bytes = 264
    # Round to 272 (0x110) for 16-byte alignment
    addi    sp, sp, -272

    # --- Save all general-purpose registers ---
    # x0 is hardwired zero, no need to save
    sd      x1,   0x00(sp)     # ra
    # x2 (sp) saved below — need the original value
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

    # Save original sp (before we allocated the frame)
    addi    t0, sp, 272
    sd      t0,   0x08(sp)     # original sp

    # Save CSRs
    csrr    t0, sstatus
    sd      t0,   0xF8(sp)     # sstatus
    csrr    t0, sepc
    sd      t0,   0x100(sp)    # sepc

    # --- Call Rust trap handler ---
    # a0 = pointer to TrapFrame (sp)
    mv      a0, sp
    call    trap_handler

    # --- Restore CSRs ---
    ld      t0, 0x100(sp)
    csrw    sepc, t0
    ld      t0, 0xF8(sp)
    csrw    sstatus, t0

    # --- Restore all general-purpose registers ---
    ld      x1,   0x00(sp)     # ra
    # skip x2 (sp) — restore last
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

    # Deallocate trap frame and restore sp
    addi    sp, sp, 272

    # Return from supervisor trap
    sret
