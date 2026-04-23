/// Syscall handlers — memory management, IRQ ownership, process spawn.
///
/// Split from process.rs for clarity. Accesses process table via pub(crate) globals.

use core::arch::asm;
use crate::page_alloc::PAGE_SIZE;
use crate::page_table::*;
use crate::process::{PROCS, CURRENT_PID, IRQ_OWNER, Process, ProcessState};
use crate::security::{self, MAX_PROCS};
use crate::trap::TrapFrame;
use crate::kvm;
use crate::println;

// ── Memory Management Syscalls ────────────────────────────────

/// SYS_MAP(phys, virt, flags) — map a physical page into caller's address space.
///
/// Convention: a0 = physical address, a1 = virtual address, a2 = flags.
///   flags: 0 = USER_RW, 1 = USER_RX
/// Returns: a0 = 0 (success), usize::MAX (error).
///
/// Validates:
///   - phys and virt are page-aligned
///   - virt is in user-mappable range (not kernel space)
///   - phys was allocated via SYS_ALLOC_PAGES
pub fn sys_map(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let phys = frame.a0;
    let virt = frame.a1;
    let flags_arg = frame.a2;

    // Validate alignment
    if !security::is_page_aligned(phys) || !security::is_page_aligned(virt) {
        frame.a0 = usize::MAX;
        return;
    }

    // Validate virt is in user range (below kernel space).
    // Reject addresses in kernel region and MMIO regions.
    // Simple check: user VA must be >= 0x5000_0000 and < 0x8000_0000
    // (avoids UART at 0x1000_0000, PLIC at 0x0C00_0000, kernel at 0x8020_0000+)
    if !security::is_user_va(virt) {
        frame.a0 = usize::MAX;
        return;
    }

    // Validate phys is an allocated page
    let alloc = unsafe { crate::page_alloc::get() };
    if !alloc.is_allocated(phys) {
        frame.a0 = usize::MAX;
        return;
    }

    // Map flags: 0 = USER_RW, 1 = USER_RX
    if !security::is_valid_map_flags(flags_arg) {
        frame.a0 = usize::MAX;
        return;
    }
    let pte_flags = match flags_arg {
        0 => USER_RW,
        _ => USER_RX,
    };

    // Get current process's page table root
    let current = unsafe { CURRENT_PID };
    let satp = unsafe { PROCS[current].satp };
    let root = kvm::satp_to_root(satp);

    // Map the page
    kvm::map_user_page(root, virt, phys, pte_flags);

    // Flush TLB for this VA
    unsafe {
        asm!("sfence.vma {}, zero", in(reg) virt);
    }

    frame.a0 = 0;
}

/// SYS_UNMAP(virt) — remove a page mapping from caller's address space.
///
/// Convention: a0 = virtual address.
/// Returns: a0 = 0 (success), usize::MAX (error).
///
/// Does NOT free the physical page — use SYS_FREE_PAGES for that.
pub fn sys_unmap(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let virt = frame.a0;

    if !security::is_page_aligned(virt) {
        frame.a0 = usize::MAX;
        return;
    }

    // SECURITY: Only allow unmapping user-range VAs.
    // Prevent user from unmapping kernel text/data/stack/MMIO pages
    // from their own page table — that would crash the kernel on next trap.
    if !security::is_user_va(virt) {
        frame.a0 = usize::MAX;
        return;
    }

    let current = unsafe { CURRENT_PID };
    let satp = unsafe { PROCS[current].satp };
    let root = kvm::satp_to_root(satp);

    if kvm::unmap_page(root, virt) {
        // Flush TLB for this VA
        unsafe {
            asm!("sfence.vma {}, zero", in(reg) virt);
        }
        frame.a0 = 0;
    } else {
        frame.a0 = usize::MAX;
    }
}

/// SYS_ALLOC_PAGES(count) — allocate physical pages.
///
/// Convention: a0 = count (must be 1 for now).
/// Returns: a0 = physical address of allocated page, usize::MAX on error.
///
/// The page is zeroed before returning. Caller must SYS_MAP it
/// before accessing it — the page is not automatically mapped.
pub fn sys_alloc_pages(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let count = frame.a0;

    // Only single-page allocations for now
    if !security::is_valid_alloc_count(count) {
        frame.a0 = usize::MAX;
        return;
    }

    let alloc = unsafe { crate::page_alloc::get() };
    match alloc.alloc() {
        Ok(phys) => {
            unsafe { crate::page_alloc::BitmapAllocator::zero_page(phys); }
            frame.a0 = phys;
        }
        Err(_) => {
            frame.a0 = usize::MAX;
        }
    }
}

