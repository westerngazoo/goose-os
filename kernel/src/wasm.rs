/// WASM binary parser — decodes the WebAssembly module format.
///
/// Phase 14: Parse .wasm binaries embedded in the kernel image.
///
/// Supports:
///   - Module header (magic + version)
///   - Type section (function signatures)
///   - Function section (type index per function)
///   - Memory section (linear memory limits)
///   - Export section (named exports)
///   - Code section (function bodies)
///
/// The WASM spec is well-defined: https://webassembly.github.io/spec/core/binary/
/// This parser validates structure and extracts what the interpreter needs.
/// ~200 lines for a complete module parser.

// ── Constants ─────────────────────────────────────────────────

const WASM_MAGIC: [u8; 4] = [0x00, b'a', b's', b'm'];
const WASM_VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

// Section IDs (from the WASM binary spec)
const SEC_TYPE: u8 = 1;
const SEC_IMPORT: u8 = 2;
const SEC_FUNCTION: u8 = 3;
const SEC_MEMORY: u8 = 5;
const SEC_EXPORT: u8 = 7;
const SEC_CODE: u8 = 10;
const SEC_DATA: u8 = 11;

// Import kinds
const IMPORT_FUNC: u8 = 0x00;
const IMPORT_TABLE: u8 = 0x01;
const IMPORT_MEMORY: u8 = 0x02;
const IMPORT_GLOBAL: u8 = 0x03;

// Value types
pub const VAL_I32: u8 = 0x7F;
pub const VAL_I64: u8 = 0x7E;
pub const VAL_F32: u8 = 0x7D;
pub const VAL_F64: u8 = 0x7C;

// Export kinds
pub const EXPORT_FUNC: u8 = 0x00;
pub const EXPORT_TABLE: u8 = 0x01;
pub const EXPORT_MEMORY: u8 = 0x02;
pub const EXPORT_GLOBAL: u8 = 0x03;

// ── Limits ────────────────────────────────────────────────────

const MAX_TYPES: usize = 32;
const MAX_FUNCS: usize = 64;
const MAX_IMPORTS: usize = 16;
const MAX_EXPORTS: usize = 16;
const MAX_PARAMS: usize = 8;
const MAX_RESULTS: usize = 4;
const MAX_EXPORT_NAME: usize = 32;
const MAX_IMPORT_NAME: usize = 32;
const MAX_DATA_SEGS: usize = 8;

// ── Error Type ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmError {
    TooSmall,
    BadMagic,
    BadVersion,
    UnexpectedEof,
    InvalidLeb128,
    InvalidSection,
    InvalidValType,
    InvalidFuncType,
    InvalidExportKind,
    TooManyTypes,
    TooManyFunctions,
    TooManyExports,
    TooManyParams,
    TooManyResults,
    ExportNameTooLong,
    TypeIndexOutOfBounds,
    MissingCodeSection,
    TooManyImports,
    TooManyDataSegments,
    InvalidDataSegment,
    ImportNameTooLong,
}

// ── Parsed Structures ─────────────────────────────────────────

/// A function type (signature): params → results.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub struct FuncType {
    pub params: [u8; MAX_PARAMS],       // value types
    pub param_count: usize,
    pub results: [u8; MAX_RESULTS],     // value types
    pub result_count: usize,
}

impl FuncType {
    const fn empty() -> Self {
        FuncType {
            params: [0; MAX_PARAMS],
            param_count: 0,
            results: [0; MAX_RESULTS],
            result_count: 0,
        }
    }
}

/// Linear memory limits.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub struct MemoryLimits {
    pub min_pages: u32,     // initial size in 64KB pages
    pub max_pages: u32,     // maximum size (0 = no limit specified)
    pub has_max: bool,
}

/// An exported symbol.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub struct Export {
    pub name: [u8; MAX_EXPORT_NAME],
    pub name_len: usize,
    pub kind: u8,           // EXPORT_FUNC, EXPORT_MEMORY, etc.
    pub index: u32,         // index into the corresponding index space
}

impl Export {
    const fn empty() -> Self {
        Export {
            name: [0; MAX_EXPORT_NAME],
            name_len: 0,
            kind: 0,
            index: 0,
        }
    }

