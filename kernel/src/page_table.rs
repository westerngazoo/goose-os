/// Sv39 page table structures — pure data, no hardware interaction.
///
/// RISC-V Sv39 uses 3-level page tables with 39-bit virtual addresses:
///
///   Virtual address (39 bits):
///   ┌─────────┬─────────┬─────────┬──────────────┐
///   │ VPN[2]   │ VPN[1]   │ VPN[0]   │  Page offset  │
///   │ 9 bits   │ 9 bits   │ 9 bits   │  12 bits      │
///   └─────────┴─────────┴─────────┴──────────────┘
///     bits 38-30  bits 29-21  bits 20-12  bits 11-0
///
///   Page Table Entry (64 bits):
///   ┌──────────────────────────────────┬──────────┐
///   │         PPN (44 bits)            │  Flags   │
///   │         bits 53-10               │ bits 9-0 │
///   └──────────────────────────────────┴──────────┘
///
/// Design decisions (formal verification path):
///   - PTE is a newtype over u64 — pure value type, no references
///   - All flag operations are const fn where possible
///   - PageTable is [PTE; 512] — exactly one 4KB page
///   - No unsafe in this module — all hardware interaction lives elsewhere

use crate::page_alloc::PAGE_SIZE;

/// Number of entries per page table (2^9 = 512).
pub const PT_ENTRIES: usize = 512;

// ── PTE Flags ───────────────────────────────────────────────────

/// PTE flag bits — each is a single bit in the low 10 bits of the PTE.
///
/// Formal property: flags are a set (bitwise OR composition is idempotent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PteFlags(u64);

impl PteFlags {
    pub const NONE:    PteFlags = PteFlags(0);
    pub const VALID:   PteFlags = PteFlags(1 << 0);  // V — entry is valid
    pub const READ:    PteFlags = PteFlags(1 << 1);  // R — readable
    pub const WRITE:   PteFlags = PteFlags(1 << 2);  // W — writable
    pub const EXECUTE: PteFlags = PteFlags(1 << 3);  // X — executable
    pub const USER:    PteFlags = PteFlags(1 << 4);  // U — accessible from U-mode
    pub const GLOBAL:  PteFlags = PteFlags(1 << 5);  // G — global mapping (not flushed on ASID switch)
    pub const ACCESS:  PteFlags = PteFlags(1 << 6);  // A — accessed (set by hardware or software)
    pub const DIRTY:   PteFlags = PteFlags(1 << 7);  // D — dirty (written to)

    /// Combine two flag sets (bitwise OR). Monoid: associative, NONE is identity.
    pub const fn union(self, other: PteFlags) -> PteFlags {
        PteFlags(self.0 | other.0)
    }

    /// Check if `other` flags are all present in `self`.
    pub const fn contains(self, other: PteFlags) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Raw bit value.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Create from raw bits (only low 10 bits used).
    pub const fn from_bits(bits: u64) -> PteFlags {
        PteFlags(bits & 0x3FF)
    }

    /// Is this a leaf PTE? (has R, W, or X set)
    /// Non-leaf PTEs point to the next level page table.
    /// Leaf PTEs map to a physical page.
    pub const fn is_leaf(self) -> bool {
        (self.0 & (Self::READ.0 | Self::WRITE.0 | Self::EXECUTE.0)) != 0
    }
}

// ── Common permission sets ──────────────────────────────────────

/// Kernel text: read + execute, no write (immutable code).
pub const KERNEL_RX: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 | PteFlags::EXECUTE.0 |
    PteFlags::ACCESS.0 | PteFlags::GLOBAL.0
);

/// Kernel read-only data.
pub const KERNEL_RO: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 |
    PteFlags::ACCESS.0 | PteFlags::GLOBAL.0
);

/// Kernel read-write data (BSS, stack, heap).
pub const KERNEL_RW: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 | PteFlags::WRITE.0 |
    PteFlags::ACCESS.0 | PteFlags::DIRTY.0 | PteFlags::GLOBAL.0
);