/// SYS_FREE_PAGES(phys, count) — return physical pages to the kernel.
///
/// Convention: a0 = physical address, a1 = count (must be 1 for now).
/// Returns: a0 = 0 (success), usize::MAX (error).
///
/// The page is zeroed (security: don't leak data between processes).
/// Caller should SYS_UNMAP first if the page is still mapped.
pub fn sys_free_pages(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let phys = frame.a0;
    let count = frame.a1;

    if !security::is_valid_alloc_count(count) {
        frame.a0 = usize::MAX;
        return;
    }

    // Validate alignment
    if !security::is_page_aligned(phys) {
        frame.a0 = usize::MAX;
        return;
    }

    // SECURITY: Verify the page is actually allocated before zeroing.
    // Without this, a malicious process could pass a kernel page table
    // address and zero_page would destroy it before free() rejects it.
    let alloc = unsafe { crate::page_alloc::get() };
    if !alloc.is_allocated(phys) {
        frame.a0 = usize::MAX;
        return;
    }

    // Zero the page before freeing (prevent data leaks between processes)
    unsafe { crate::page_alloc::BitmapAllocator::zero_page(phys); }

    match alloc.free(phys) {
        Ok(()) => {
            frame.a0 = 0;
        }
        Err(_) => {
            frame.a0 = usize::MAX;
        }
    }
}

// ── IRQ Syscalls (Phase 13) ───────────────────────────────────

/// SYS_IRQ_REGISTER(irq_num) — claim ownership of a hardware interrupt.
///
/// Convention: a0 = IRQ number.
/// Returns: a0 = 0 (success), usize::MAX (error: invalid IRQ or already claimed).
///
/// After registration, when this IRQ fires at the PLIC, the kernel delivers
/// it as an IPC message (sender=0, msg=irq_num) to this process's SYS_RECEIVE.
/// Only one process can own an IRQ at a time.
pub fn sys_irq_register(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let current = unsafe { CURRENT_PID };
    let irq = frame.a0 as u32;

    if !security::is_valid_irq(irq as usize) {
        frame.a0 = usize::MAX;
        return;
    }

    unsafe {
        // Check if already claimed
        if IRQ_OWNER[irq as usize] != 0 {
            frame.a0 = usize::MAX;
            return;
        }

        IRQ_OWNER[irq as usize] = current;
        PROCS[current].irq_num = irq;
    }

    // Enable this IRQ at the PLIC now that a process owns it.
    // Before this, the PLIC ignores the IRQ even if the device asserts it.
    crate::plic::enable_irq(irq);

    println!("  [kernel] PID {} registered for IRQ {}", current, irq);
    frame.a0 = 0;
}

/// SYS_IRQ_ACK() — acknowledge completion of interrupt handling.
///
/// Convention: no arguments.
/// Returns: a0 = 0 (success), usize::MAX (error: no registered IRQ).
///
/// Completes the PLIC cycle for the process's registered IRQ.
/// The PLIC won't deliver the next instance of this IRQ until acknowledged.
/// Must be called after handling an IRQ notification from SYS_RECEIVE.
pub fn sys_irq_ack(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let current = unsafe { CURRENT_PID };
    let irq = unsafe { PROCS[current].irq_num };

    if irq == 0 {
        frame.a0 = usize::MAX;
        return;
    }

    // Complete the PLIC claim/complete cycle
    crate::plic::complete(irq);
    frame.a0 = 0;
}

/// Look up the owning PID for an IRQ. Returns 0 if unclaimed.
pub fn irq_owner(irq: u32) -> usize {
    if !security::is_valid_irq(irq as usize) { return 0; }
    unsafe { IRQ_OWNER[irq as usize] }
}

// ── Process Spawn ─────────────────────────────────────────────

