/// Bitmap-based physical page allocator.
///
/// Design decisions (for formal verification path):
///   - State is a bitvector — transitions are set/clear, both monoid ops
///   - Core invariant: bit N set ↔ page at (base + N * PAGE_SIZE) is allocated
///   - Pure logic: no unsafe, no MMIO, no linker symbols in the allocator core
///   - Kernel integration is a thin wrapper that reads linker symbols
///
/// Memory layout managed by this allocator:
///   _end (kernel image end) → _heap_end (stack bottom) = free pages

pub const PAGE_SIZE: usize = 4096;

/// Maximum pages we can track.
/// 128MB / 4KB = 32,768 pages → 32,768 bits → 512 u64 words → 4KB of bitmap.
/// We size for the worst case; actual page count may be smaller.
const MAX_PAGES: usize = 32_768;
const BITMAP_WORDS: usize = MAX_PAGES / 64;

/// Errors from allocator operations — explicit, no silent failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    OutOfMemory,
    DoubleFree,
    InvalidAddress,
    NotAligned,
}

// ── Global allocator instance ─────────────────────────────────
// Lives in .bss as zeroed memory, initialized once at boot.
// After init(), accessible from anywhere (boot code + syscall handlers).
static mut ALLOC: BitmapAllocator = BitmapAllocator::new(0, 0);

/// Get a mutable reference to the global page allocator.
///
/// # Safety
/// Caller must ensure single-threaded access (single-hart + interrupts off).
pub unsafe fn get() -> &'static mut BitmapAllocator {
    &mut *core::ptr::addr_of_mut!(ALLOC)
}

/// Bitmap page allocator.
///
/// Formal properties:
///   - `bitmap[i] & (1 << j)` set ↔ page `i*64 + j` is allocated
///   - `alloc()` finds first zero bit, sets it, returns page address
///   - `free(addr)` clears the bit — errors on double-free
///   - `free_count() + alloc_count() == total_pages` (conservation)
pub struct BitmapAllocator {
    bitmap: [u64; BITMAP_WORDS],
    base_addr: usize,
    total_pages: usize,
}

impl BitmapAllocator {
    /// Create a new allocator managing `num_pages` starting at `base`.
    ///
    /// All pages start FREE (bit = 0). Caller must mark any reserved pages.
    ///
    /// # Panics
    /// Panics if `num_pages > MAX_PAGES` or `base` is not page-aligned.
    pub const fn new(base: usize, num_pages: usize) -> Self {
        // Can't use assert! in const fn on stable, so we'll check at runtime
        BitmapAllocator {
            bitmap: [0u64; BITMAP_WORDS],
            base_addr: base,
            total_pages: num_pages,
        }
    }

    /// Initialize and validate parameters. Call once after construction.
    pub fn init(&self) {
        assert!(self.base_addr % PAGE_SIZE == 0, "base not page-aligned");
        assert!(self.total_pages > 0, "zero pages");
        assert!(self.total_pages <= MAX_PAGES, "too many pages");
    }

    /// Allocate one physical page. Returns the physical address.
    ///
    /// Scans bitmap for first zero bit, sets it, returns `base + index * PAGE_SIZE`.
    /// Returns `Err(OutOfMemory)` if all pages are allocated.
    pub fn alloc(&mut self) -> Result<usize, AllocError> {
        for word_idx in 0..self.words_used() {
            let word = self.bitmap[word_idx];
            if word == u64::MAX {
                continue; // all 64 bits set, skip
            }
            // Find first zero bit
            let bit_idx = (!word).trailing_zeros() as usize;
            let page_idx = word_idx * 64 + bit_idx;

            if page_idx >= self.total_pages {
                return Err(AllocError::OutOfMemory);
            }

            // Set the bit (mark allocated)
            self.bitmap[word_idx] |= 1u64 << bit_idx;
            return Ok(self.base_addr + page_idx * PAGE_SIZE);
        }
        Err(AllocError::OutOfMemory)
    }