/// MMIO device registers (UART, PLIC): read-write, no execute, no cache.
pub const KERNEL_MMIO: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 | PteFlags::WRITE.0 |
    PteFlags::ACCESS.0 | PteFlags::DIRTY.0 | PteFlags::GLOBAL.0
);

/// User code: read + execute + user-accessible.
pub const USER_RX: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 | PteFlags::EXECUTE.0 |
    PteFlags::USER.0 | PteFlags::ACCESS.0
);

/// User data: read + write + user-accessible.
pub const USER_RW: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 | PteFlags::WRITE.0 |
    PteFlags::USER.0 | PteFlags::ACCESS.0 | PteFlags::DIRTY.0
);

/// User MMIO: read + write + user-accessible (for userspace device servers).
/// Same PTE bits as USER_RW — RISC-V cache control is via PMA, not PTEs.
/// Separate constant for documentation: these pages map device registers.
pub const USER_MMIO: PteFlags = PteFlags(
    PteFlags::VALID.0 | PteFlags::READ.0 | PteFlags::WRITE.0 |
    PteFlags::USER.0 | PteFlags::ACCESS.0 | PteFlags::DIRTY.0
);

// ── Page Table Entry ────────────────────────────────────────────

/// A single Sv39 page table entry.
///
/// Layout (64 bits):
///   Bits  0-7:   flags (V, R, W, X, U, G, A, D)
///   Bits  8-9:   RSW (reserved for software, we use 0)
///   Bits 10-53:  PPN (physical page number, 44 bits)
///   Bits 54-63:  reserved (must be 0)
///
/// Formal property: a PTE is a pure value. Two PTEs with the same bits are equal.
/// No hidden state, no pointers, no side effects.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Pte(u64);

impl Pte {
    /// An invalid (zero) PTE.
    pub const INVALID: Pte = Pte(0);

    /// Reconstruct a PTE from raw bits (read back from memory).
    pub const fn new_from_bits(bits: u64) -> Pte {
        Pte(bits)
    }

    /// Create a leaf PTE mapping to a physical address with given flags.
    ///
    /// `phys_addr` must be page-aligned (low 12 bits zero).
    /// The physical page number is extracted and stored in bits 10-53.
    pub const fn new(phys_addr: usize, flags: PteFlags) -> Pte {
        let ppn = (phys_addr as u64 >> 12) & 0x00FF_FFFF_FFFF_FFFF; // 44 bits
        Pte((ppn << 10) | flags.bits())
    }

    /// Create a non-leaf PTE pointing to the next level page table.
    ///
    /// `table_phys_addr` is the physical address of the child page table.
    /// Flags: Valid only (no R/W/X — this is a branch, not a leaf).
    pub const fn branch(table_phys_addr: usize) -> Pte {
        let ppn = (table_phys_addr as u64 >> 12) & 0x00FF_FFFF_FFFF_FFFF;
        Pte((ppn << 10) | PteFlags::VALID.0)
    }

    /// Is this PTE valid?
    pub const fn is_valid(self) -> bool {
        (self.0 & PteFlags::VALID.0) != 0
    }

    /// Is this a leaf PTE (maps to a physical page)?
    pub const fn is_leaf(self) -> bool {
        self.is_valid() && self.flags().is_leaf()
    }

    /// Is this a branch PTE (points to next level table)?
    pub const fn is_branch(self) -> bool {
        self.is_valid() && !self.flags().is_leaf()
    }

    /// Extract the flags (low 10 bits).
    pub const fn flags(self) -> PteFlags {
        PteFlags::from_bits(self.0)
    }

    /// Extract the physical page number (bits 10-53).
    pub const fn ppn(self) -> u64 {
        (self.0 >> 10) & 0x00FF_FFFF_FFFF_FFFF
    }

    /// Convert PPN back to a physical address.
    pub const fn phys_addr(self) -> usize {
        (self.ppn() << 12) as usize
    }

    /// Raw 64-bit value.
    pub const fn bits(self) -> u64 {
        self.0
    }
}