/// SYS_SPAWN(elf_ptr, elf_len) — create a new process from an ELF binary.
///
/// Convention: a0 = pointer to ELF data (in caller's memory), a1 = length.
/// Returns: a0 = new PID, usize::MAX on error.
///
/// Parses the ELF, allocates pages for each LOAD segment, copies data,
/// builds a user page table, and registers the process as Ready.
pub fn sys_spawn(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let elf_ptr = frame.a0;
    let elf_len = frame.a1;
    let parent = unsafe { CURRENT_PID };

    // Basic validation
    if !security::is_valid_elf_size(elf_len) {
        frame.a0 = usize::MAX;
        return;
    }

    // Enable Supervisor User Memory access (SUM bit in sstatus).
    // Without this, S-mode cannot read pages with the U bit set.
    // trap.S will restore the original sstatus before sret, so
    // this only affects the current trap handling.
    unsafe { asm!("csrs sstatus, {}", in(reg) 1usize << 18); }

    // Read the ELF data from caller's address space
    let elf_data = unsafe {
        core::slice::from_raw_parts(elf_ptr as *const u8, elf_len)
    };

    // Parse ELF headers
    let info = match crate::elf::parse(elf_data) {
        Ok(info) => info,
        Err(_) => {
            frame.a0 = usize::MAX;
            return;
        }
    };

    // Find a free process slot
    let mut new_pid = 0;
    unsafe {
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::Free {
                new_pid = i;
                break;
            }
        }
    }
    if new_pid == 0 {
        frame.a0 = usize::MAX;
        return;
    }

    // Build user page table
    let user_root = kvm::alloc_zeroed_page();
    kvm::map_kernel_regions(user_root);

    // Load each segment
    for seg_idx in 0..info.num_segments {
        let seg = &info.segments[seg_idx];
        let flags = if seg.executable { USER_RX } else { USER_RW };

        let num_pages = crate::elf::pages_needed(seg.memsz, seg.vaddr);
        let base_va = seg.vaddr & !(PAGE_SIZE - 1);

        for p in 0..num_pages {
            let va = base_va + p * PAGE_SIZE;
            let page = kvm::alloc_zeroed_page();

            // Calculate how much of this page needs file data
            let page_start = va;
            let page_end = va + PAGE_SIZE;

            let seg_file_start = seg.vaddr;
            let seg_file_end = seg.vaddr + seg.filesz;

            // Overlap between [page_start..page_end] and [seg_file_start..seg_file_end]
            let copy_start = if seg_file_start > page_start { seg_file_start } else { page_start };
            let copy_end = if seg_file_end < page_end { seg_file_end } else { page_end };

            if copy_start < copy_end {
                let file_offset = seg.file_offset + (copy_start - seg.vaddr);
                let dst_offset = copy_start - page_start;
                let len = copy_end - copy_start;

                unsafe {
                    let src = elf_data.as_ptr().add(file_offset);
                    let dst = (page as *mut u8).add(dst_offset);
                    for b in 0..len {
                        core::ptr::write_volatile(dst.add(b), core::ptr::read_volatile(src.add(b)));
                    }
                }
            }
            // memsz > filesz portion is already zeroed (alloc_zeroed_page)

            kvm::map_user_page(user_root, va, page, flags);
        }
    }

    // Allocate user stack (one page)
    let user_stack = kvm::alloc_zeroed_page();
    let stack_va = 0x7FFF_0000; // Fixed stack VA for spawned processes
    kvm::map_user_page(user_root, stack_va, user_stack, USER_RW);
    let user_sp = stack_va + PAGE_SIZE;

    let satp = make_satp(user_root, new_pid as u16);

    // Set up initial context
    let mut ctx = TrapFrame::zero();
    ctx.sepc = info.entry;
    ctx.sp = user_sp;
    ctx.sstatus = 1 << 5; // SPIE=1, SPP=0 (U-mode)

    unsafe {
        PROCS[new_pid] = Process {
            pid: new_pid,
            state: ProcessState::Ready,
            satp,
            context: ctx,
            ipc_target: 0,
            ipc_value: 0,
            ipc_arg2: 0,
            ipc_arg3: 0,
            parent,
            exit_code: 0,
            irq_num: 0,
            irq_pending: false,
            net_op: crate::process::NetOp::None,
            net_socket: 0,
            net_buf_va: 0,
            net_buf_len: 0,
        };
    }

    println!("  [kernel] PID {} spawned by PID {} (entry={:#x})",
        new_pid, parent, info.entry);

    frame.a0 = new_pid;
}
