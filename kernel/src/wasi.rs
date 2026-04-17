/// WASI bridge — maps WebAssembly System Interface calls to GooseOS operations.
///
/// Phase 16: The missing piece connecting the WASM interpreter to the outside world.
///
/// WASI preview1 defines ~45 functions that WASM modules can import from
/// "wasi_snapshot_preview1". We implement the minimum set for "Hello World":
///
///   fd_write          — write to stdout/stderr (→ UART)
///   proc_exit         — terminate the WASM program
///   environ_sizes_get — returns 0 (no environment variables)
///   environ_get       — no-op
///   args_sizes_get    — returns 0 (no arguments)
///   args_get          — no-op
///
/// The bridge implements the interpreter's `HostFunctions` trait, dispatching
/// imported function calls to the correct WASI handler based on the import
/// field name from the WASM binary.

use crate::wasm::WasmModule;
use crate::interp::HostFunctions;

// ── WASI Function Dispatch ────────────────────────────────────

const MAX_DISPATCH: usize = 16;

/// Known WASI functions we can handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WasiFunc {
    Unknown,
    FdWrite,
    ProcExit,
    EnvironSizesGet,
    EnvironGet,
    ArgsSizesGet,
    ArgsGet,
}

/// WASI bridge — maps import indices to WASI function handlers.
pub struct WasiBridge {
    dispatch: [WasiFunc; MAX_DISPATCH],
    import_count: usize,
    /// Set by proc_exit. Checked by the runner after interpreter returns.
    pub exit_code: Option<i32>,
    /// Accumulated output bytes (test mode only).
    #[cfg(test)]
    pub output: [u8; 4096],
    #[cfg(test)]
    pub output_len: usize,
}

impl WasiBridge {
    /// Build a dispatch table from a parsed WASM module's import names.
    pub fn from_module(module: &WasmModule) -> Self {
        let mut bridge = WasiBridge {
            dispatch: [WasiFunc::Unknown; MAX_DISPATCH],
            import_count: module.import_count,
            exit_code: None,
            #[cfg(test)]
            output: [0; 4096],
            #[cfg(test)]
            output_len: 0,
        };

        let mut i = 0;
        while i < module.import_count && i < MAX_DISPATCH {
            bridge.dispatch[i] = match_wasi_name(module, i);
            i += 1;
        }

        bridge
    }

    // ── WASI Function Implementations ─────────────────────────

    /// fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno
    ///
    /// Reads scatter-gather iov entries from WASM linear memory,
    /// writes each buffer to the UART (stdout/stderr), and stores
    /// the total byte count at nwritten_ptr.
    fn fd_write(&mut self, args: &[u64], memory: &mut [u8]) -> Result<Option<u64>, u32> {
        if args.len() < 4 { return Err(1); }

        let fd = args[0] as i32;
        let iovs_ptr = args[1] as u32 as usize;
        let iovs_len = args[2] as u32 as usize;
        let nwritten_ptr = args[3] as u32 as usize;

        // Only stdout (1) and stderr (2) are supported
        if fd != 1 && fd != 2 {
            return Ok(Some(8)); // EBADF
        }

        let mut total_written: u32 = 0;

        for i in 0..iovs_len {
            let iov_addr = iovs_ptr + i * 8;

            let buf_ptr = read_u32_le(memory, iov_addr)? as usize;
            let buf_len = read_u32_le(memory, iov_addr + 4)? as usize;

            if buf_ptr + buf_len > memory.len() {
                return Ok(Some(28)); // EINVAL — bad pointer
            }

            for j in 0..buf_len {
                let byte = memory[buf_ptr + j];
                self.emit_byte(byte);
            }

            total_written += buf_len as u32;
        }

        write_u32_le(memory, nwritten_ptr, total_written)?;
        Ok(Some(0)) // success
    }

    /// proc_exit(code) -> !
    ///
    /// Stores the exit code so the runner can retrieve it, then returns
    /// an error to halt the interpreter.
    fn proc_exit(&mut self, args: &[u64]) -> Result<Option<u64>, u32> {
        let code = if !args.is_empty() { args[0] as i32 } else { 0 };
        self.exit_code = Some(code);
        // Return Err to halt the interpreter. The runner checks exit_code.
        Err(0xDEAD)
    }

