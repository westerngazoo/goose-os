/// Process management — create and launch userspace processes.
///
/// Part 6: First userspace process.
///
/// This module handles:
///   - Building a user page table (kernel mappings + user pages)
///   - Copying a user program into user memory
///   - Switching to U-mode via sret
///
/// The user program is embedded in the kernel image as assembly code.
/// At boot, we copy it to a freshly allocated user page and map it
/// with USER_RX permissions.

use core::arch::{asm, global_asm};
use crate::page_alloc::{BitmapAllocator, PAGE_SIZE};
use crate::page_table::*;
use crate::kvm;
use crate::println;

// ── Embedded user program ──────────────────────────────────────

// The first userspace program: prints "Honk!" via ecalls, then exits.
// Assembled as RISC-V machine code, linked into the kernel's .text section.
// We copy these bytes to a user page at runtime.
global_asm!(r#"
.section .text
.balign 4
.global _user_init_start
.global _user_init_end

_user_init_start:
    # ─── GooseOS init process ───
    # Prints "Honk! GooseOS userspace is alive!\n" via SYS_PUTCHAR ecalls.
    # Then exits via SYS_EXIT.
    #
    # Syscall convention:
    #   a7 = syscall number (0=putchar, 1=exit)
    #   a0 = argument (character or exit code)

    li      a7, 0           # SYS_PUTCHAR

    li      a0, 0x48        # 'H'
    ecall
    li      a0, 0x6F        # 'o'
    ecall
    li      a0, 0x6E        # 'n'
    ecall
    li      a0, 0x6B        # 'k'
    ecall
    li      a0, 0x21        # '!'
    ecall
    li      a0, 0x20        # ' '
    ecall
    li      a0, 0x47        # 'G'
    ecall
    li      a0, 0x6F        # 'o'
    ecall
    li      a0, 0x6F        # 'o'
    ecall
    li      a0, 0x73        # 's'
    ecall
    li      a0, 0x65        # 'e'
    ecall
    li      a0, 0x4F        # 'O'
    ecall
    li      a0, 0x53        # 'S'
    ecall
    li      a0, 0x20        # ' '
    ecall
    li      a0, 0x75        # 'u'
    ecall
    li      a0, 0x73        # 's'
    ecall
    li      a0, 0x65        # 'e'
    ecall
    li      a0, 0x72        # 'r'
    ecall
    li      a0, 0x73        # 's'
    ecall
    li      a0, 0x70        # 'p'
    ecall
    li      a0, 0x61        # 'a'
    ecall
    li      a0, 0x63        # 'c'
    ecall
    li      a0, 0x65        # 'e'
    ecall
    li      a0, 0x20        # ' '
    ecall
    li      a0, 0x69        # 'i'
    ecall
    li      a0, 0x73        # 's'
    ecall
    li      a0, 0x20        # ' '
    ecall
    li      a0, 0x61        # 'a'
    ecall
    li      a0, 0x6C        # 'l'
    ecall
    li      a0, 0x69        # 'i'
    ecall
    li      a0, 0x76        # 'v'
    ecall
    li      a0, 0x65        # 'e'
    ecall
    li      a0, 0x21        # '!'
    ecall
    li      a0, 0x0A        # '\n'
    ecall

    # Exit with code 42 (the answer)
    li      a0, 42          # exit code
    li      a7, 1           # SYS_EXIT
    ecall

    # Should never reach here — spin just in case
1:  j       1b

_user_init_end:
"#);

// ── Process launch ─────────────────────────────────────────────

/// Create a user page table and launch the first userspace process.
///
/// Steps:
///   1. Allocate user code page + user stack page
///   2. Copy embedded user program to user code page
///   3. Build user page table (kernel mappings + user pages)
///   4. Set up CPU state for U-mode entry
///   5. sret into userspace
pub fn launch_init(alloc: &mut BitmapAllocator) -> ! {
    extern "C" {
        static _user_init_start: u8;
        static _user_init_end: u8;
    }

    let prog_start = unsafe { &_user_init_start as *const u8 as usize };
    let prog_end = unsafe { &_user_init_end as *const u8 as usize };
    let prog_size = prog_end - prog_start;

    println!("  [proc] User program: {:#x} - {:#x} ({} bytes)",
        prog_start, prog_end, prog_size);

    // Allocate user code page
    let user_code_page = kvm::alloc_zeroed_page(alloc);
    println!("  [proc] User code page: {:#010x}", user_code_page);

    // Copy user program to user code page
    unsafe {
        let src = prog_start as *const u8;
        let dst = user_code_page as *mut u8;
        for i in 0..prog_size {
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        }
    }

    // Allocate user stack (one page, 4KB)
    let user_stack_page = kvm::alloc_zeroed_page(alloc);
    let user_sp = user_stack_page + PAGE_SIZE; // stack grows down — start at top
    println!("  [proc] User stack page: {:#010x} (sp = {:#010x})", user_stack_page, user_sp);

    // Build user page table
    let user_root = kvm::alloc_zeroed_page(alloc);
    println!("  [proc] User page table root: {:#010x}", user_root);

    // Map kernel regions (without U bit — S-mode accessible only)
    kvm::map_kernel_regions(user_root, alloc);

    // Map user code page as USER_RX (readable + executable + user-accessible)
    kvm::map_range(user_root, user_code_page, user_code_page + PAGE_SIZE, USER_RX, alloc);

    // Map user stack page as USER_RW (readable + writable + user-accessible)
    kvm::map_range(user_root, user_stack_page, user_stack_page + PAGE_SIZE, USER_RW, alloc);

    let user_satp = make_satp(user_root, 1); // ASID = 1 for first process
    let user_entry = user_code_page; // entry point = start of code page

    println!("  [proc] User satp: {:#018x}", user_satp);
    println!("  [proc] User entry: {:#010x}", user_entry);
    println!("  [proc] Launching init process...");
    println!();

    println!("  [page_alloc] {} pages used, {} free",
        alloc.allocated_count(), alloc.free_count());
    println!();

    // ── Switch to U-mode ───────────────────────────────────────
    //
    // Set up CSRs:
    //   sepc = user entry point (sret jumps here)
    //   sstatus.SPP = 0 (sret goes to U-mode, not S-mode)
    //   sstatus.SPIE = 1 (enable interrupts after sret)
    //   sscratch = kernel stack pointer (trap entry will swap it)
    //   satp = user page table
    //
    // Then sret: CPU drops to U-mode at sepc with interrupts enabled.

    unsafe {
        asm!(
            // Set sepc = user entry point
            "csrw sepc, {entry}",

            // Read sstatus, clear SPP (bit 8), set SPIE (bit 5)
            "csrr t0, sstatus",
            "li t1, -257",          // ~(1 << 8) = 0xFFFF...FEFF = -257 in two's complement
            "and t0, t0, t1",       // clear SPP
            "ori t0, t0, 32",       // set SPIE (bit 5 = 32)
            "csrw sstatus, t0",

            // Save kernel sp in sscratch (trap vector will swap it)
            "csrw sscratch, sp",

            // Switch to user page table
            "csrw satp, {satp}",
            "sfence.vma zero, zero",

            // Set user stack pointer
            "mv sp, {user_sp}",

            // Jump to userspace!
            "sret",

            entry = in(reg) user_entry,
            satp = in(reg) user_satp,
            user_sp = in(reg) user_sp,
            options(noreturn),
        );
    }
}