    /// Free a previously allocated page.
    ///
    /// Clears the bit. Errors on double-free or invalid address.
    pub fn free(&mut self, addr: usize) -> Result<(), AllocError> {
        if addr % PAGE_SIZE != 0 {
            return Err(AllocError::NotAligned);
        }
        if addr < self.base_addr {
            return Err(AllocError::InvalidAddress);
        }
        let page_idx = (addr - self.base_addr) / PAGE_SIZE;
        if page_idx >= self.total_pages {
            return Err(AllocError::InvalidAddress);
        }

        let word_idx = page_idx / 64;
        let bit_idx = page_idx % 64;
        let mask = 1u64 << bit_idx;

        if self.bitmap[word_idx] & mask == 0 {
            return Err(AllocError::DoubleFree);
        }

        // Clear the bit (mark free)
        self.bitmap[word_idx] &= !mask;
        Ok(())
    }

    /// Check if a specific page address is currently allocated.
    pub fn is_allocated(&self, addr: usize) -> bool {
        if addr % PAGE_SIZE != 0 || addr < self.base_addr {
            return false;
        }
        let page_idx = (addr - self.base_addr) / PAGE_SIZE;
        if page_idx >= self.total_pages {
            return false;
        }
        let word_idx = page_idx / 64;
        let bit_idx = page_idx % 64;
        self.bitmap[word_idx] & (1u64 << bit_idx) != 0
    }

    /// Count of free (unallocated) pages.
    pub fn free_count(&self) -> usize {
        let allocated = self.allocated_count();
        self.total_pages - allocated
    }

    /// Count of allocated pages.
    pub fn allocated_count(&self) -> usize {
        let mut count = 0usize;
        for i in 0..self.words_used() {
            count += self.bitmap[i].count_ones() as usize;
        }
        count
    }

    /// Total pages managed by this allocator.
    pub fn total_pages(&self) -> usize {
        self.total_pages
    }

    /// Base physical address.
    pub fn base_addr(&self) -> usize {
        self.base_addr
    }

    /// Allocate a specific page by address (for reserving known regions).
    /// Returns `Err` if already allocated or invalid.
    pub fn mark_allocated(&mut self, addr: usize) -> Result<(), AllocError> {
        if addr % PAGE_SIZE != 0 {
            return Err(AllocError::NotAligned);
        }
        if addr < self.base_addr {
            return Err(AllocError::InvalidAddress);
        }
        let page_idx = (addr - self.base_addr) / PAGE_SIZE;
        if page_idx >= self.total_pages {
            return Err(AllocError::InvalidAddress);
        }

        let word_idx = page_idx / 64;
        let bit_idx = page_idx % 64;
        let mask = 1u64 << bit_idx;

        if self.bitmap[word_idx] & mask != 0 {
            return Err(AllocError::DoubleFree); // already allocated
        }

        self.bitmap[word_idx] |= mask;
        Ok(())
    }

    /// Zero a page (fill with 0x00). Requires the address to be valid.
    ///
    /// # Safety
    /// Caller must ensure `addr` is a valid, mapped, writable physical address.
    pub unsafe fn zero_page(addr: usize) {
        let ptr = addr as *mut u8;
        for i in 0..PAGE_SIZE {
            core::ptr::write_volatile(ptr.add(i), 0);
        }
    }

    /// How many u64 words are actually used for our page count.
    fn words_used(&self) -> usize {
        (self.total_pages + 63) / 64
    }
}

// ── Kernel integration (not available during host tests) ────────