impl core::fmt::Debug for Pte {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if !self.is_valid() {
            write!(f, "PTE(invalid)")
        } else {
            let fl = self.flags();
            write!(f, "PTE(phys={:#x}, ", self.phys_addr())?;
            if fl.contains(PteFlags::READ)    { write!(f, "R")?; }
            if fl.contains(PteFlags::WRITE)   { write!(f, "W")?; }
            if fl.contains(PteFlags::EXECUTE) { write!(f, "X")?; }
            if fl.contains(PteFlags::USER)    { write!(f, "U")?; }
            if fl.contains(PteFlags::GLOBAL)  { write!(f, "G")?; }
            write!(f, ")")
        }
    }
}

// ── Virtual Address Decomposition ───────────────────────────────

/// Decompose a 39-bit virtual address into its VPN components and page offset.
///
/// Pure function — no side effects, trivially verifiable.
pub const fn va_parts(va: usize) -> (usize, usize, usize, usize) {
    let vpn2 = (va >> 30) & 0x1FF;   // bits 38-30 → index into level-2 (root) table
    let vpn1 = (va >> 21) & 0x1FF;   // bits 29-21 → index into level-1 table
    let vpn0 = (va >> 12) & 0x1FF;   // bits 20-12 → index into level-0 (leaf) table
    let offset = va & 0xFFF;          // bits 11-0  → offset within the 4KB page
    (vpn2, vpn1, vpn0, offset)
}

/// Construct a virtual address from VPN indices and offset.
pub const fn va_from_parts(vpn2: usize, vpn1: usize, vpn0: usize, offset: usize) -> usize {
    (vpn2 << 30) | (vpn1 << 21) | (vpn0 << 12) | offset
}

// ── SATP register ───────────────────────────────────────────────

/// Sv39 mode value for the satp CSR.
const SATP_SV39: u64 = 8; // mode field = 8 means Sv39

/// Build a satp register value for Sv39.
///
/// `root_table_phys` is the physical address of the root page table.
/// `asid` is the address space identifier (0 = no ASID).
pub const fn make_satp(root_table_phys: usize, asid: u16) -> u64 {
    let ppn = (root_table_phys as u64 >> 12) & 0x00FF_FFFF_FFFF_FFFF;
    (SATP_SV39 << 60) | ((asid as u64) << 44) | ppn
}

