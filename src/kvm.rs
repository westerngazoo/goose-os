/// Kernel Virtual Memory — builds the identity-mapped Sv39 page table.
///
/// This module is the ONLY place that writes to page table memory.
/// All page table data structures live in page_table.rs (pure, testable).
/// All page allocation lives in page_alloc.rs (pure, testable).
/// This module is the glue: it reads linker symbols, allocates pages,
/// and writes PTEs into physical memory.
///
/// Design constraints (formal verification path):
///   - The kernel page table is built ONCE at boot and NEVER modified
///   - After init() returns, no kernel PTE is ever written again
///   - This is the "identity element" — frozen, immutable, provably correct
///   - User page tables (Part 6+) are dynamic and live elsewhere

use crate::page_alloc::{BitmapAllocator, PAGE_SIZE};
use crate::page_table::*;
use crate::platform;
use core::ptr;

/// PLIC spans 0x0C00_0000 to 0x0FFF_FFFF (64MB, but we only need the active region).
/// Map 4MB to cover priority, pending, enable, and threshold/claim registers.
const PLIC_MAP_SIZE: usize = 4 * 1024 * 1024; // 4MB = 1024 pages

/// Kernel's satp value — stored after MMU enable so processes can switch back.
static mut KERNEL_SATP: u64 = 0;

/// Get the kernel's satp value (for switching back from user page tables).
pub fn kernel_satp() -> u64 {
    unsafe { KERNEL_SATP }
}

/// Build the kernel identity-mapped page table.
///
/// Returns the physical address of the root page table (for satp).
///
/// Identity map means: virtual address == physical address.
/// The kernel runs at the same addresses before and after MMU enable.
/// The MMU only adds protection — no writing .text, no executing .data.
///
/// Memory mapped (all identity-mapped):
///   .text      → R+X (immutable code)
///   .rodata    → R   (immutable data)
///   .data+.bss → R+W (mutable data)
///   stack      → R+W
///   free pages → R+W (page allocator region)
///   UART MMIO  → R+W (temporary — moves to userspace in Part 8)
///   PLIC MMIO  → R+W (stays in kernel — interrupt dispatch)
pub fn init(alloc: &mut BitmapAllocator) -> usize {
    // Read linker-script section boundaries
    let (text_start, text_end) = linker_range("_text_start", "_text_end");
    let (rodata_start, rodata_end) = linker_range("_rodata_start", "_rodata_end");
    let (data_start, data_end) = linker_range("_data_start", "_data_end");
    let (bss_start, bss_end) = linker_range("_bss_start", "_bss_end");
    let heap_start = linker_symbol("_end");
    let heap_end = linker_symbol("_heap_end");
    let stack_top = linker_symbol("_stack_top");

    // Allocate root page table (level 2)
    let root_phys = alloc_zeroed_page(alloc);

    crate::println!("  [kvm] Building kernel page table...");
    crate::println!("    .text   {:#010x} - {:#010x} (R+X)", text_start, text_end);
    crate::println!("    .rodata {:#010x} - {:#010x} (R  )", rodata_start, rodata_end);
    crate::println!("    .data   {:#010x} - {:#010x} (R+W)", data_start, data_end);
    crate::println!("    .bss    {:#010x} - {:#010x} (R+W)", bss_start, bss_end);
    crate::println!("    heap    {:#010x} - {:#010x} (R+W)", heap_start, heap_end);
    crate::println!("    stack   {:#010x} - {:#010x} (R+W)", heap_end, stack_top);

    // Map kernel sections with proper permissions (W^X enforced)
    map_range(root_phys, text_start, text_end, KERNEL_RX, alloc);
    map_range(root_phys, rodata_start, rodata_end, KERNEL_RO, alloc);
    map_range(root_phys, data_start, data_end, KERNEL_RW, alloc);
    map_range(root_phys, bss_start, bss_end, KERNEL_RW, alloc);

    // Map free page region (heap) + stack
    map_range(root_phys, heap_start, stack_top, KERNEL_RW, alloc);

    // Map UART MMIO (one page, temporary — will move to userspace)
    let uart_base = platform::UART_BASE;
    map_range(root_phys, uart_base, uart_base + PAGE_SIZE, KERNEL_MMIO, alloc);
    crate::println!("    UART    {:#010x} - {:#010x} (R+W, MMIO)", uart_base, uart_base + PAGE_SIZE);

    // Map PLIC MMIO (4MB covers all PLIC registers)
    let plic_base = platform::PLIC_BASE;
    map_range(root_phys, plic_base, plic_base + PLIC_MAP_SIZE, KERNEL_MMIO, alloc);
    crate::println!("    PLIC    {:#010x} - {:#010x} (R+W, MMIO)", plic_base, plic_base + PLIC_MAP_SIZE);

    crate::println!("  [kvm] Kernel page table at {:#010x}", root_phys);

    root_phys
}