/// Initialize the global page allocator from linker-script symbols.
///
/// This is the ONLY function that touches linker symbols.
/// Everything else is pure logic operating on the BitmapAllocator struct.
/// After this call, use `get()` to access the allocator from anywhere.
#[cfg(not(test))]
pub fn init() {
    extern "C" {
        static _end: u8;
        static _heap_end: u8;
    }

    let free_start = unsafe { &_end as *const u8 as usize };
    let free_end = unsafe { &_heap_end as *const u8 as usize };

    // Align start up to page boundary (should already be from linker)
    let base = (free_start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let num_pages = (free_end - base) / PAGE_SIZE;

    unsafe {
        core::ptr::addr_of_mut!(ALLOC).write(BitmapAllocator::new(base, num_pages));
        (*core::ptr::addr_of_mut!(ALLOC)).init();
    }
}

/// Boot self-test — run at startup to verify allocator correctness.
///
/// This is a runtime proof that the core invariants hold on this hardware.
/// If any assertion fails, the kernel panics before enabling the MMU.
/// Uses the global allocator (must call init() first).
#[cfg(not(test))]
pub fn self_test() {
    use crate::println;

    let alloc = unsafe { get() };

    // Invariant 1: all pages start free
    assert_eq!(alloc.allocated_count(), 0, "pages should start free");
    assert_eq!(alloc.free_count(), alloc.total_pages(), "free count mismatch");

    // Invariant 2: alloc returns a valid, page-aligned address
    let p1 = alloc.alloc().expect("alloc p1 failed");
    assert!(p1 % PAGE_SIZE == 0, "p1 not aligned");
    assert!(p1 >= alloc.base_addr(), "p1 below base");
    assert!(alloc.is_allocated(p1), "p1 not marked allocated");

    // Invariant 3: conservation — allocated + free = total
    assert_eq!(alloc.allocated_count() + alloc.free_count(), alloc.total_pages());

    // Invariant 4: free works, double-free is caught
    alloc.free(p1).expect("free p1 failed");
    assert!(!alloc.is_allocated(p1), "p1 still marked after free");
    assert_eq!(alloc.free(p1), Err(AllocError::DoubleFree), "double free not caught");

    // Invariant 5: freed page is reusable
    let p2 = alloc.alloc().expect("alloc p2 failed");
    assert_eq!(p1, p2, "freed page should be reallocated first");
    alloc.free(p2).expect("free p2 failed");

    // Invariant 6: sequential allocations don't overlap
    let a = alloc.alloc().expect("alloc a");
    let b = alloc.alloc().expect("alloc b");
    assert_ne!(a, b, "two allocations returned same page");
    assert_eq!(b, a + PAGE_SIZE, "sequential allocs should be contiguous");
    alloc.free(a).expect("free a");
    alloc.free(b).expect("free b");

    // Invariant 7: bad addresses are rejected
    assert_eq!(alloc.free(0xDEAD), Err(AllocError::NotAligned));
    assert_eq!(alloc.free(0), Err(AllocError::InvalidAddress));

    // Clean — all pages free again
    assert_eq!(alloc.allocated_count(), 0);

    println!("  [page_alloc] self-test passed ({} pages, {}MB)",
        alloc.total_pages(),
        alloc.total_pages() * PAGE_SIZE / (1024 * 1024));
}

// ── Host-side unit tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BASE: usize = 0x8000_0000;
    const TEST_PAGES: usize = 256; // 1MB

    fn make_alloc() -> BitmapAllocator {
        let mut a = BitmapAllocator::new(TEST_BASE, TEST_PAGES);
        a.init();
        a
    }

    #[test]
    fn test_new_allocator_all_free() {
        let a = make_alloc();
        assert_eq!(a.free_count(), TEST_PAGES);
        assert_eq!(a.allocated_count(), 0);
        assert_eq!(a.total_pages(), TEST_PAGES);
    }

    #[test]
    fn test_alloc_returns_base() {
        let mut a = make_alloc();
        let p = a.alloc().unwrap();
        assert_eq!(p, TEST_BASE);
    }

    #[test]
    fn test_alloc_sequential() {
        let mut a = make_alloc();
        let p1 = a.alloc().unwrap();
        let p2 = a.alloc().unwrap();
        let p3 = a.alloc().unwrap();
        assert_eq!(p1, TEST_BASE);
        assert_eq!(p2, TEST_BASE + PAGE_SIZE);
        assert_eq!(p3, TEST_BASE + 2 * PAGE_SIZE);
    }

    #[test]
    fn test_alloc_marks_allocated() {
        let mut a = make_alloc();
        let p = a.alloc().unwrap();
        assert!(a.is_allocated(p));
        assert!(!a.is_allocated(p + PAGE_SIZE)); // next page still free
    }

    #[test]
    fn test_conservation() {
        let mut a = make_alloc();
        for _ in 0..10 {
            a.alloc().unwrap();
        }
        assert_eq!(a.allocated_count() + a.free_count(), TEST_PAGES);
    }

    #[test]
    fn test_free_and_realloc() {
        let mut a = make_alloc();
        let p1 = a.alloc().unwrap();
        a.free(p1).unwrap();
        let p2 = a.alloc().unwrap();
        assert_eq!(p1, p2, "freed page should be reallocated");
    }

    #[test]
    fn test_double_free() {
        let mut a = make_alloc();
        let p = a.alloc().unwrap();
        a.free(p).unwrap();
        assert_eq!(a.free(p), Err(AllocError::DoubleFree));
    }

    #[test]
    fn test_free_invalid_address() {
        let mut a = make_alloc();
        assert_eq!(a.free(0), Err(AllocError::InvalidAddress));
        assert_eq!(a.free(TEST_BASE - PAGE_SIZE), Err(AllocError::InvalidAddress));
    }

    #[test]
    fn test_free_unaligned() {
        let mut a = make_alloc();
        assert_eq!(a.free(TEST_BASE + 1), Err(AllocError::NotAligned));
        assert_eq!(a.free(TEST_BASE + 13), Err(AllocError::NotAligned));
    }

    #[test]
    fn test_free_beyond_range() {
        let mut a = make_alloc();
        let beyond = TEST_BASE + TEST_PAGES * PAGE_SIZE;
        assert_eq!(a.free(beyond), Err(AllocError::InvalidAddress));
    }

    #[test]
    fn test_exhaust_all_pages() {
        let mut a = BitmapAllocator::new(TEST_BASE, 4);
        a.init();
        a.alloc().unwrap();
        a.alloc().unwrap();
        a.alloc().unwrap();
        a.alloc().unwrap();
        assert_eq!(a.alloc(), Err(AllocError::OutOfMemory));
        assert_eq!(a.free_count(), 0);
    }

    #[test]
    fn test_free_then_alloc_after_exhaust() {
        let mut a = BitmapAllocator::new(TEST_BASE, 2);
        a.init();
        let p1 = a.alloc().unwrap();
        let _p2 = a.alloc().unwrap();
        assert_eq!(a.alloc(), Err(AllocError::OutOfMemory));
        a.free(p1).unwrap();
        let p3 = a.alloc().unwrap();
        assert_eq!(p3, p1); // got the freed page back
    }

    #[test]
    fn test_mark_allocated() {
        let mut a = make_alloc();
        let addr = TEST_BASE + 10 * PAGE_SIZE;
        a.mark_allocated(addr).unwrap();
        assert!(a.is_allocated(addr));
        assert_eq!(a.allocated_count(), 1);

        // Trying to mark again should fail
        assert_eq!(a.mark_allocated(addr), Err(AllocError::DoubleFree));
    }

    #[test]
    fn test_alloc_skips_marked() {
        let mut a = BitmapAllocator::new(TEST_BASE, 4);
        a.init();
        // Mark page 0 as reserved
        a.mark_allocated(TEST_BASE).unwrap();
        // First alloc should skip page 0 and return page 1
        let p = a.alloc().unwrap();
        assert_eq!(p, TEST_BASE + PAGE_SIZE);
    }

    #[test]
    fn test_is_allocated_boundary() {
        let a = make_alloc();
        // Below base
        assert!(!a.is_allocated(TEST_BASE - PAGE_SIZE));
        // Above range
        assert!(!a.is_allocated(TEST_BASE + TEST_PAGES * PAGE_SIZE));
        // Unaligned
        assert!(!a.is_allocated(TEST_BASE + 1));
    }

    #[test]
    fn test_word_boundary_alloc() {
        // Allocate across a u64 word boundary (pages 63 and 64)
        let mut a = BitmapAllocator::new(TEST_BASE, 128);
        a.init();
        for i in 0..64 {
            let p = a.alloc().unwrap();
            assert_eq!(p, TEST_BASE + i * PAGE_SIZE);
        }
        // Page 64 is in word[1], bit 0
        let p64 = a.alloc().unwrap();
        assert_eq!(p64, TEST_BASE + 64 * PAGE_SIZE);
        assert!(a.is_allocated(p64));
    }

    #[test]
    fn test_alloc_free_all() {
        let num = 128;
        let mut a = BitmapAllocator::new(TEST_BASE, num);
        a.init();
        let mut pages = Vec::new();

        // Allocate all
        for _ in 0..num {
            pages.push(a.alloc().unwrap());
        }
        assert_eq!(a.alloc(), Err(AllocError::OutOfMemory));
        assert_eq!(a.allocated_count(), num);

        // Free all
        for p in pages {
            a.free(p).unwrap();
        }
        assert_eq!(a.free_count(), num);
        assert_eq!(a.allocated_count(), 0);
    }
}