// ── Host-side unit tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PteFlags tests ──

    #[test]
    fn test_flags_none_is_identity() {
        let f = PteFlags::READ.union(PteFlags::NONE);
        assert_eq!(f, PteFlags::READ);
    }

    #[test]
    fn test_flags_union_associative() {
        let a = PteFlags::READ.union(PteFlags::WRITE).union(PteFlags::EXECUTE);
        let b = PteFlags::READ.union(PteFlags::WRITE.union(PteFlags::EXECUTE));
        assert_eq!(a, b);
    }

    #[test]
    fn test_flags_union_idempotent() {
        let f = PteFlags::READ.union(PteFlags::READ);
        assert_eq!(f, PteFlags::READ);
    }

    #[test]
    fn test_flags_contains() {
        let rw = PteFlags::READ.union(PteFlags::WRITE);
        assert!(rw.contains(PteFlags::READ));
        assert!(rw.contains(PteFlags::WRITE));
        assert!(!rw.contains(PteFlags::EXECUTE));
        assert!(rw.contains(PteFlags::NONE)); // NONE is always contained
    }

    #[test]
    fn test_flags_is_leaf() {
        assert!(PteFlags::READ.is_leaf());
        assert!(PteFlags::WRITE.is_leaf());
        assert!(PteFlags::EXECUTE.is_leaf());
        assert!(PteFlags::READ.union(PteFlags::WRITE).is_leaf());
        assert!(!PteFlags::VALID.is_leaf());   // V alone = branch
        assert!(!PteFlags::NONE.is_leaf());
        assert!(!PteFlags::USER.is_leaf());    // U without R/W/X is not leaf
    }

    #[test]
    fn test_kernel_permission_sets() {
        // Kernel text: readable, executable, not writable
        assert!(KERNEL_RX.contains(PteFlags::VALID));
        assert!(KERNEL_RX.contains(PteFlags::READ));
        assert!(KERNEL_RX.contains(PteFlags::EXECUTE));
        assert!(!KERNEL_RX.contains(PteFlags::WRITE));
        assert!(!KERNEL_RX.contains(PteFlags::USER));
        assert!(KERNEL_RX.is_leaf());

        // Kernel data: readable, writable, not executable
        assert!(KERNEL_RW.contains(PteFlags::READ));
        assert!(KERNEL_RW.contains(PteFlags::WRITE));
        assert!(!KERNEL_RW.contains(PteFlags::EXECUTE));
        assert!(!KERNEL_RW.contains(PteFlags::USER));

        // User code: readable, executable, user-accessible
        assert!(USER_RX.contains(PteFlags::USER));
        assert!(USER_RX.contains(PteFlags::READ));
        assert!(USER_RX.contains(PteFlags::EXECUTE));
        assert!(!USER_RX.contains(PteFlags::WRITE));
    }

    // ── PTE tests ──

    #[test]
    fn test_pte_invalid() {
        let p = Pte::INVALID;
        assert!(!p.is_valid());
        assert!(!p.is_leaf());
        assert!(!p.is_branch());
        assert_eq!(p.bits(), 0);
    }

    #[test]
    fn test_pte_leaf() {
        let phys = 0x8020_0000usize;
        let p = Pte::new(phys, KERNEL_RX);
        assert!(p.is_valid());
        assert!(p.is_leaf());
        assert!(!p.is_branch());
        assert_eq!(p.phys_addr(), phys);
        assert!(p.flags().contains(PteFlags::READ));
        assert!(p.flags().contains(PteFlags::EXECUTE));
    }

    #[test]
    fn test_pte_branch() {
        let table_addr = 0x8030_0000usize;
        let p = Pte::branch(table_addr);
        assert!(p.is_valid());
        assert!(p.is_branch());
        assert!(!p.is_leaf());
        assert_eq!(p.phys_addr(), table_addr);
    }

    #[test]
    fn test_pte_phys_addr_roundtrip() {
        // Test various physical addresses
        let addrs = [0x0, 0x1000, 0x8020_0000, 0x1_0000_0000, 0xF_FFFF_F000];
        for &addr in &addrs {
            let aligned = addr & !0xFFF; // ensure page-aligned
            let p = Pte::new(aligned, KERNEL_RW);
            assert_eq!(p.phys_addr(), aligned,
                "roundtrip failed for {:#x}", aligned);
        }
    }

    #[test]
    fn test_pte_preserves_all_flags() {
        let all = PteFlags::VALID.union(PteFlags::READ).union(PteFlags::WRITE)
            .union(PteFlags::EXECUTE).union(PteFlags::USER).union(PteFlags::GLOBAL)
            .union(PteFlags::ACCESS).union(PteFlags::DIRTY);
        let p = Pte::new(0x1000, all);
        assert_eq!(p.flags().bits() & 0xFF, all.bits() & 0xFF);
    }

    #[test]
    fn test_pte_unaligned_addr_truncates() {
        // Physical address 0x1234 is not page-aligned.
        // PTE stores PPN only (bits 12+), so low 12 bits are lost.
        let p = Pte::new(0x1234, KERNEL_RW);
        assert_eq!(p.phys_addr(), 0x1000); // truncated to page boundary
    }

    // ── Virtual address decomposition tests ──

    #[test]
    fn test_va_parts_zero() {
        let (vpn2, vpn1, vpn0, offset) = va_parts(0);
        assert_eq!((vpn2, vpn1, vpn0, offset), (0, 0, 0, 0));
    }

    #[test]
    fn test_va_parts_page_offset() {
        let (_, _, _, offset) = va_parts(0xABC);
        assert_eq!(offset, 0xABC);
    }

    #[test]
    fn test_va_parts_kernel_addr() {
        // 0x8020_0000 in Sv39:
        //   vpn2 = (0x80200000 >> 30) & 0x1FF = 0x200 >> 1 = 0x100 = 256
        //   Wait, let me recalculate properly
        //   0x80200000 = 0b 10_0000_0000_1 0_0000_0000 0_0000_0000 0000_0000_0000
        //   bits 38-30 = 10_0000_000 = 256
        //   bits 29-21 = 01_0000_000 = 128? No...
        //   0x80200000 = 0x80200000
        //   >> 30 = 0x80200000 / 0x40000000 = 2.00... → vpn2 = 2
        //   >> 21 & 0x1FF = (0x80200000 >> 21) & 0x1FF = 0x401 & 0x1FF = 1
        //   >> 12 & 0x1FF = (0x80200000 >> 12) & 0x1FF = 0x80200 & 0x1FF = 0
        let va = 0x8020_0000usize;
        let (vpn2, vpn1, vpn0, offset) = va_parts(va);
        assert_eq!(vpn2, 2,   "vpn2 for {:#x}", va);
        assert_eq!(vpn1, 1,   "vpn1 for {:#x}", va);
        assert_eq!(vpn0, 0,   "vpn0 for {:#x}", va);
        assert_eq!(offset, 0, "offset for {:#x}", va);
    }

    #[test]
    fn test_va_parts_roundtrip() {
        let original = 0x8020_1ABC;
        let (vpn2, vpn1, vpn0, offset) = va_parts(original);
        let reconstructed = va_from_parts(vpn2, vpn1, vpn0, offset);
        assert_eq!(reconstructed, original);
    }

    #[test]
    fn test_va_parts_all_ones() {
        // Maximum 39-bit address: 0x7F_FFFF_FFFF
        let va = 0x7F_FFFF_FFFF;
        let (vpn2, vpn1, vpn0, offset) = va_parts(va);
        assert_eq!(vpn2, 0x1FF); // 9 bits all set
        assert_eq!(vpn1, 0x1FF);
        assert_eq!(vpn0, 0x1FF);
        assert_eq!(offset, 0xFFF); // 12 bits all set
    }

    #[test]
    fn test_va_roundtrip_various() {
        let addrs = [0x0, 0x1000, 0x8020_0000, 0x4020_0000, 0x1000_0000, 0x0C00_0000];
        for &va in &addrs {
            let parts = va_parts(va);
            let rt = va_from_parts(parts.0, parts.1, parts.2, parts.3);
            assert_eq!(rt, va, "roundtrip failed for {:#x}", va);
        }
    }

    // ── SATP tests ──

    #[test]
    fn test_satp_sv39_mode() {
        let satp = make_satp(0x8030_0000, 0);
        let mode = satp >> 60;
        assert_eq!(mode, 8, "satp mode should be Sv39 (8)");
    }

    #[test]
    fn test_satp_ppn() {
        let root_phys = 0x8030_0000usize;
        let satp = make_satp(root_phys, 0);
        let ppn = satp & 0x00FF_FFFF_FFFF_FFFF;
        assert_eq!(ppn, (root_phys >> 12) as u64);
    }

    #[test]
    fn test_satp_asid() {
        let satp = make_satp(0x8030_0000, 42);
        let asid = (satp >> 44) & 0xFFFF;
        assert_eq!(asid, 42);
    }

    #[test]
    fn test_satp_zero_asid() {
        let satp = make_satp(0x8030_0000, 0);
        let asid = (satp >> 44) & 0xFFFF;
        assert_eq!(asid, 0);
    }
}