    /// Compare export name against a string.
    pub fn name_eq(&self, s: &[u8]) -> bool {
        if self.name_len != s.len() { return false; }
        let mut i = 0;
        while i < self.name_len {
            if self.name[i] != s[i] { return false; }
            i += 1;
        }
        true
    }
}

/// A data segment — pre-initialized bytes loaded into linear memory.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub struct DataSegment {
    pub offset: u32,         // destination offset in linear memory
    pub data_offset: usize,  // byte offset into the raw WASM binary
    pub data_len: usize,     // length of data bytes
}

impl DataSegment {
    const fn empty() -> Self {
        DataSegment { offset: 0, data_offset: 0, data_len: 0 }
    }
}

/// A function body from the code section.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub struct FuncBody {
    pub offset: usize,      // byte offset into the original WASM binary
    pub length: usize,      // length of the body (locals + code)
}

/// Parsed WASM module — everything the interpreter needs.
#[cfg_attr(test, derive(Debug))]
pub struct WasmModule {
    // Type section
    pub types: [FuncType; MAX_TYPES],
    pub type_count: usize,

    // Import section (function imports only — table/memory/global skipped)
    pub import_count: usize,
    pub import_types: [u32; MAX_IMPORTS],                // type index per import
    pub import_names: [[u8; MAX_IMPORT_NAME]; MAX_IMPORTS], // field name bytes
    pub import_name_lens: [usize; MAX_IMPORTS],          // field name lengths

    // Function section (type indices for LOCAL functions)
    pub func_types: [u32; MAX_FUNCS],   // type index for each local function
    pub func_count: usize,              // count of local functions (excludes imports)

    // Memory section
    pub memory: MemoryLimits,
    pub has_memory: bool,

    // Export section
    pub exports: [Export; MAX_EXPORTS],
    pub export_count: usize,

    // Code section (offsets into the binary)
    pub bodies: [FuncBody; MAX_FUNCS],
    pub body_count: usize,

    // Data section (pre-initialized linear memory)
    pub data_segments: [DataSegment; MAX_DATA_SEGS],
    pub data_segment_count: usize,
}

impl WasmModule {
    fn new() -> Self {
        WasmModule {
            types: [FuncType::empty(); MAX_TYPES],
            type_count: 0,
            import_count: 0,
            import_types: [0; MAX_IMPORTS],
            import_names: [[0; MAX_IMPORT_NAME]; MAX_IMPORTS],
            import_name_lens: [0; MAX_IMPORTS],
            func_types: [0; MAX_FUNCS],
            func_count: 0,
            memory: MemoryLimits { min_pages: 0, max_pages: 0, has_max: false },
            has_memory: false,
            exports: [Export::empty(); MAX_EXPORTS],
            export_count: 0,
            bodies: [FuncBody { offset: 0, length: 0 }; MAX_FUNCS],
            body_count: 0,
            data_segments: [DataSegment::empty(); MAX_DATA_SEGS],
            data_segment_count: 0,
        }
    }

    /// Find an exported function by name. Returns the function index.
    pub fn find_export(&self, name: &[u8], kind: u8) -> Option<u32> {
        let mut i = 0;
        while i < self.export_count {
            if self.exports[i].kind == kind && self.exports[i].name_eq(name) {
                return Some(self.exports[i].index);
            }
            i += 1;
        }
        None
    }

    /// Create an empty module (public for use in tests).
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self::new()
    }

    /// Compare import field name at index `i` against a string.
    pub fn import_name_eq(&self, i: usize, s: &[u8]) -> bool {
        if i >= self.import_count { return false; }
        if self.import_name_lens[i] != s.len() { return false; }
        let mut j = 0;
        while j < s.len() {
            if self.import_names[i][j] != s[j] { return false; }
            j += 1;
        }
        true
    }

    /// Get the type signature for a function by its function index.
    pub fn func_type(&self, func_idx: u32) -> Option<&FuncType> {
        let idx = func_idx as usize;
        if idx >= self.func_count { return None; }
        let type_idx = self.func_types[idx] as usize;
        if type_idx >= self.type_count { return None; }
        Some(&self.types[type_idx])
    }
}