    /// environ_sizes_get(count_ptr, buf_size_ptr) -> errno
    ///
    /// Returns 0 environment variables.
    fn environ_sizes_get(&self, args: &[u64], memory: &mut [u8]) -> Result<Option<u64>, u32> {
        if args.len() < 2 { return Err(1); }
        let count_ptr = args[0] as u32 as usize;
        let buf_size_ptr = args[1] as u32 as usize;
        write_u32_le(memory, count_ptr, 0)?;
        write_u32_le(memory, buf_size_ptr, 0)?;
        Ok(Some(0))
    }

    /// environ_get(environ_ptr, environ_buf_ptr) -> errno
    fn environ_get(&self, _args: &[u64], _memory: &mut [u8]) -> Result<Option<u64>, u32> {
        Ok(Some(0)) // no-op, nothing to fill
    }

    /// args_sizes_get(argc_ptr, argv_buf_size_ptr) -> errno
    fn args_sizes_get(&self, args: &[u64], memory: &mut [u8]) -> Result<Option<u64>, u32> {
        if args.len() < 2 { return Err(1); }
        let argc_ptr = args[0] as u32 as usize;
        let buf_size_ptr = args[1] as u32 as usize;
        write_u32_le(memory, argc_ptr, 0)?;
        write_u32_le(memory, buf_size_ptr, 0)?;
        Ok(Some(0))
    }

    /// args_get(argv_ptr, argv_buf_ptr) -> errno
    fn args_get(&self, _args: &[u64], _memory: &mut [u8]) -> Result<Option<u64>, u32> {
        Ok(Some(0)) // no-op
    }

    // ── Output helpers ────────────────────────────────────────

    /// Emit a byte: UART in kernel mode, buffer in test mode.
    fn emit_byte(&mut self, byte: u8) {
        #[cfg(test)]
        {
            if self.output_len < self.output.len() {
                self.output[self.output_len] = byte;
                self.output_len += 1;
            }
        }
        #[cfg(not(test))]
        {
            // Convert \n to \r\n for serial terminal
            if byte == b'\n' {
                crate::uart::Uart::platform().putc(b'\r');
            }
            crate::uart::Uart::platform().putc(byte);
        }
    }
}

// ── HostFunctions trait implementation ────────────────────────

impl HostFunctions for WasiBridge {
    fn call(
        &mut self,
        func_idx: u32,
        args: &[u64],
        memory: &mut [u8],
    ) -> Result<Option<u64>, u32> {
        let idx = func_idx as usize;
        if idx >= self.import_count {
            return Err(func_idx);
        }

        match self.dispatch[idx] {
            WasiFunc::FdWrite => self.fd_write(args, memory),
            WasiFunc::ProcExit => self.proc_exit(args),
            WasiFunc::EnvironSizesGet => self.environ_sizes_get(args, memory),
            WasiFunc::EnvironGet => self.environ_get(args, memory),
            WasiFunc::ArgsSizesGet => self.args_sizes_get(args, memory),
            WasiFunc::ArgsGet => self.args_get(args, memory),
            WasiFunc::Unknown => Err(func_idx), // unsupported WASI function
        }
    }
}

// ── Memory helpers ───────────────────────────────────────────

fn read_u32_le(memory: &[u8], addr: usize) -> Result<u32, u32> {
    if addr + 4 > memory.len() { return Err(28); } // EINVAL
    Ok(u32::from_le_bytes([
        memory[addr],
        memory[addr + 1],
        memory[addr + 2],
        memory[addr + 3],
    ]))
}

fn write_u32_le(memory: &mut [u8], addr: usize, val: u32) -> Result<(), u32> {
    if addr + 4 > memory.len() { return Err(28); } // EINVAL
    let bytes = val.to_le_bytes();
    memory[addr] = bytes[0];
    memory[addr + 1] = bytes[1];
    memory[addr + 2] = bytes[2];
    memory[addr + 3] = bytes[3];
    Ok(())
}