// ── Kani Proofs ─────────────────────────────────────────────────
//
// Machine-checked verification of page table invariants.
// Run with: cargo kani --harness <name>

#[cfg(kani)]
mod proofs {
    use super::*;

    /// va_parts → va_from_parts is identity for any 39-bit virtual address.
    #[kani::proof]
    fn proof_va_roundtrip() {
        let va: usize = kani::any();
        // Constrain to valid 39-bit address space
        kani::assume(va < (1 << 39));

        let (vpn2, vpn1, vpn0, offset) = va_parts(va);
        let reconstructed = va_from_parts(vpn2, vpn1, vpn0, offset);
        assert_eq!(reconstructed, va);
    }

    /// va_parts produces valid VPN indices (< 512) and offset (< 4096).
    #[kani::proof]
    fn proof_va_parts_bounds() {
        let va: usize = kani::any();
        let (vpn2, vpn1, vpn0, offset) = va_parts(va);
        assert!(vpn2 < PT_ENTRIES);
        assert!(vpn1 < PT_ENTRIES);
        assert!(vpn0 < PT_ENTRIES);
        assert!(offset < PAGE_SIZE);
    }

    /// PTE encode/decode: phys_addr round-trips for page-aligned addresses.
    #[kani::proof]
    fn proof_pte_phys_roundtrip() {
        let pa: usize = kani::any();
        // Constrain to page-aligned, within 56-bit physical address space
        kani::assume(pa % PAGE_SIZE == 0);
        kani::assume(pa < (1usize << 56));

        let pte = Pte::new(pa, KERNEL_RW);
        assert_eq!(pte.phys_addr(), pa);
    }