// ── Cursor ────────────────────────────────────────────────────

/// Simple byte cursor for sequential reads from a byte slice.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn read_byte(&mut self) -> Result<u8, WasmError> {
        if self.pos >= self.data.len() {
            return Err(WasmError::UnexpectedEof);
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], WasmError> {
        if self.pos + n > self.data.len() {
            return Err(WasmError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// Read an unsigned LEB128 encoded u32.
    fn read_u32_leb128(&mut self) -> Result<u32, WasmError> {
        let mut result: u32 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            result |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift >= 35 {
                return Err(WasmError::InvalidLeb128);
            }
        }
    }

    fn skip(&mut self, n: usize) -> Result<(), WasmError> {
        if self.pos + n > self.data.len() {
            return Err(WasmError::UnexpectedEof);
        }
        self.pos += n;
        Ok(())
    }
}

// ── Parser ────────────────────────────────────────────────────

/// Parse a WASM binary from a byte slice.
///
/// Validates the header, iterates sections, and extracts type,
/// function, memory, export, and code sections. Skips unknown sections.
pub fn parse(data: &[u8]) -> Result<WasmModule, WasmError> {
    if data.len() < 8 {
        return Err(WasmError::TooSmall);
    }

    // Check magic + version
    if data[0..4] != WASM_MAGIC {
        return Err(WasmError::BadMagic);
    }
    if data[4..8] != WASM_VERSION {
        return Err(WasmError::BadVersion);
    }

    let mut module = WasmModule::new();
    let mut cursor = Cursor::new(data);
    cursor.pos = 8; // skip header

    // Parse sections
    while cursor.remaining() > 0 {
        let section_id = cursor.read_byte()?;
        let section_len = cursor.read_u32_leb128()? as usize;
        let section_start = cursor.pos;

        match section_id {
            SEC_TYPE => parse_type_section(&mut cursor, &mut module)?,
            SEC_IMPORT => parse_import_section(&mut cursor, &mut module)?,
            SEC_FUNCTION => parse_function_section(&mut cursor, &mut module)?,
            SEC_MEMORY => parse_memory_section(&mut cursor, &mut module)?,
            SEC_EXPORT => parse_export_section(&mut cursor, &mut module)?,
            SEC_CODE => parse_code_section(&mut cursor, &mut module, data)?,
            SEC_DATA => parse_data_section(&mut cursor, &mut module)?,
            _ => {
                // Skip unknown/unneeded sections (custom, table, element, global, etc.)
                cursor.skip(section_len)?;
            }
        }

        // Ensure cursor advanced exactly to the end of the section
        let expected_end = section_start + section_len;
        if cursor.pos != expected_end {
            // Section parser read wrong amount — force position
            cursor.pos = expected_end;
        }
    }

    // Validate: func_count must match body_count
    if module.func_count > 0 && module.body_count > 0 && module.func_count != module.body_count {
        return Err(WasmError::MissingCodeSection);
    }

    Ok(module)
}

// ── Section Parsers ───────────────────────────────────────────

fn parse_type_section(cursor: &mut Cursor, module: &mut WasmModule) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()? as usize;

    for _ in 0..count {
        if module.type_count >= MAX_TYPES {
            return Err(WasmError::TooManyTypes);
        }

        let form = cursor.read_byte()?;
        if form != 0x60 {
            return Err(WasmError::InvalidFuncType);
        }

        let mut ft = FuncType::empty();

        // Params
        let param_count = cursor.read_u32_leb128()? as usize;
        if param_count > MAX_PARAMS {
            return Err(WasmError::TooManyParams);
        }
        ft.param_count = param_count;
        for j in 0..param_count {
            let vt = cursor.read_byte()?;
            if !is_valid_valtype(vt) {
                return Err(WasmError::InvalidValType);
            }
            ft.params[j] = vt;
        }

        // Results
        let result_count = cursor.read_u32_leb128()? as usize;
        if result_count > MAX_RESULTS {
            return Err(WasmError::TooManyResults);
        }
        ft.result_count = result_count;
        for j in 0..result_count {
            let vt = cursor.read_byte()?;
            if !is_valid_valtype(vt) {
                return Err(WasmError::InvalidValType);
            }
            ft.results[j] = vt;
        }

        module.types[module.type_count] = ft;
        module.type_count += 1;
    }

    Ok(())
}