// ── Import name matching ─────────────────────────────────────

fn match_wasi_name(module: &WasmModule, idx: usize) -> WasiFunc {
    if module.import_name_eq(idx, b"fd_write") { WasiFunc::FdWrite }
    else if module.import_name_eq(idx, b"proc_exit") { WasiFunc::ProcExit }
    else if module.import_name_eq(idx, b"environ_sizes_get") { WasiFunc::EnvironSizesGet }
    else if module.import_name_eq(idx, b"environ_get") { WasiFunc::EnvironGet }
    else if module.import_name_eq(idx, b"args_sizes_get") { WasiFunc::ArgsSizesGet }
    else if module.import_name_eq(idx, b"args_get") { WasiFunc::ArgsGet }
    else { WasiFunc::Unknown }
}

// ── Data segment initialization ──────────────────────────────

/// Copy data segments from the WASM binary into linear memory.
///
/// Called before interpreter execution to pre-populate strings, tables,
/// and other static data that the WASM module expects to find in memory.
pub fn init_data_segments(module: &WasmModule, wasm_bytes: &[u8], memory: &mut [u8]) {
    for i in 0..module.data_segment_count {
        let seg = &module.data_segments[i];
        let dst_start = seg.offset as usize;
        let dst_end = dst_start + seg.data_len;
        let src_start = seg.data_offset;
        let src_end = src_start + seg.data_len;

        // Bounds check — skip bad segments rather than panicking
        if dst_end > memory.len() || src_end > wasm_bytes.len() {
            continue;
        }

        let mut j = 0;
        while j < seg.data_len {
            memory[dst_start + j] = wasm_bytes[src_start + j];
            j += 1;
        }
    }
}

// ── Top-level runner ─────────────────────────────────────────

/// Run a WASM module from raw bytes. Returns the exit code.
///
/// This is the main entry point: parse → allocate memory → init data →
/// build WASI bridge → create interpreter → call _start → return exit code.
#[cfg(not(test))]
pub fn run_wasm(wasm_bytes: &[u8]) -> i32 {
    use crate::wasm;
    use crate::interp::Interpreter;

    // 1. Parse
    let module = match wasm::parse(wasm_bytes) {
        Ok(m) => m,
        Err(e) => {
            crate::println!("  [wasm] Parse error: {:?}", e);
            return -1;
        }
    };

    crate::println!("  [wasm] Module: {} imports, {} functions, {} data segments",
        module.import_count, module.func_count, module.data_segment_count);

    // 2. Allocate linear memory (static to avoid stack overflow)
    static mut WASM_MEMORY: [u8; 65536] = [0; 65536];
    let memory = unsafe {
        // Zero the memory (may be reused across runs)
        let mut i = 0;
        while i < WASM_MEMORY.len() {
            WASM_MEMORY[i] = 0;
            i += 1;
        }
        &mut WASM_MEMORY
    };

    // 3. Initialize data segments
    init_data_segments(&module, wasm_bytes, memory);

    // 4. Build WASI bridge
    let mut bridge = WasiBridge::from_module(&module);

    // 5. Create interpreter
    let num_imports = module.import_count as u32;
    let mut interp = Interpreter::new(&module, wasm_bytes, num_imports);

    // 6. Find _start export
    let start_idx = match module.find_export(b"_start", wasm::EXPORT_FUNC) {
        Some(idx) => idx,
        None => {
            crate::println!("  [wasm] No _start export found");
            return -2;
        }
    };

    crate::println!("  [wasm] Running _start (func index {})...", start_idx);
    crate::println!();

    // 7. Execute
    match interp.call_function(start_idx, &[], memory, &mut bridge) {
        Ok(_) => bridge.exit_code.unwrap_or(0),
        Err(crate::interp::TrapKind::HostCallError(0xDEAD)) => {
            // proc_exit was called — clean exit
            bridge.exit_code.unwrap_or(0)
        }
        Err(e) => {
            crate::println!("\n  [wasm] Trap: {:?}", e);
            -3
        }
    }
}