    /// PTE flags are preserved through encode/decode.
    #[kani::proof]
    fn proof_pte_flags_roundtrip() {
        let pa: usize = kani::any();
        kani::assume(pa % PAGE_SIZE == 0);
        kani::assume(pa < (1usize << 56));

        let flags = KERNEL_RX;
        let pte = Pte::new(pa, flags);
        assert_eq!(pte.flags(), flags);
    }

    /// Branch PTEs are valid and non-leaf.
    #[kani::proof]
    fn proof_branch_pte_invariants() {
        let table_pa: usize = kani::any();
        kani::assume(table_pa % PAGE_SIZE == 0);
        kani::assume(table_pa < (1usize << 56));

        let pte = Pte::branch(table_pa);
        assert!(pte.is_valid());
        assert!(pte.is_branch());
        assert!(!pte.is_leaf());
        assert_eq!(pte.phys_addr(), table_pa);
    }

    /// make_satp roundtrip: extract root physical address back.
    #[kani::proof]
    fn proof_satp_roundtrip() {
        let root: usize = kani::any();
        let asid: u16 = kani::any();
        kani::assume(root % PAGE_SIZE == 0);
        kani::assume(root < (1usize << 56));

        let satp = make_satp(root, asid);
        // satp_to_root logic (from kvm.rs): ((satp & mask) << 12)
        let extracted = ((satp & 0x0000_0FFF_FFFF_FFFF) << 12) as usize;
        assert_eq!(extracted, root);
    }

    /// W^X: kernel permission sets never have both WRITE and EXECUTE.
    #[kani::proof]
    fn proof_kernel_wx_separation() {
        assert!(!KERNEL_RX.contains(PteFlags::WRITE));
        assert!(!KERNEL_RW.contains(PteFlags::EXECUTE));
        assert!(!KERNEL_RO.contains(PteFlags::WRITE));
        assert!(!KERNEL_RO.contains(PteFlags::EXECUTE));
        assert!(!KERNEL_MMIO.contains(PteFlags::EXECUTE));
    }

    /// User permission sets never have both WRITE and EXECUTE.
    #[kani::proof]
    fn proof_user_wx_separation() {
        assert!(!USER_RX.contains(PteFlags::WRITE));
        assert!(!USER_RW.contains(PteFlags::EXECUTE));
        assert!(!USER_MMIO.contains(PteFlags::EXECUTE));
    }

    /// Kernel permission sets never have the USER bit.
    #[kani::proof]
    fn proof_kernel_no_user_bit() {
        assert!(!KERNEL_RX.contains(PteFlags::USER));
        assert!(!KERNEL_RW.contains(PteFlags::USER));
        assert!(!KERNEL_RO.contains(PteFlags::USER));
        assert!(!KERNEL_MMIO.contains(PteFlags::USER));
    }

    /// User permission sets always have the USER bit.
    #[kani::proof]
    fn proof_user_has_user_bit() {
        assert!(USER_RX.contains(PteFlags::USER));
        assert!(USER_RW.contains(PteFlags::USER));
        assert!(USER_MMIO.contains(PteFlags::USER));
    }

    /// PteFlags union is a monoid: NONE is identity.
    #[kani::proof]
    fn proof_flags_monoid_identity() {
        let bits: u64 = kani::any();
        kani::assume(bits < (1 << 10));
        let f = PteFlags(bits);
        assert_eq!(f.union(PteFlags::NONE), f);
        assert_eq!(PteFlags::NONE.union(f), f);
    }
}