fn parse_function_section(cursor: &mut Cursor, module: &mut WasmModule) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()? as usize;

    for _ in 0..count {
        if module.func_count >= MAX_FUNCS {
            return Err(WasmError::TooManyFunctions);
        }

        let type_idx = cursor.read_u32_leb128()?;
        if type_idx as usize >= module.type_count {
            return Err(WasmError::TypeIndexOutOfBounds);
        }

        module.func_types[module.func_count] = type_idx;
        module.func_count += 1;
    }

    Ok(())
}

fn parse_memory_section(cursor: &mut Cursor, module: &mut WasmModule) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()?;

    // WASM MVP allows at most 1 memory
    if count >= 1 {
        let flags = cursor.read_byte()?;
        let min = cursor.read_u32_leb128()?;
        let (max, has_max) = if flags & 0x01 != 0 {
            (cursor.read_u32_leb128()?, true)
        } else {
            (0, false)
        };

        module.memory = MemoryLimits {
            min_pages: min,
            max_pages: max,
            has_max,
        };
        module.has_memory = true;

        // Skip additional memories (shouldn't exist in MVP)
        for _ in 1..count {
            let f = cursor.read_byte()?;
            cursor.read_u32_leb128()?; // min
            if f & 0x01 != 0 {
                cursor.read_u32_leb128()?; // max
            }
        }
    }

    Ok(())
}

fn parse_export_section(cursor: &mut Cursor, module: &mut WasmModule) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()? as usize;

    for _ in 0..count {
        if module.export_count >= MAX_EXPORTS {
            return Err(WasmError::TooManyExports);
        }

        let name_len = cursor.read_u32_leb128()? as usize;
        if name_len > MAX_EXPORT_NAME {
            return Err(WasmError::ExportNameTooLong);
        }

        let name_bytes = cursor.read_bytes(name_len)?;

        let kind = cursor.read_byte()?;
        if kind > EXPORT_GLOBAL {
            return Err(WasmError::InvalidExportKind);
        }

        let index = cursor.read_u32_leb128()?;

        let mut export = Export::empty();
        export.name_len = name_len;
        let mut i = 0;
        while i < name_len {
            export.name[i] = name_bytes[i];
            i += 1;
        }
        export.kind = kind;
        export.index = index;

        module.exports[module.export_count] = export;
        module.export_count += 1;
    }

    Ok(())
}

fn parse_code_section(
    cursor: &mut Cursor,
    module: &mut WasmModule,
    _raw: &[u8],
) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()? as usize;

    for _ in 0..count {
        if module.body_count >= MAX_FUNCS {
            return Err(WasmError::TooManyFunctions);
        }

        let body_size = cursor.read_u32_leb128()? as usize;
        let body_offset = cursor.pos;

        module.bodies[module.body_count] = FuncBody {
            offset: body_offset,
            length: body_size,
        };
        module.body_count += 1;

        cursor.skip(body_size)?;
    }

    Ok(())
}