/// Enable the MMU by writing the satp CSR.
///
/// THIS IS THE SCARIEST INSTRUCTION IN OS DEVELOPMENT.
///
/// After `csrw satp`, every instruction fetch, load, and store goes through
/// the page tables. If ANY needed address is not mapped — the next instruction
/// fetch faults, the trap handler faults, infinite loop, hard lock.
///
/// Prerequisites:
///   - All kernel code/data/stack is identity-mapped
///   - UART is mapped (so we can print after enable)
///   - PLIC is mapped (so interrupts still work)
///   - Trap vector is mapped (so exceptions are catchable)
///
/// # Safety
/// Caller must ensure the root page table at `root_phys` has valid identity
/// mappings for all memory the kernel will access after this call.
pub unsafe fn enable_mmu(root_phys: usize) {
    let satp_val = make_satp(root_phys, 0);

    // Store for later (process.rs needs this to switch back)
    KERNEL_SATP = satp_val;

    crate::println!("  [kvm] Enabling Sv39 MMU (satp = {:#018x})...", satp_val);

    core::arch::asm!(
        // Write satp — MMU is now ON
        "csrw satp, {0}",
        // Fence to ensure all subsequent accesses use new page tables
        "sfence.vma zero, zero",
        in(reg) satp_val,
    );

    // If we reach here, we survived. The MMU is active.
    crate::println!("  [kvm] MMU enabled — Sv39 active!");
}

// ── Internal helpers ────────────────────────────────────────────

/// Identity-map a range of physical pages into the root page table.
///
/// `start` and `end` are physical addresses. Both are rounded to page boundaries.
/// Each page in the range gets a leaf PTE with the given flags.
///
/// For each page, we walk/create the 3-level table:
///   root[vpn2] → level1[vpn1] → level0[vpn0] = leaf PTE
pub(crate) fn map_range(
    root_phys: usize,
    start: usize,
    end: usize,
    flags: PteFlags,
    alloc: &mut BitmapAllocator,
) {
    // Round start down, end up to page boundaries
    let start_aligned = start & !(PAGE_SIZE - 1);
    let end_aligned = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    let mut addr = start_aligned;
    while addr < end_aligned {
        map_page(root_phys, addr, addr, flags, alloc);
        addr += PAGE_SIZE;
    }
}

/// Map a single virtual page to a physical page in the 3-level Sv39 table.
///
/// Walks root → level1 → level0, allocating intermediate tables as needed.
pub(crate) fn map_page(
    root_phys: usize,
    va: usize,
    pa: usize,
    flags: PteFlags,
    alloc: &mut BitmapAllocator,
) {
    let (vpn2, vpn1, vpn0, _) = va_parts(va);

    // Level 2 (root): get or create level-1 table
    let level1_phys = walk_or_create(root_phys, vpn2, alloc);

    // Level 1: get or create level-0 table
    let level0_phys = walk_or_create(level1_phys, vpn1, alloc);

    // Level 0 (leaf): write the final PTE
    let pte = Pte::new(pa, flags);
    write_pte(level0_phys, vpn0, pte);
}

/// Read PTE at `table_phys + index * 8`. If it's a valid branch, return the
/// child table address. If invalid, allocate a new table and install a branch PTE.
pub(crate) fn walk_or_create(table_phys: usize, index: usize, alloc: &mut BitmapAllocator) -> usize {
    let existing = read_pte(table_phys, index);

    if existing.is_valid() {
        // Already have an intermediate table — return its address
        assert!(existing.is_branch(),
            "walk_or_create: expected branch PTE at index {}, got leaf", index);
        existing.phys_addr()
    } else {
        // Allocate a new page table page (zeroed = all entries invalid)
        let new_table = alloc_zeroed_page(alloc);
        let branch = Pte::branch(new_table);
        write_pte(table_phys, index, branch);
        new_table
    }
}