/// Run the built-in WASM test module. Called from kmain with wasm-test feature.
#[cfg(not(test))]
pub fn run_wasm_test() -> i32 {
    run_wasm(&HELLO_WASM)
}

// ── Hand-crafted test WASM binary ────────────────────────────
//
// This is a complete, valid WASM module that prints "Hello from WASM!\n"
// using WASI fd_write. Built byte-by-byte, no toolchain needed.
//
// Module structure:
//   Import: fd_write from "wasi_snapshot_preview1"
//   Function: _start (calls fd_write to print the message)
//   Memory: 1 page (64KB)
//   Data: "Hello from WASM!\n" at offset 100
//   Export: _start (function 1), memory (memory 0)
//
// _start body:
//   1. Store iov at memory[0]: { buf_ptr=100, buf_len=17 }
//   2. Call fd_write(fd=1, iovs=0, iovs_len=1, nwritten=8)
//   3. Drop result
//   4. Return (end)

// Carefully hand-counted byte-by-byte. Each section length was computed
// from the content that follows it (not including the id+length bytes).
//
// Signed LEB128 note: i32.const uses SLEB128. For value 100 (0x64),
// bit 6 is set, so a single byte 0x64 would decode as -28. Two bytes
// needed: 0xE4, 0x00 → (0x64 & 0x7F) | ((0x00 & 0x7F) << 7) = 100.
#[rustfmt::skip]
const HELLO_WASM: [u8; 147] = [
    // ── Header (8 bytes) ──
    0x00, 0x61, 0x73, 0x6D,   // magic: \0asm
    0x01, 0x00, 0x00, 0x00,   // version: 1

    // ── Type Section (id=1, length=12) ──  [14 bytes total]
    0x01, 0x0C,                 // id=1, len=12
    0x02,                       // 2 types
    // Type 0: (i32, i32, i32, i32) -> i32  [fd_write]
    0x60, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F,
    // Type 1: () -> ()  [_start]
    0x60, 0x00, 0x00,

    // ── Import Section (id=2, length=35) ──  [37 bytes total]
    // Content: 1 + (1+22) + (1+8) + 1 + 1 = 35
    0x02, 0x23,                 // id=2, len=35
    0x01,                       // 1 import
    // module: "wasi_snapshot_preview1" (22 bytes)
    0x16,
    b'w', b'a', b's', b'i', b'_', b's', b'n', b'a',
    b'p', b's', b'h', b'o', b't', b'_', b'p', b'r',
    b'e', b'v', b'i', b'e', b'w', b'1',
    // field: "fd_write" (8 bytes)
    0x08,
    b'f', b'd', b'_', b'w', b'r', b'i', b't', b'e',
    // kind=function, type=0
    0x00, 0x00,

    // ── Function Section (id=3, length=2) ──  [4 bytes total]
    0x03, 0x02,                 // id=3, len=2
    0x01, 0x01,                 // 1 function, type index=1

    // ── Memory Section (id=5, length=3) ──  [5 bytes total]
    0x05, 0x03,                 // id=5, len=3
    0x01, 0x00, 0x01,           // 1 memory, no max, min=1 page

    // ── Export Section (id=7, length=19) ──  [21 bytes total]
    // Content: 1 + (1+6+1+1) + (1+6+1+1) = 19
    0x07, 0x13,                 // id=7, len=19
    0x02,                       // 2 exports
    0x06, b'_', b's', b't', b'a', b'r', b't', 0x00, 0x01,
    0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00,

    // ── Code Section (id=10, length=30) ──  [32 bytes total]
    // Content: 1 (count) + 1 (body_size) + 28 (body) = 30
    // Body: 1 + 2 + 3 + 3 + 2 + 2 + 3 + 2 + 2 + 2 + 2 + 2 + 1 + 1 = 28
    0x0A, 0x1E,                 // id=10, len=30
    0x01,                       // 1 body
    0x1C,                       // body size = 28
    0x00,                       // 0 local declarations
    // -- store iov[0].buf_ptr = 100 at memory[0] --
    0x41, 0x00,                 // i32.const 0     (addr for store)
    0x41, 0xE4, 0x00,           // i32.const 100   (value: SLEB128)
    0x36, 0x02, 0x00,           // i32.store align=2 offset=0
    // -- store iov[0].buf_len = 17 at memory[4] --
    0x41, 0x04,                 // i32.const 4     (addr for store)
    0x41, 0x11,                 // i32.const 17    (value)
    0x36, 0x02, 0x00,           // i32.store align=2 offset=0
    // -- call fd_write(1, 0, 1, 8) --
    0x41, 0x01,                 // i32.const 1     (fd = stdout)
    0x41, 0x00,                 // i32.const 0     (iovs_ptr)
    0x41, 0x01,                 // i32.const 1     (iovs_len)
    0x41, 0x08,                 // i32.const 8     (nwritten_ptr)
    0x10, 0x00,                 // call 0          (fd_write = import 0)
    0x1A,                       // drop            (discard errno)
    0x0B,                       // end

    // ── Data Section (id=11, length=24) ──  [26 bytes total]
    // Content: 1 + 1 + 3 + 1 + 1 + 17 = 24
    0x0B, 0x18,                 // id=11, len=24
    0x01,                       // 1 segment
    0x00,                       // kind=0 (active, memory 0)
    0x41, 0xE4, 0x00,           // i32.const 100   (offset: SLEB128)
    0x0B,                       // end expression
    0x11,                       // data length = 17
    b'H', b'e', b'l', b'l', b'o', b' ',
    b'f', b'r', b'o', b'm', b' ',
    b'W', b'A', b'S', b'M', b'!',
    0x0A,                       // '\n'
];

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm;
    use crate::interp::Interpreter;

    /// Test the memory read/write helpers.
    #[test]
    fn test_memory_helpers() {
        let mut mem = [0u8; 16];
        write_u32_le(&mut mem, 0, 0x12345678).unwrap();
        assert_eq!(read_u32_le(&mem, 0).unwrap(), 0x12345678);

        write_u32_le(&mut mem, 4, 42).unwrap();
        assert_eq!(read_u32_le(&mem, 4).unwrap(), 42);

        // Out of bounds
        assert!(read_u32_le(&mem, 14).is_err());
        assert!(write_u32_le(&mut mem, 14, 0).is_err());
    }

    /// Test fd_write with a simple iov pointing to "Hi".
    #[test]
    fn test_fd_write_stdout() {
        // Set up a mock module with one import (fd_write)
        let mut module = wasm::WasmModule::new_for_test();
        module.import_count = 1;
        module.import_name_lens[0] = 8;
        module.import_names[0][..8].copy_from_slice(b"fd_write");

        let mut bridge = WasiBridge::from_module(&module);

        let mut memory = [0u8; 256];
        // Place string "Hi" at offset 50
        memory[50] = b'H';
        memory[51] = b'i';
        // Place iov at offset 0: { buf_ptr=50, buf_len=2 }
        write_u32_le(&mut memory, 0, 50).unwrap();
        write_u32_le(&mut memory, 4, 2).unwrap();

        // Call fd_write(fd=1, iovs_ptr=0, iovs_len=1, nwritten_ptr=8)
        let args = [1u64, 0, 1, 8];
        let result = bridge.fd_write(&args, &mut memory);
        assert_eq!(result, Ok(Some(0))); // success

        // Check nwritten
        assert_eq!(read_u32_le(&memory, 8).unwrap(), 2);

        // Check captured output
        assert_eq!(&bridge.output[..bridge.output_len], b"Hi");
    }

    /// Test fd_write with bad file descriptor.
    #[test]
    fn test_fd_write_bad_fd() {
        let module = wasm::WasmModule::new_for_test();
        let mut bridge = WasiBridge::from_module(&module);
        let mut memory = [0u8; 64];

        let args = [42u64, 0, 0, 0]; // fd=42 (not stdout/stderr)
        let result = bridge.fd_write(&args, &mut memory);
        assert_eq!(result, Ok(Some(8))); // EBADF
    }

    /// Test proc_exit sets exit code.
    #[test]
    fn test_proc_exit() {
        let module = wasm::WasmModule::new_for_test();
        let mut bridge = WasiBridge::from_module(&module);

        let args = [42u64];
        let result = bridge.proc_exit(&args);
        assert!(result.is_err()); // should halt
        assert_eq!(bridge.exit_code, Some(42));
    }

    /// Test environ_sizes_get writes zeros.
    #[test]
    fn test_environ_sizes_get() {
        let module = wasm::WasmModule::new_for_test();
        let bridge = WasiBridge::from_module(&module);
        let mut memory = [0xFFu8; 32];

        let args = [0u64, 4]; // count_ptr=0, buf_size_ptr=4
        let result = bridge.environ_sizes_get(&args, &mut memory);
        assert_eq!(result, Ok(Some(0)));
        assert_eq!(read_u32_le(&memory, 0).unwrap(), 0);
        assert_eq!(read_u32_le(&memory, 4).unwrap(), 0);
    }

    /// Test args_sizes_get writes zeros.
    #[test]
    fn test_args_sizes_get() {
        let module = wasm::WasmModule::new_for_test();
        let bridge = WasiBridge::from_module(&module);
        let mut memory = [0xFFu8; 32];

        let args = [0u64, 4];
        let result = bridge.args_sizes_get(&args, &mut memory);
        assert_eq!(result, Ok(Some(0)));
        assert_eq!(read_u32_le(&memory, 0).unwrap(), 0);
        assert_eq!(read_u32_le(&memory, 4).unwrap(), 0);
    }

    /// Test data segment initialization.
    #[test]
    fn test_init_data_segments() {
        let wasm_bytes: [u8; 64] = {
            let mut b = [0u8; 64];
            // Simulate data content at offset 50 in the binary
            b[50] = b'A';
            b[51] = b'B';
            b[52] = b'C';
            b
        };

        let mut module = wasm::WasmModule::new_for_test();
        module.data_segment_count = 1;
        module.data_segments[0] = crate::wasm::DataSegment {
            offset: 10,        // destination in linear memory
            data_offset: 50,   // source in wasm_bytes
            data_len: 3,
        };

        let mut memory = [0u8; 256];
        init_data_segments(&module, &wasm_bytes, &mut memory);

        assert_eq!(memory[10], b'A');
        assert_eq!(memory[11], b'B');
        assert_eq!(memory[12], b'C');
        assert_eq!(memory[13], 0); // not overwritten
    }

    /// Integration test: parse the hand-crafted HELLO_WASM binary and
    /// run it through the interpreter with WASI bridge.
    #[test]
    fn test_hello_wasm_integration() {
        // 1. Parse
        let module = wasm::parse(&HELLO_WASM).expect("should parse HELLO_WASM");
        assert_eq!(module.import_count, 1);
        assert!(module.import_name_eq(0, b"fd_write"));
        assert_eq!(module.func_count, 1);
        assert_eq!(module.data_segment_count, 1);
        assert_eq!(module.data_segments[0].offset, 100);
        assert_eq!(module.data_segments[0].data_len, 17);

        // 2. Allocate linear memory
        let mut memory = [0u8; 65536];

        // 3. Initialize data segments
        init_data_segments(&module, &HELLO_WASM, &mut memory);

        // Verify data was loaded
        assert_eq!(&memory[100..117], b"Hello from WASM!\n");

        // 4. Build WASI bridge
        let mut bridge = WasiBridge::from_module(&module);
        assert_eq!(bridge.dispatch[0], WasiFunc::FdWrite);

        // 5. Create interpreter
        let num_imports = module.import_count as u32;
        let mut interp = Interpreter::new(&module, &HELLO_WASM, num_imports);

        // 6. Find _start
        let start_idx = module.find_export(b"_start", wasm::EXPORT_FUNC)
            .expect("should find _start");
        assert_eq!(start_idx, 1); // import 0 + local 0

        // 7. Execute
        let result = interp.call_function(start_idx, &[], &mut memory, &mut bridge);
        assert!(result.is_ok(), "interpreter should succeed: {:?}", result);

        // 8. Verify output
        assert_eq!(&bridge.output[..bridge.output_len], b"Hello from WASM!\n");
    }
}