fn parse_import_section(cursor: &mut Cursor, module: &mut WasmModule) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()? as usize;

    for _ in 0..count {
        // Read module name (e.g. "wasi_snapshot_preview1") — skip it
        let mod_name_len = cursor.read_u32_leb128()? as usize;
        cursor.skip(mod_name_len)?;

        // Read field name (e.g. "fd_write") — store it for WASI dispatch
        let field_name_len = cursor.read_u32_leb128()? as usize;
        let field_name_bytes = cursor.read_bytes(field_name_len)?;

        // Read import kind
        let kind = cursor.read_byte()?;

        match kind {
            IMPORT_FUNC => {
                if module.import_count >= MAX_IMPORTS {
                    return Err(WasmError::TooManyImports);
                }
                let type_idx = cursor.read_u32_leb128()?;
                if type_idx as usize >= module.type_count {
                    return Err(WasmError::TypeIndexOutOfBounds);
                }
                module.import_types[module.import_count] = type_idx;

                // Store field name (truncate if too long)
                let copy_len = if field_name_len <= MAX_IMPORT_NAME {
                    field_name_len
                } else {
                    MAX_IMPORT_NAME
                };
                let mut i = 0;
                while i < copy_len {
                    module.import_names[module.import_count][i] = field_name_bytes[i];
                    i += 1;
                }
                module.import_name_lens[module.import_count] = copy_len;

                module.import_count += 1;
            }
            IMPORT_TABLE => {
                // Table: element type + limits
                cursor.read_byte()?;  // element type (0x70 = funcref)
                let flags = cursor.read_byte()?;
                cursor.read_u32_leb128()?; // min
                if flags & 0x01 != 0 { cursor.read_u32_leb128()?; } // max
            }
            IMPORT_MEMORY => {
                // Memory: limits
                let flags = cursor.read_byte()?;
                cursor.read_u32_leb128()?; // min
                if flags & 0x01 != 0 { cursor.read_u32_leb128()?; } // max
            }
            IMPORT_GLOBAL => {
                // Global: value type + mutability
                cursor.read_byte()?;  // value type
                cursor.read_byte()?;  // mutability
            }
            _ => return Err(WasmError::InvalidSection),
        }
    }

    Ok(())
}