/// Read a PTE from a page table page.
fn read_pte(table_phys: usize, index: usize) -> Pte {
    assert!(index < PT_ENTRIES, "PTE index out of range: {}", index);
    let addr = table_phys + index * 8;
    let bits = unsafe { ptr::read_volatile(addr as *const u64) };
    Pte::new_from_bits(bits)
}

/// Write a PTE to a page table page.
fn write_pte(table_phys: usize, index: usize, pte: Pte) {
    assert!(index < PT_ENTRIES, "PTE index out of range: {}", index);
    let addr = table_phys + index * 8;
    unsafe { ptr::write_volatile(addr as *mut u64, pte.bits()); }
}

/// Allocate a zeroed page from the allocator.
pub(crate) fn alloc_zeroed_page(alloc: &mut BitmapAllocator) -> usize {
    let page = alloc.alloc().expect("kvm: out of memory for page tables");
    unsafe { BitmapAllocator::zero_page(page); }
    page
}

/// Map all kernel regions into an arbitrary root page table.
///
/// Used by process.rs to build user page tables that include kernel mappings.
/// The kernel regions are mapped WITHOUT the U bit, so user code can't access
/// them — but the kernel trap handler CAN, even when satp points to this table.
pub fn map_kernel_regions(root_phys: usize, alloc: &mut BitmapAllocator) {
    let (text_start, text_end) = linker_range("_text_start", "_text_end");
    let (rodata_start, rodata_end) = linker_range("_rodata_start", "_rodata_end");
    let (data_start, data_end) = linker_range("_data_start", "_data_end");
    let (bss_start, bss_end) = linker_range("_bss_start", "_bss_end");
    let heap_start = linker_symbol("_end");
    let stack_top = linker_symbol("_stack_top");

    map_range(root_phys, text_start, text_end, KERNEL_RX, alloc);
    map_range(root_phys, rodata_start, rodata_end, KERNEL_RO, alloc);
    map_range(root_phys, data_start, data_end, KERNEL_RW, alloc);
    map_range(root_phys, bss_start, bss_end, KERNEL_RW, alloc);
    map_range(root_phys, heap_start, stack_top, KERNEL_RW, alloc);

    // MMIO
    let uart_base = platform::UART_BASE;
    map_range(root_phys, uart_base, uart_base + PAGE_SIZE, KERNEL_MMIO, alloc);
    let plic_base = platform::PLIC_BASE;
    map_range(root_phys, plic_base, plic_base + PLIC_MAP_SIZE, KERNEL_MMIO, alloc);
}

// ── Linker symbol accessors ─────────────────────────────────────

/// Read a linker symbol as a usize address.
///
/// Linker symbols don't have values in the traditional sense — their ADDRESS
/// is the value. We take the address of the extern static to get it.
fn linker_symbol(name: &str) -> usize {
    extern "C" {
        static _text_start: u8;
        static _text_end: u8;
        static _rodata_start: u8;
        static _rodata_end: u8;
        static _data_start: u8;
        static _data_end: u8;
        static _bss_start: u8;
        static _bss_end: u8;
        static _end: u8;
        static _heap_end: u8;
        static _stack_top: u8;
    }

    unsafe {
        match name {
            "_text_start"   => &_text_start as *const u8 as usize,
            "_text_end"     => &_text_end as *const u8 as usize,
            "_rodata_start" => &_rodata_start as *const u8 as usize,
            "_rodata_end"   => &_rodata_end as *const u8 as usize,
            "_data_start"   => &_data_start as *const u8 as usize,
            "_data_end"     => &_data_end as *const u8 as usize,
            "_bss_start"    => &_bss_start as *const u8 as usize,
            "_bss_end"      => &_bss_end as *const u8 as usize,
            "_end"          => &_end as *const u8 as usize,
            "_heap_end"     => &_heap_end as *const u8 as usize,
            "_stack_top"    => &_stack_top as *const u8 as usize,
            _ => panic!("unknown linker symbol: {}", name),
        }
    }
}

/// Get a (start, end) range from two linker symbols.
fn linker_range(start_name: &str, end_name: &str) -> (usize, usize) {
    let s = linker_symbol(start_name);
    let e = linker_symbol(end_name);
    assert!(e >= s, "linker range {}-{} is inverted: {:#x} > {:#x}", start_name, end_name, s, e);
    (s, e)
}
