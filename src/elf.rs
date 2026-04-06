/// ELF64 parser for RISC-V — loads static executables into user page tables.
///
/// Only supports:
///   - ELF64, little-endian, RISC-V (ET_EXEC)
///   - PT_LOAD segments (everything else is ignored)
///   - Static linking (no dynamic linker, no relocations)
///
/// This is intentionally minimal. ~80 lines of parsing for a complete loader.

use crate::page_alloc::PAGE_SIZE;

// ── ELF Constants ─────────────────────────────────────────────

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;       // Little-endian
const ET_EXEC: u16 = 2;          // Executable file
const EM_RISCV: u16 = 0xF3;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
// const PF_W: u32 = 2;
// const PF_R: u32 = 4;

// ── ELF Structures ────────────────────────────────────────────

/// ELF64 file header — always at offset 0 (64 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Header {
    pub e_ident:     [u8; 16],
    pub e_type:      u16,
    pub e_machine:   u16,
    pub e_version:   u32,
    pub e_entry:     u64,
    pub e_phoff:     u64,
    pub e_shoff:     u64,
    pub e_flags:     u32,
    pub e_ehsize:    u16,
    pub e_phentsize: u16,
    pub e_phnum:     u16,
    pub e_shentsize: u16,
    pub e_shnum:     u16,
    pub e_shstrndx:  u16,
}

/// ELF64 program header — describes one loadable segment (56 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type:   u32,
    pub p_flags:  u32,
    pub p_offset: u64,
    pub p_vaddr:  u64,
    pub p_paddr:  u64,
    pub p_filesz: u64,
    pub p_memsz:  u64,
    pub p_align:  u64,
}

// ── Parsed ELF Info ───────────────────────────────────────────

/// A loadable segment extracted from the ELF.
#[derive(Clone, Copy)]
pub struct LoadSegment {
    pub file_offset: usize,   // Where in the ELF data
    pub vaddr: usize,         // Where in virtual memory
    pub filesz: usize,        // Bytes to copy from file
    pub memsz: usize,         // Total bytes in memory (memsz >= filesz)
    pub executable: bool,     // Contains code (determines USER_RX vs USER_RW)
}

/// Result of parsing an ELF file.
pub struct ElfInfo {
    pub entry: usize,                      // Entry point virtual address
    pub segments: [LoadSegment; 4],        // Up to 4 LOAD segments
    pub num_segments: usize,
}

// ── Error Type ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    NotElf64,
    NotLittleEndian,
    NotExecutable,
    NotRiscV,
    TooManySegments,
    SegmentOutOfBounds,
}

// ── Parser ────────────────────────────────────────────────────

/// Parse an ELF binary from a byte slice.
///
/// Validates the header, extracts PT_LOAD segments, returns entry point
/// and segment descriptors. Does NOT allocate or map anything — that's
/// the caller's job (SYS_SPAWN).
pub fn parse(data: &[u8]) -> Result<ElfInfo, ElfError> {
    // Need at least the ELF header (64 bytes)
    if data.len() < 64 {
        return Err(ElfError::TooSmall);
    }

    // Safety: data is long enough, Elf64Header is 64 bytes, repr(C).
    let hdr = unsafe { &*(data.as_ptr() as *const Elf64Header) };

    // Validate magic
    if hdr.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if hdr.e_ident[4] != ELFCLASS64 {
        return Err(ElfError::NotElf64);
    }
    if hdr.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::NotLittleEndian);
    }
    if hdr.e_type != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }
    if hdr.e_machine != EM_RISCV {
        return Err(ElfError::NotRiscV);
    }

    let entry = hdr.e_entry as usize;
    let phoff = hdr.e_phoff as usize;
    let phnum = hdr.e_phnum as usize;
    let phentsize = hdr.e_phentsize as usize;

    let mut info = ElfInfo {
        entry,
        segments: [LoadSegment {
            file_offset: 0, vaddr: 0, filesz: 0, memsz: 0, executable: false,
        }; 4],
        num_segments: 0,
    };

    // Iterate program headers
    for i in 0..phnum {
        let offset = phoff + i * phentsize;
        if offset + 56 > data.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        let phdr = unsafe { &*(data.as_ptr().add(offset) as *const Elf64Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        if info.num_segments >= 4 {
            return Err(ElfError::TooManySegments);
        }

        // Validate segment data is within the file
        let seg_end = phdr.p_offset as usize + phdr.p_filesz as usize;
        if seg_end > data.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        info.segments[info.num_segments] = LoadSegment {
            file_offset: phdr.p_offset as usize,
            vaddr: phdr.p_vaddr as usize,
            filesz: phdr.p_filesz as usize,
            memsz: phdr.p_memsz as usize,
            executable: (phdr.p_flags & PF_X) != 0,
        };
        info.num_segments += 1;
    }

    Ok(info)
}

/// Calculate how many pages a segment needs.
pub fn pages_needed(memsz: usize, vaddr: usize) -> usize {
    let start_page = vaddr & !(PAGE_SIZE - 1);
    let end = vaddr + memsz;
    let end_page = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    (end_page - start_page) / PAGE_SIZE
}