fn parse_data_section(cursor: &mut Cursor, module: &mut WasmModule) -> Result<(), WasmError> {
    let count = cursor.read_u32_leb128()? as usize;

    for _ in 0..count {
        if module.data_segment_count >= MAX_DATA_SEGS {
            return Err(WasmError::TooManyDataSegments);
        }

        // Data segment kind: 0 = active (memory 0, with offset expression)
        let kind = cursor.read_u32_leb128()?;
        if kind != 0 {
            // Passive (kind=1) or active-with-index (kind=2) — not needed for MVP
            return Err(WasmError::InvalidDataSegment);
        }

        // Evaluate constant offset expression: must be (i32.const N) (end)
        let opcode = cursor.read_byte()?;
        if opcode != 0x41 {
            // 0x41 = i32.const — only supported offset expression
            return Err(WasmError::InvalidDataSegment);
        }
        let offset = cursor.read_u32_leb128()?;

        let end = cursor.read_byte()?;
        if end != 0x0B {
            return Err(WasmError::InvalidDataSegment);
        }

        // Read data bytes
        let data_len = cursor.read_u32_leb128()? as usize;
        let data_offset = cursor.pos;
        cursor.skip(data_len)?;

        module.data_segments[module.data_segment_count] = DataSegment {
            offset,
            data_offset,
            data_len,
        };
        module.data_segment_count += 1;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────

fn is_valid_valtype(vt: u8) -> bool {
    matches!(vt, VAL_I32 | VAL_I64 | VAL_F32 | VAL_F64)
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid WASM module: empty module with just header.
    #[test]
    fn test_parse_empty_module() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, // magic: \0asm
            0x01, 0x00, 0x00, 0x00, // version: 1
        ];
        let module = parse(&wasm).expect("should parse empty module");
        assert_eq!(module.type_count, 0);
        assert_eq!(module.func_count, 0);
        assert_eq!(module.export_count, 0);
        assert_eq!(module.body_count, 0);
        assert!(!module.has_memory);
    }

    #[test]
    fn test_bad_magic() {
        let wasm = [0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
        assert_eq!(parse(&wasm).unwrap_err(), WasmError::BadMagic);
    }

    #[test]
    fn test_bad_version() {
        let wasm = [0x00, 0x61, 0x73, 0x6D, 0x02, 0x00, 0x00, 0x00];
        assert_eq!(parse(&wasm).unwrap_err(), WasmError::BadVersion);
    }

    #[test]
    fn test_too_small() {
        assert_eq!(parse(&[0x00, 0x61, 0x73]).unwrap_err(), WasmError::TooSmall);
    }

    /// Parse a module with one type: () -> i32
    #[test]
    fn test_parse_type_section() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            // Type section
            0x01,                   // section id = 1 (type)
            0x05,                   // section length = 5
            0x01,                   // count = 1 type
            0x60,                   // functype marker
            0x00,                   // 0 params
            0x01, 0x7F,             // 1 result: i32
        ];
        let module = parse(&wasm).expect("should parse type section");
        assert_eq!(module.type_count, 1);
        assert_eq!(module.types[0].param_count, 0);
        assert_eq!(module.types[0].result_count, 1);
        assert_eq!(module.types[0].results[0], VAL_I32);
    }

    /// Parse a module with type (i32, i32) -> i32
    #[test]
    fn test_parse_type_with_params() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            0x01,                   // type section
            0x07,                   // length = 7
            0x01,                   // 1 type
            0x60,                   // functype
            0x02, 0x7F, 0x7F,       // 2 params: i32, i32
            0x01, 0x7F,             // 1 result: i32
        ];
        let module = parse(&wasm).expect("should parse");
        assert_eq!(module.types[0].param_count, 2);
        assert_eq!(module.types[0].params[0], VAL_I32);
        assert_eq!(module.types[0].params[1], VAL_I32);
        assert_eq!(module.types[0].result_count, 1);
    }

    /// Parse a complete minimal module with one function that returns i32 42.
    ///
    /// Sections: type, function, export ("main"), code.
    #[test]
    fn test_parse_full_module() {
        let wasm = [
            // Header
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            // Type section: 1 type () -> i32
            0x01, 0x05,
            0x01, 0x60, 0x00, 0x01, 0x7F,
            // Function section: 1 function, type index 0
            0x03, 0x02,
            0x01, 0x00,
            // Export section: export "main" as function 0
            0x07, 0x08,
            0x01,                   // 1 export
            0x04,                   // name length = 4
            b'm', b'a', b'i', b'n', // "main"
            0x00,                   // kind = function
            0x00,                   // index = 0
            // Code section: 1 body
            0x0A, 0x06,
            0x01,                   // 1 body
            0x04,                   // body size = 4
            0x00,                   // 0 local declarations
            0x41, 0x2A,             // i32.const 42
            0x0B,                   // end
        ];

        let module = parse(&wasm).expect("should parse full module");

        // Types
        assert_eq!(module.type_count, 1);
        assert_eq!(module.types[0].param_count, 0);
        assert_eq!(module.types[0].result_count, 1);

        // Functions
        assert_eq!(module.func_count, 1);
        assert_eq!(module.func_types[0], 0); // function 0 uses type 0

        // Exports
        assert_eq!(module.export_count, 1);
        assert!(module.exports[0].name_eq(b"main"));
        assert_eq!(module.exports[0].kind, EXPORT_FUNC);
        assert_eq!(module.exports[0].index, 0);

        // Code
        assert_eq!(module.body_count, 1);
        assert_eq!(module.bodies[0].length, 4);

        // Find export by name
        assert_eq!(module.find_export(b"main", EXPORT_FUNC), Some(0));
        assert_eq!(module.find_export(b"nope", EXPORT_FUNC), None);

        // Function type lookup
        let ft = module.func_type(0).expect("should find func type");
        assert_eq!(ft.param_count, 0);
        assert_eq!(ft.result_count, 1);
    }

    /// Parse memory section with min and max.
    #[test]
    fn test_parse_memory_section() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            // Memory section: 1 memory, min=1, max=4
            0x05, 0x04,
            0x01,                   // 1 memory
            0x01,                   // flags: has max
            0x01,                   // min = 1 page (64KB)
            0x04,                   // max = 4 pages (256KB)
        ];
        let module = parse(&wasm).expect("should parse memory");
        assert!(module.has_memory);
        assert_eq!(module.memory.min_pages, 1);
        assert_eq!(module.memory.max_pages, 4);
        assert!(module.memory.has_max);
    }

    /// Parse memory section without max.
    #[test]
    fn test_parse_memory_no_max() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            0x05, 0x03,
            0x01,                   // 1 memory
            0x00,                   // flags: no max
            0x02,                   // min = 2 pages
        ];
        let module = parse(&wasm).expect("should parse");
        assert!(module.has_memory);
        assert_eq!(module.memory.min_pages, 2);
        assert!(!module.memory.has_max);
    }

    /// Unknown sections are skipped without error.
    #[test]
    fn test_skip_unknown_sections() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            // Custom section (id=0): "test" + 3 bytes payload
            0x00, 0x08,
            0x04, b't', b'e', b's', b't',
            0xAA, 0xBB, 0xCC,
            // Type section after custom
            0x01, 0x05,
            0x01, 0x60, 0x00, 0x01, 0x7F,
        ];
        let module = parse(&wasm).expect("should skip custom section");
        assert_eq!(module.type_count, 1);
    }

    /// LEB128 encoding of larger values.
    #[test]
    fn test_leb128_multibyte() {
        let mut cursor = Cursor::new(&[0x80, 0x01]); // 128 in LEB128
        assert_eq!(cursor.read_u32_leb128().unwrap(), 128);

        let mut cursor = Cursor::new(&[0xE5, 0x8E, 0x26]); // 624485
        assert_eq!(cursor.read_u32_leb128().unwrap(), 624485);
    }

    /// Multiple exports.
    #[test]
    fn test_multiple_exports() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            // Type section
            0x01, 0x05,
            0x01, 0x60, 0x00, 0x01, 0x7F,
            // Function section: 2 functions
            0x03, 0x03,
            0x02, 0x00, 0x00,
            // Export section: 2 exports
            0x07, 0x10,
            0x02,                   // 2 exports
            // export "add"
            0x03, b'a', b'd', b'd',
            0x00, 0x00,             // func 0
            // export "memory"
            0x06, b'm', b'e', b'm', b'o', b'r', b'y',
            0x02, 0x00,             // memory 0
        ];
        let module = parse(&wasm).expect("should parse");
        assert_eq!(module.export_count, 2);
        assert!(module.exports[0].name_eq(b"add"));
        assert_eq!(module.exports[0].kind, EXPORT_FUNC);
        assert!(module.exports[1].name_eq(b"memory"));
        assert_eq!(module.exports[1].kind, EXPORT_MEMORY);

        assert_eq!(module.find_export(b"add", EXPORT_FUNC), Some(0));
        assert_eq!(module.find_export(b"memory", EXPORT_MEMORY), Some(0));
        assert_eq!(module.find_export(b"add", EXPORT_MEMORY), None);
    }

    /// Parse import section: one function import (fd_write).
    #[test]
    fn test_parse_import_section() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            // Type section: type 0 = (i32,i32,i32,i32)->i32
            0x01, 0x09,
            0x01, 0x60,
            0x04, 0x7F, 0x7F, 0x7F, 0x7F, // 4 params: i32×4
            0x01, 0x7F,                     // 1 result: i32
            // Import section: 1 import
            // Content: 1 + (1+4) + (1+8) + 1 + 1 = 17
            0x02, 0x11,                     // section id=2, length=17
            0x01,                           // 1 import
            // module name: "wasi" (4 bytes)
            0x04, b'w', b'a', b's', b'i',
            // field name: "fd_write" (8 bytes)
            0x08, b'f', b'd', b'_', b'w', b'r', b'i', b't', b'e',
            // kind: function, type index: 0
            0x00, 0x00,
        ];
        let module = parse(&wasm).expect("should parse import");
        assert_eq!(module.import_count, 1);
        assert_eq!(module.import_types[0], 0);
        assert!(module.import_name_eq(0, b"fd_write"));
        assert!(!module.import_name_eq(0, b"fd_read"));
        // func_count should be 0 (no local functions)
        assert_eq!(module.func_count, 0);
    }

    /// Parse import + function + export: verify function index space.
    /// Import 0 = fd_write, Local function 0 = _start (exported as func index 1).
    #[test]
    fn test_parse_import_with_local_func() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            // Type section: 2 types
            0x01, 0x0C,                     // type section, len=12
            0x02,                           // 2 types
            0x60, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, // type 0: (i32×4)->i32
            0x60, 0x00, 0x00,               // type 1: ()->()
            // Import section: 1 function import
            // Content: 1 + (1+4) + (1+8) + 1 + 1 = 17
            0x02, 0x11,
            0x01,
            0x04, b'w', b'a', b's', b'i',
            0x08, b'f', b'd', b'_', b'w', b'r', b'i', b't', b'e',
            0x00, 0x00,                     // function, type 0
            // Function section: 1 local function, type 1
            0x03, 0x02,
            0x01, 0x01,
            // Export section: export "_start" as function 1
            0x07, 0x0A,
            0x01,
            0x06, b'_', b's', b't', b'a', b'r', b't',
            0x00, 0x01,                     // function index 1 (import 0 + local 0)
            // Code section: 1 body (empty _start)
            0x0A, 0x04,
            0x01,                           // 1 body
            0x02,                           // body size = 2
            0x00,                           // 0 locals
            0x0B,                           // end
        ];
        let module = parse(&wasm).expect("should parse import + func");
        assert_eq!(module.import_count, 1);
        assert_eq!(module.func_count, 1);
        assert_eq!(module.body_count, 1);
        // Export "_start" is at function index 1 (import_count + 0)
        assert_eq!(module.find_export(b"_start", EXPORT_FUNC), Some(1));
        // Import type check
        assert_eq!(module.import_types[0], 0);
        // Local function type check
        assert_eq!(module.func_types[0], 1);
    }

    /// Parse data section: one segment at offset 100, 5 bytes of data.
    #[test]
    fn test_parse_data_section() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            // Memory section: 1 page
            0x05, 0x03, 0x01, 0x00, 0x01,
            // Data section
            0x0B, 0x0B,                     // section id=11, length=11
            0x01,                           // 1 segment
            0x00,                           // kind=0 (active, memory 0)
            0x41, 0x64,                     // i32.const 100
            0x0B,                           // end expr
            0x05,                           // data length = 5
            b'H', b'e', b'l', b'l', b'o',  // "Hello"
        ];
        let module = parse(&wasm).expect("should parse data section");
        assert_eq!(module.data_segment_count, 1);
        assert_eq!(module.data_segments[0].offset, 100);
        assert_eq!(module.data_segments[0].data_len, 5);
        // Verify data_offset points to "Hello" in the raw binary
        let start = module.data_segments[0].data_offset;
        assert_eq!(&wasm[start..start + 5], b"Hello");
    }

    /// Parse a complete module with import, function, data, and export.
    #[test]
    fn test_parse_wasi_hello_module() {
        let wasm = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            // Type section: 2 types
            0x01, 0x0C,                     // type section, len=12
            0x02,
            0x60, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, // type 0: fd_write sig
            0x60, 0x00, 0x00,                                 // type 1: _start sig
            // Import section: 1 import (len=17)
            0x02, 0x11,
            0x01,
            0x04, b'w', b'a', b's', b'i',
            0x08, b'f', b'd', b'_', b'w', b'r', b'i', b't', b'e',
            0x00, 0x00,
            // Function section: 1 local
            0x03, 0x02,
            0x01, 0x01,
            // Memory section: 1 page
            0x05, 0x03, 0x01, 0x00, 0x01,
            // Export: "_start" = func 1, "memory" = memory 0 (len=19)
            0x07, 0x13,
            0x02,
            0x06, b'_', b's', b't', b'a', b'r', b't', 0x00, 0x01,
            0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00,
            // Code section
            0x0A, 0x04,
            0x01, 0x02, 0x00, 0x0B,
            // Data section: "Hi" at offset 50
            0x0B, 0x08,
            0x01, 0x00,
            0x41, 0x32,                     // i32.const 50
            0x0B,
            0x02, b'H', b'i',
        ];
        let module = parse(&wasm).expect("should parse WASI hello");
        assert_eq!(module.type_count, 2);
        assert_eq!(module.import_count, 1);
        assert!(module.import_name_eq(0, b"fd_write"));
        assert_eq!(module.func_count, 1);
        assert_eq!(module.export_count, 2);
        assert_eq!(module.find_export(b"_start", EXPORT_FUNC), Some(1));
        assert_eq!(module.find_export(b"memory", EXPORT_MEMORY), Some(0));
        assert!(module.has_memory);
        assert_eq!(module.data_segment_count, 1);
        assert_eq!(module.data_segments[0].offset, 50);
        assert_eq!(module.data_segments[0].data_len, 2);
    }
}
