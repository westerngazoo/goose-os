/// WASM interpreter — stack-based bytecode execution engine.
///
/// Phase 15: Execute WASM function bodies parsed by wasm.rs.
///
/// Implements a subset of WASM MVP opcodes:
///   - i32/i64 arithmetic, comparison, bitwise
///   - Control flow: block, loop, br, br_if, if/else, return, call
///   - Linear memory: i32.load, i32.store, i32.load8_u, i32.store8
///   - Variables: local.get, local.set, local.tee, global.get, global.set
///   - Constants: i32.const, i64.const
///   - Misc: drop, select, nop, unreachable
///   - Host calls: call to imported function index triggers host callback
///
/// Design constraints (no_std, no alloc):
///   - Fixed-size operand stack (256 entries)
///   - Fixed-size call stack (32 frames)
///   - Linear memory provided as a mutable byte slice by the caller
///   - Host function calls via a callback trait

use crate::wasm::{WasmModule, FuncBody, VAL_I32, VAL_I64};

// ── Constants ─────────────────────────────────────────────────

const MAX_STACK: usize = 256;
const MAX_CALL_DEPTH: usize = 32;
const MAX_LOCALS: usize = 32;
const MAX_LABELS: usize = 32;

// ── WASM Opcodes ──────────────────────────────────────────────

mod op {
    // Control
    pub const UNREACHABLE: u8 = 0x00;
    pub const NOP: u8 = 0x01;
    pub const BLOCK: u8 = 0x02;
    pub const LOOP: u8 = 0x03;
    pub const IF: u8 = 0x04;
    pub const ELSE: u8 = 0x05;
    pub const END: u8 = 0x0B;
    pub const BR: u8 = 0x0C;
    pub const BR_IF: u8 = 0x0D;
    pub const RETURN: u8 = 0x0F;
    pub const CALL: u8 = 0x10;

    // Parametric
    pub const DROP: u8 = 0x1A;
    pub const SELECT: u8 = 0x1B;

    // Variable
    pub const LOCAL_GET: u8 = 0x20;
    pub const LOCAL_SET: u8 = 0x21;
    pub const LOCAL_TEE: u8 = 0x22;

    // Memory
    pub const I32_LOAD: u8 = 0x28;
    pub const I64_LOAD: u8 = 0x29;
    pub const I32_LOAD8_S: u8 = 0x2C;
    pub const I32_LOAD8_U: u8 = 0x2D;
    pub const I32_LOAD16_S: u8 = 0x2E;
    pub const I32_LOAD16_U: u8 = 0x2F;
    pub const I32_STORE: u8 = 0x36;
    pub const I64_STORE: u8 = 0x37;
    pub const I32_STORE8: u8 = 0x38;
    pub const I32_STORE16: u8 = 0x39;
    pub const MEMORY_SIZE: u8 = 0x3F;
    pub const MEMORY_GROW: u8 = 0x40;

    // Constants
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;

    // i32 comparison
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_NE: u8 = 0x47;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_LT_U: u8 = 0x49;
    pub const I32_GT_S: u8 = 0x4A;
    pub const I32_GT_U: u8 = 0x4B;
    pub const I32_LE_S: u8 = 0x4C;
    pub const I32_LE_U: u8 = 0x4D;
    pub const I32_GE_S: u8 = 0x4E;
    pub const I32_GE_U: u8 = 0x4F;

    // i64 comparison
    pub const I64_EQZ: u8 = 0x50;
    pub const I64_EQ: u8 = 0x51;
    pub const I64_NE: u8 = 0x52;
    pub const I64_LT_S: u8 = 0x53;
    pub const I64_LT_U: u8 = 0x54;
    pub const I64_GT_S: u8 = 0x55;
    pub const I64_GT_U: u8 = 0x56;
    pub const I64_LE_S: u8 = 0x57;
    pub const I64_LE_U: u8 = 0x58;
    pub const I64_GE_S: u8 = 0x59;
    pub const I64_GE_U: u8 = 0x5A;

    // i32 arithmetic
    pub const I32_CLZ: u8 = 0x67;
    pub const I32_CTZ: u8 = 0x68;
    pub const I32_ADD: u8 = 0x6A;
    pub const I32_SUB: u8 = 0x6B;
    pub const I32_MUL: u8 = 0x6C;
    pub const I32_DIV_S: u8 = 0x6D;
    pub const I32_DIV_U: u8 = 0x6E;
    pub const I32_REM_S: u8 = 0x6F;
    pub const I32_REM_U: u8 = 0x70;
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    pub const I32_SHL: u8 = 0x74;
    pub const I32_SHR_S: u8 = 0x75;
    pub const I32_SHR_U: u8 = 0x76;
    pub const I32_ROTL: u8 = 0x77;
    pub const I32_ROTR: u8 = 0x78;

    // i64 arithmetic
    pub const I64_CLZ: u8 = 0x79;
    pub const I64_CTZ: u8 = 0x7A;
    pub const I64_ADD: u8 = 0x7C;
    pub const I64_SUB: u8 = 0x7D;
    pub const I64_MUL: u8 = 0x7E;
    pub const I64_DIV_S: u8 = 0x7F;
    pub const I64_DIV_U: u8 = 0x80;
    pub const I64_REM_S: u8 = 0x81;
    pub const I64_REM_U: u8 = 0x82;
    pub const I64_AND: u8 = 0x83;
    pub const I64_OR: u8 = 0x84;
    pub const I64_XOR: u8 = 0x85;
    pub const I64_SHL: u8 = 0x86;
    pub const I64_SHR_S: u8 = 0x87;
    pub const I64_SHR_U: u8 = 0x88;
    pub const I64_ROTL: u8 = 0x89;
    pub const I64_ROTR: u8 = 0x8A;

    // Conversions
    pub const I32_WRAP_I64: u8 = 0xA7;
    pub const I64_EXTEND_I32_S: u8 = 0xAC;
    pub const I64_EXTEND_I32_U: u8 = 0xAD;
}

// ── Error Type ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapKind {
    Unreachable,
    StackOverflow,
    StackUnderflow,
    CallStackOverflow,
    OutOfBounds,         // memory access out of bounds
    DivisionByZero,
    IntegerOverflow,
    UnknownOpcode(u8),
    InvalidLocal,
    InvalidFunction,
    TypeMismatch,
    LabelStackOverflow,
    InvalidBranch,
    UnexpectedEnd,
    HostCallError(u32),  // host function returned error
}

// ── Host Call Interface ───────────────────────────────────────

/// Trait for handling calls to imported/host functions.
///
/// Phase 16 will implement this for WASI — mapping fd_write, proc_exit,
/// etc. to GooseOS syscalls via IPC.
pub trait HostFunctions {
    /// Call a host function by index.
    ///
    /// `func_idx` is the import index.
    /// `args` contains the arguments from the WASM stack.
    /// `memory` is the linear memory (for pointer arguments).
    ///
    /// Returns Ok(results) or Err(error_code).
    fn call(
        &mut self,
        func_idx: u32,
        args: &[u64],
        memory: &mut [u8],
    ) -> Result<Option<u64>, u32>;
}

/// No-op host functions (for testing without WASI).
pub struct NoHost;

impl HostFunctions for NoHost {
    fn call(&mut self, func_idx: u32, _args: &[u64], _memory: &mut [u8]) -> Result<Option<u64>, u32> {
        Err(func_idx) // all host calls fail
    }
}

// ── Label (Block/Loop/If control frame) ───────────────────────

#[derive(Clone, Copy)]
struct Label {
    /// PC to branch to (for block/if: end, for loop: loop start)
    target_pc: usize,
    /// Stack depth when this label was entered
    stack_depth: usize,
    /// Number of result values this block produces
    result_count: usize,
    /// Is this a loop? (affects branch target)
    is_loop: bool,
}

// ── Call Frame ────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct CallFrame {
    /// Return PC (instruction after the call)
    return_pc: usize,
    /// Function index (for debugging)
    func_idx: u32,
    /// Base index into the locals array
    locals_base: usize,
    /// Number of locals (params + declared locals)
    local_count: usize,
    /// Stack depth before arguments were pushed
    stack_depth: usize,
    /// Number of results this function returns
    result_count: usize,
    /// Base index into the label stack
    label_base: usize,
}

// ── Interpreter ───────────────────────────────────────────────

/// WASM bytecode interpreter.
///
/// Operates on a parsed WasmModule and its raw binary data.
/// All state is on the Interpreter struct — no heap allocation.
pub struct Interpreter<'a> {
    /// The parsed module
    module: &'a WasmModule,
    /// Raw WASM binary (for reading instruction bytes)
    code: &'a [u8],
    /// Program counter (byte offset into raw binary)
    pc: usize,

    /// Operand stack
    stack: [u64; MAX_STACK],
    sp: usize,

    /// Call stack
    frames: [CallFrame; MAX_CALL_DEPTH],
    frame_depth: usize,

    /// Locals storage (shared across all active frames)
    locals: [u64; MAX_LOCALS * MAX_CALL_DEPTH],

    /// Label stack (for block/loop/if nesting)
    labels: [Label; MAX_LABELS],
    label_depth: usize,

    /// Number of host (imported) functions (offset for local func indices)
    num_imports: u32,
}

impl<'a> Interpreter<'a> {
    /// Create a new interpreter for a parsed WASM module.
    ///
    /// `code` must be the same byte slice that was passed to `wasm::parse()`.
    /// `num_imports` is the number of imported functions (shifts local func indices).
    pub fn new(module: &'a WasmModule, code: &'a [u8], num_imports: u32) -> Self {
        Interpreter {
            module,
            code,
            pc: 0,
            stack: [0; MAX_STACK],
            sp: 0,
            frames: [CallFrame {
                return_pc: 0, func_idx: 0, locals_base: 0,
                local_count: 0, stack_depth: 0, result_count: 0,
                label_base: 0,
            }; MAX_CALL_DEPTH],
            frame_depth: 0,
            locals: [0; MAX_LOCALS * MAX_CALL_DEPTH],
            labels: [Label {
                target_pc: 0, stack_depth: 0, result_count: 0, is_loop: false,
            }; MAX_LABELS],
            label_depth: 0,
            num_imports,
        }
    }

    // ── Stack operations ──────────────────────────────────────

    #[inline]
    fn push(&mut self, val: u64) -> Result<(), TrapKind> {
        if self.sp >= MAX_STACK {
            return Err(TrapKind::StackOverflow);
        }
        self.stack[self.sp] = val;
        self.sp += 1;
        Ok(())
    }

    #[inline]
    fn pop(&mut self) -> Result<u64, TrapKind> {
        if self.sp == 0 {
            return Err(TrapKind::StackUnderflow);
        }
        self.sp -= 1;
        Ok(self.stack[self.sp])
    }

    #[inline]
    fn pop_i32(&mut self) -> Result<i32, TrapKind> {
        Ok(self.pop()? as i32)
    }

    #[inline]
    fn pop_u32(&mut self) -> Result<u32, TrapKind> {
        Ok(self.pop()? as u32)
    }

    // ── Byte reading ──────────────────────────────────────────

    #[inline]
    fn read_byte(&mut self) -> Result<u8, TrapKind> {
        if self.pc >= self.code.len() {
            return Err(TrapKind::UnexpectedEnd);
        }
        let b = self.code[self.pc];
        self.pc += 1;
        Ok(b)
    }

    fn read_u32_leb128(&mut self) -> Result<u32, TrapKind> {
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
                return Err(TrapKind::UnexpectedEnd);
            }
        }
    }

    fn read_i32_leb128(&mut self) -> Result<i32, TrapKind> {
        let mut result: i32 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            result |= ((byte & 0x7F) as i32) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                // Sign extend
                if shift < 32 && (byte & 0x40) != 0 {
                    result |= !0i32 << shift;
                }
                return Ok(result);
            }
            if shift >= 35 {
                return Err(TrapKind::UnexpectedEnd);
            }
        }
    }

    fn read_i64_leb128(&mut self) -> Result<i64, TrapKind> {
        let mut result: i64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            result |= ((byte & 0x7F) as i64) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && (byte & 0x40) != 0 {
                    result |= !0i64 << shift;
                }
                return Ok(result);
            }
            if shift >= 70 {
                return Err(TrapKind::UnexpectedEnd);
            }
        }
    }

    // ── Local variables ───────────────────────────────────────

    fn local_get(&self, idx: u32) -> Result<u64, TrapKind> {
        if self.frame_depth == 0 { return Err(TrapKind::InvalidLocal); }
        let frame = &self.frames[self.frame_depth - 1];
        let abs = frame.locals_base + idx as usize;
        if idx as usize >= frame.local_count || abs >= self.locals.len() {
            return Err(TrapKind::InvalidLocal);
        }
        Ok(self.locals[abs])
    }

    fn local_set(&mut self, idx: u32, val: u64) -> Result<(), TrapKind> {
        if self.frame_depth == 0 { return Err(TrapKind::InvalidLocal); }
        let frame = &self.frames[self.frame_depth - 1];
        let abs = frame.locals_base + idx as usize;
        if idx as usize >= frame.local_count || abs >= self.locals.len() {
            return Err(TrapKind::InvalidLocal);
        }
        self.locals[abs] = val;
        Ok(())
    }

    // ── Label stack ───────────────────────────────────────────

    fn push_label(&mut self, label: Label) -> Result<(), TrapKind> {
        if self.label_depth >= MAX_LABELS {
            return Err(TrapKind::LabelStackOverflow);
        }
        self.labels[self.label_depth] = label;
        self.label_depth += 1;
        Ok(())
    }

    fn get_label(&self, depth: u32) -> Result<&Label, TrapKind> {
        if depth as usize >= self.label_depth {
            return Err(TrapKind::InvalidBranch);
        }
        let label_base = if self.frame_depth > 0 {
            self.frames[self.frame_depth - 1].label_base
        } else {
            0
        };
        let idx = self.label_depth - 1 - depth as usize;
        if idx < label_base {
            return Err(TrapKind::InvalidBranch);
        }
        Ok(&self.labels[idx])
    }

    // ── Branch helpers ────────────────────────────────────────

    /// Skip over a block body to find the matching END.
    /// Skips past ELSE — finds the true end of the block.
    /// Handles nested blocks correctly.
    fn skip_to_end(&mut self) -> Result<(), TrapKind> {
        let mut depth: u32 = 1;
        while depth > 0 {
            let byte = self.read_byte()?;
            match byte {
                op::BLOCK | op::LOOP | op::IF => {
                    self.read_byte()?; // block type
                    depth += 1;
                }
                op::END => {
                    depth -= 1;
                }
                // ELSE is just part of the block — skip over it
                // Skip immediates for instructions that have them
                op::BR | op::BR_IF | op::CALL |
                op::LOCAL_GET | op::LOCAL_SET | op::LOCAL_TEE => {
                    self.read_u32_leb128()?;
                }
                op::I32_CONST => { self.read_i32_leb128()?; }
                op::I64_CONST => { self.read_i64_leb128()?; }
                op::I32_LOAD | op::I64_LOAD |
                op::I32_LOAD8_S | op::I32_LOAD8_U |
                op::I32_LOAD16_S | op::I32_LOAD16_U |
                op::I32_STORE | op::I64_STORE |
                op::I32_STORE8 | op::I32_STORE16 => {
                    self.read_u32_leb128()?; // align
                    self.read_u32_leb128()?; // offset
                }
                op::MEMORY_SIZE | op::MEMORY_GROW => {
                    self.read_byte()?; // memory index (always 0)
                }
                _ => {} // no immediates
            }
        }
        Ok(())
    }

    /// Skip to the matching else or end for an if-false branch.
    fn skip_to_else_or_end(&mut self) -> Result<bool, TrapKind> {
        let mut depth: u32 = 1;
        while depth > 0 {
            let byte = self.read_byte()?;
            match byte {
                op::BLOCK | op::LOOP | op::IF => {
                    self.read_byte()?;
                    depth += 1;
                }
                op::END => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(false); // no else found, hit end
                    }
                }
                op::ELSE => {
                    if depth == 1 {
                        return Ok(true); // found else at our level
                    }
                }
                op::BR | op::BR_IF | op::CALL |
                op::LOCAL_GET | op::LOCAL_SET | op::LOCAL_TEE => {
                    self.read_u32_leb128()?;
                }
                op::I32_CONST => { self.read_i32_leb128()?; }
                op::I64_CONST => { self.read_i64_leb128()?; }
                op::I32_LOAD | op::I64_LOAD |
                op::I32_LOAD8_S | op::I32_LOAD8_U |
                op::I32_LOAD16_S | op::I32_LOAD16_U |
                op::I32_STORE | op::I64_STORE |
                op::I32_STORE8 | op::I32_STORE16 => {
                    self.read_u32_leb128()?;
                    self.read_u32_leb128()?;
                }
                op::MEMORY_SIZE | op::MEMORY_GROW => {
                    self.read_byte()?;
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Execute a branch to label at depth `depth`.
    fn do_branch(&mut self, depth: u32) -> Result<(), TrapKind> {
        let label = *self.get_label(depth)?;

        if label.is_loop {
            // Branch to loop header — unwind stack to label's depth, jump back
            self.sp = label.stack_depth;
            self.pc = label.target_pc;
            // Don't pop the label — loops keep their label
        } else {
            // Branch to block end — unwind stack, keep result values
            let results = if label.result_count > 0 && self.sp > 0 {
                Some(self.stack[self.sp - 1])
            } else {
                None
            };

            self.sp = label.stack_depth;

            // Pop labels down to (and including) the target
            let pop_count = depth as usize + 1;
            if self.label_depth >= pop_count {
                self.label_depth -= pop_count;
            }

            // Push back result value
            if let Some(val) = results {
                if label.result_count > 0 {
                    self.push(val)?;
                }
            }

            self.pc = label.target_pc;
        }
        Ok(())
    }

    // ── Memory helpers ────────────────────────────────────────

    fn mem_load_u8(&self, memory: &[u8], addr: u32, offset: u32) -> Result<u8, TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea >= memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        Ok(memory[ea])
    }

    fn mem_load_u16(&self, memory: &[u8], addr: u32, offset: u32) -> Result<u16, TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea + 2 > memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        Ok(u16::from_le_bytes([memory[ea], memory[ea + 1]]))
    }

    fn mem_load_u32(&self, memory: &[u8], addr: u32, offset: u32) -> Result<u32, TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea + 4 > memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        Ok(u32::from_le_bytes([
            memory[ea], memory[ea + 1], memory[ea + 2], memory[ea + 3],
        ]))
    }

    fn mem_load_u64(&self, memory: &[u8], addr: u32, offset: u32) -> Result<u64, TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea + 8 > memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        Ok(u64::from_le_bytes([
            memory[ea], memory[ea + 1], memory[ea + 2], memory[ea + 3],
            memory[ea + 4], memory[ea + 5], memory[ea + 6], memory[ea + 7],
        ]))
    }

    fn mem_store_u8(&self, memory: &mut [u8], addr: u32, offset: u32, val: u8) -> Result<(), TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea >= memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        memory[ea] = val;
        Ok(())
    }

    fn mem_store_u16(&self, memory: &mut [u8], addr: u32, offset: u32, val: u16) -> Result<(), TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea + 2 > memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        let bytes = val.to_le_bytes();
        memory[ea] = bytes[0];
        memory[ea + 1] = bytes[1];
        Ok(())
    }

    fn mem_store_u32(&self, memory: &mut [u8], addr: u32, offset: u32, val: u32) -> Result<(), TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea + 4 > memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        let bytes = val.to_le_bytes();
        memory[ea] = bytes[0];
        memory[ea + 1] = bytes[1];
        memory[ea + 2] = bytes[2];
        memory[ea + 3] = bytes[3];
        Ok(())
    }

    fn mem_store_u64(&self, memory: &mut [u8], addr: u32, offset: u32, val: u64) -> Result<(), TrapKind> {
        let ea = addr as usize + offset as usize;
        if ea + 8 > memory.len() {
            return Err(TrapKind::OutOfBounds);
        }
        let bytes = val.to_le_bytes();
        let mut i = 0;
        while i < 8 {
            memory[ea + i] = bytes[i];
            i += 1;
        }
        Ok(())
    }

    // ── Function entry ────────────────────────────────────────

    /// Decode locals from a function body and set up the call frame.
    fn enter_function(&mut self, func_idx: u32, return_pc: usize) -> Result<(), TrapKind> {
        if self.frame_depth >= MAX_CALL_DEPTH {
            return Err(TrapKind::CallStackOverflow);
        }

        let local_func_idx = if func_idx >= self.num_imports {
            (func_idx - self.num_imports) as usize
        } else {
            return Err(TrapKind::InvalidFunction);
        };

        if local_func_idx >= self.module.func_count {
            return Err(TrapKind::InvalidFunction);
        }

        // Get function type
        let type_idx = self.module.func_types[local_func_idx] as usize;
        if type_idx >= self.module.type_count {
            return Err(TrapKind::TypeMismatch);
        }
        let func_type = &self.module.types[type_idx];
        let param_count = func_type.param_count;
        let result_count = func_type.result_count;

        // Get body
        let body = &self.module.bodies[local_func_idx];
        self.pc = body.offset;

        // Decode local declarations
        let num_local_groups = self.read_u32_leb128()? as usize;
        let mut declared_locals: usize = 0;
        // We need to read the local declarations to advance PC,
        // and to know total local count
        let mut local_types: [(u32, u8); 16] = [(0, 0); 16];
        let groups = if num_local_groups <= 16 { num_local_groups } else { 16 };
        for i in 0..num_local_groups {
            let count = self.read_u32_leb128()?;
            let _vtype = self.read_byte()?;
            if i < 16 {
                local_types[i] = (count, _vtype);
            }
            declared_locals += count as usize;
        }

        let total_locals = param_count + declared_locals;
        if total_locals > MAX_LOCALS {
            return Err(TrapKind::InvalidLocal);
        }

        // Compute locals base for this frame
        let locals_base = if self.frame_depth > 0 {
            let prev = &self.frames[self.frame_depth - 1];
            prev.locals_base + prev.local_count
        } else {
            0
        };

        if locals_base + total_locals > self.locals.len() {
            return Err(TrapKind::InvalidLocal);
        }

        // Pop arguments from stack into locals (in order)
        let stack_depth_before_args = if self.sp >= param_count {
            self.sp - param_count
        } else {
            return Err(TrapKind::StackUnderflow);
        };

        for i in 0..param_count {
            self.locals[locals_base + i] = self.stack[stack_depth_before_args + i];
        }
        self.sp = stack_depth_before_args;

        // Zero declared locals
        for i in param_count..total_locals {
            self.locals[locals_base + i] = 0;
        }

        // Push call frame
        self.frames[self.frame_depth] = CallFrame {
            return_pc: return_pc,
            func_idx,
            locals_base,
            local_count: total_locals,
            stack_depth: self.sp,
            result_count,
            label_base: self.label_depth,
        };
        self.frame_depth += 1;

        // Push implicit function-body label
        // For the function body, target_pc doesn't matter for forward branches —
        // return will use return_pc from the call frame.
        // We need to find the end of this function body for the label target.
        let body_end = body.offset + body.length;
        self.push_label(Label {
            target_pc: body_end,
            stack_depth: self.sp,
            result_count,
            is_loop: false,
        })?;

        Ok(())
    }

    // ── Main execution loop ───────────────────────────────────

    /// Execute a function by index, with the given arguments.
    ///
    /// Returns the result value (if any) or a trap.
    /// `memory` is the linear memory for load/store instructions.
    /// `host` handles calls to imported functions.
    pub fn call_function<H: HostFunctions>(
        &mut self,
        func_idx: u32,
        args: &[u64],
        memory: &mut [u8],
        host: &mut H,
    ) -> Result<Option<u64>, TrapKind> {
        // Push arguments onto the stack
        for &arg in args {
            self.push(arg)?;
        }

        // Enter the function
        self.enter_function(func_idx, 0)?;

        // Execute until we return from this function
        self.run(memory, host)
    }

    /// Main interpreter loop.
    fn run<H: HostFunctions>(
        &mut self,
        memory: &mut [u8],
        host: &mut H,
    ) -> Result<Option<u64>, TrapKind> {
        loop {
            let opcode = self.read_byte()?;

            match opcode {
                // ── Control ───────────────────────────────────

                op::UNREACHABLE => return Err(TrapKind::Unreachable),

                op::NOP => {}

                op::BLOCK => {
                    let block_type = self.read_byte()?;
                    let result_count = if block_type == 0x40 { 0 } else { 1 };
                    // Find the end of this block (for branch target)
                    let save_pc = self.pc;
                    self.skip_to_end()?;
                    let end_pc = self.pc;
                    self.pc = save_pc;

                    self.push_label(Label {
                        target_pc: end_pc,
                        stack_depth: self.sp,
                        result_count,
                        is_loop: false,
                    })?;
                }

                op::LOOP => {
                    let _block_type = self.read_byte()?;
                    let loop_pc = self.pc; // branch target = loop start

                    self.push_label(Label {
                        target_pc: loop_pc,
                        stack_depth: self.sp,
                        result_count: 0, // loops don't produce values on branch
                        is_loop: true,
                    })?;
                }

                op::IF => {
                    let block_type = self.read_byte()?;
                    let result_count = if block_type == 0x40 { 0 } else { 1 };
                    let cond = self.pop_u32()?;

                    // Find end for the label
                    let save_pc = self.pc;
                    self.skip_to_end()?;
                    let end_pc = self.pc;
                    self.pc = save_pc;

                    if cond != 0 {
                        // True branch — execute body
                        self.push_label(Label {
                            target_pc: end_pc,
                            stack_depth: self.sp,
                            result_count,
                            is_loop: false,
                        })?;
                    } else {
                        // False branch — skip to else or end
                        let has_else = self.skip_to_else_or_end()?;
                        if has_else {
                            self.push_label(Label {
                                target_pc: end_pc,
                                stack_depth: self.sp,
                                result_count,
                                is_loop: false,
                            })?;
                        }
                        // If no else, we're past the end — no label needed
                    }
                }

                op::ELSE => {
                    // We're in the true branch and hit else — skip the else body.
                    // The label's target_pc points past the end. We skip there
                    // and pop the label now (true branch is done, result is on stack).
                    if self.label_depth > 0 {
                        let label = self.labels[self.label_depth - 1];

                        // Preserve result value if this block produces one
                        let result = if label.result_count > 0 && self.sp > label.stack_depth {
                            Some(self.stack[self.sp - 1])
                        } else {
                            None
                        };

                        self.sp = label.stack_depth;
                        if let Some(val) = result {
                            self.push(val).ok();
                        }

                        self.label_depth -= 1;
                        self.pc = label.target_pc;
                    }
                }

                op::END => {
                    if self.label_depth > 0 {
                        let frame_label_base = if self.frame_depth > 0 {
                            self.frames[self.frame_depth - 1].label_base
                        } else {
                            0
                        };

                        if self.label_depth - 1 == frame_label_base {
                            // This is the function body's end — return
                            let frame = self.frames[self.frame_depth - 1];
                            let result = if frame.result_count > 0 && self.sp > 0 {
                                Some(self.stack[self.sp - 1])
                            } else {
                                None
                            };

                            // Restore stack
                            self.sp = frame.stack_depth;
                            if let Some(val) = result {
                                self.push(val)?;
                            }

                            // Pop call frame
                            self.label_depth = frame.label_base;
                            self.frame_depth -= 1;

                            if self.frame_depth == 0 {
                                // Returned from top-level call
                                return Ok(result);
                            }

                            // Return to caller
                            self.pc = frame.return_pc;
                        } else {
                            // End of a block/loop/if — pop label
                            let label = self.labels[self.label_depth - 1];
                            // Keep result values on stack
                            if !label.is_loop && label.result_count > 0 && self.sp > label.stack_depth {
                                let result = self.stack[self.sp - 1];
                                self.sp = label.stack_depth;
                                self.push(result)?;
                            } else if self.sp > label.stack_depth + label.result_count {
                                self.sp = label.stack_depth;
                            }
                            self.label_depth -= 1;
                        }
                    } else {
                        // End with no labels — shouldn't happen in valid WASM
                        return Ok(if self.sp > 0 { Some(self.stack[self.sp - 1]) } else { None });
                    }
                }

                op::BR => {
                    let depth = self.read_u32_leb128()?;
                    self.do_branch(depth)?;
                }

                op::BR_IF => {
                    let depth = self.read_u32_leb128()?;
                    let cond = self.pop_u32()?;
                    if cond != 0 {
                        self.do_branch(depth)?;
                    }
                }

                op::RETURN => {
                    if self.frame_depth == 0 {
                        return Ok(if self.sp > 0 { Some(self.stack[self.sp - 1]) } else { None });
                    }

                    let frame = self.frames[self.frame_depth - 1];
                    let result = if frame.result_count > 0 && self.sp > 0 {
                        Some(self.stack[self.sp - 1])
                    } else {
                        None
                    };

                    self.sp = frame.stack_depth;
                    if let Some(val) = result {
                        self.push(val)?;
                    }

                    self.label_depth = frame.label_base;
                    self.frame_depth -= 1;

                    if self.frame_depth == 0 {
                        return Ok(result);
                    }

                    self.pc = frame.return_pc;
                }

                op::CALL => {
                    let target = self.read_u32_leb128()?;

                    if target < self.num_imports {
                        // Host function call — type comes from import_types, not func_types
                        let type_idx = self.module.import_types[target as usize] as usize;
                        let func_type = &self.module.types[type_idx];
                        let param_count = func_type.param_count;

                        // Gather args from stack
                        let mut args = [0u64; 8];
                        if self.sp < param_count {
                            return Err(TrapKind::StackUnderflow);
                        }
                        for i in 0..param_count {
                            args[param_count - 1 - i] = self.pop()?;
                        }

                        let result = host.call(target, &args[..param_count], memory)
                            .map_err(TrapKind::HostCallError)?;

                        if let Some(val) = result {
                            self.push(val)?;
                        }
                    } else {
                        // Local function call
                        let return_pc = self.pc;
                        self.enter_function(target, return_pc)?;
                    }
                }

                // ── Parametric ────────────────────────────────

                op::DROP => { self.pop()?; }

                op::SELECT => {
                    let cond = self.pop_u32()?;
                    let val2 = self.pop()?;
                    let val1 = self.pop()?;
                    self.push(if cond != 0 { val1 } else { val2 })?;
                }

                // ── Variables ─────────────────────────────────

                op::LOCAL_GET => {
                    let idx = self.read_u32_leb128()?;
                    let val = self.local_get(idx)?;
                    self.push(val)?;
                }

                op::LOCAL_SET => {
                    let idx = self.read_u32_leb128()?;
                    let val = self.pop()?;
                    self.local_set(idx, val)?;
                }

                op::LOCAL_TEE => {
                    let idx = self.read_u32_leb128()?;
                    let val = if self.sp > 0 { self.stack[self.sp - 1] } else {
                        return Err(TrapKind::StackUnderflow);
                    };
                    self.local_set(idx, val)?;
                }

                // ── Memory ────────────────────────────────────

                op::I32_LOAD => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let addr = self.pop_u32()?;
                    let val = self.mem_load_u32(memory, addr, offset)?;
                    self.push(val as u64)?;
                }

                op::I64_LOAD => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let addr = self.pop_u32()?;
                    let val = self.mem_load_u64(memory, addr, offset)?;
                    self.push(val)?;
                }

                op::I32_LOAD8_S => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let addr = self.pop_u32()?;
                    let val = self.mem_load_u8(memory, addr, offset)? as i8 as i32;
                    self.push(val as u64)?;
                }

                op::I32_LOAD8_U => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let addr = self.pop_u32()?;
                    let val = self.mem_load_u8(memory, addr, offset)?;
                    self.push(val as u64)?;
                }

                op::I32_LOAD16_S => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let addr = self.pop_u32()?;
                    let val = self.mem_load_u16(memory, addr, offset)? as i16 as i32;
                    self.push(val as u64)?;
                }

                op::I32_LOAD16_U => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let addr = self.pop_u32()?;
                    let val = self.mem_load_u16(memory, addr, offset)?;
                    self.push(val as u64)?;
                }

                op::I32_STORE => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let val = self.pop_u32()?;
                    let addr = self.pop_u32()?;
                    self.mem_store_u32(memory, addr, offset, val)?;
                }

                op::I64_STORE => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let val = self.pop()?;
                    let addr = self.pop_u32()?;
                    self.mem_store_u64(memory, addr, offset, val)?;
                }

                op::I32_STORE8 => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let val = self.pop_u32()? as u8;
                    let addr = self.pop_u32()?;
                    self.mem_store_u8(memory, addr, offset, val)?;
                }

                op::I32_STORE16 => {
                    let _align = self.read_u32_leb128()?;
                    let offset = self.read_u32_leb128()?;
                    let val = self.pop_u32()? as u16;
                    let addr = self.pop_u32()?;
                    self.mem_store_u16(memory, addr, offset, val)?;
                }

                op::MEMORY_SIZE => {
                    let _mem_idx = self.read_byte()?;
                    let pages = (memory.len() / 65536) as u32;
                    self.push(pages as u64)?;
                }

                op::MEMORY_GROW => {
                    let _mem_idx = self.read_byte()?;
                    let _delta = self.pop_u32()?;
                    // Memory growth not supported yet — return -1
                    self.push(0xFFFF_FFFF_u64)?;
                }

                // ── Constants ─────────────────────────────────

                op::I32_CONST => {
                    let val = self.read_i32_leb128()?;
                    self.push(val as u32 as u64)?;
                }

                op::I64_CONST => {
                    let val = self.read_i64_leb128()?;
                    self.push(val as u64)?;
                }

                // ── i32 comparison ────────────────────────────

                op::I32_EQZ => {
                    let a = self.pop_u32()?;
                    self.push(if a == 0 { 1 } else { 0 })?;
                }
                op::I32_EQ => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(if a == b { 1 } else { 0 })?;
                }
                op::I32_NE => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(if a != b { 1 } else { 0 })?;
                }
                op::I32_LT_S => {
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    self.push(if a < b { 1 } else { 0 })?;
                }
                op::I32_LT_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(if a < b { 1 } else { 0 })?;
                }
                op::I32_GT_S => {
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    self.push(if a > b { 1 } else { 0 })?;
                }
                op::I32_GT_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(if a > b { 1 } else { 0 })?;
                }
                op::I32_LE_S => {
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    self.push(if a <= b { 1 } else { 0 })?;
                }
                op::I32_LE_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(if a <= b { 1 } else { 0 })?;
                }
                op::I32_GE_S => {
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    self.push(if a >= b { 1 } else { 0 })?;
                }
                op::I32_GE_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(if a >= b { 1 } else { 0 })?;
                }

                // ── i64 comparison ────────────────────────────

                op::I64_EQZ => {
                    let a = self.pop()?;
                    self.push(if a == 0 { 1 } else { 0 })?;
                }
                op::I64_EQ => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(if a == b { 1 } else { 0 })?;
                }
                op::I64_NE => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(if a != b { 1 } else { 0 })?;
                }
                op::I64_LT_S => {
                    let b = self.pop()? as i64;
                    let a = self.pop()? as i64;
                    self.push(if a < b { 1 } else { 0 })?;
                }
                op::I64_LT_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(if a < b { 1 } else { 0 })?;
                }
                op::I64_GT_S => {
                    let b = self.pop()? as i64;
                    let a = self.pop()? as i64;
                    self.push(if a > b { 1 } else { 0 })?;
                }
                op::I64_GT_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(if a > b { 1 } else { 0 })?;
                }
                op::I64_LE_S => {
                    let b = self.pop()? as i64;
                    let a = self.pop()? as i64;
                    self.push(if a <= b { 1 } else { 0 })?;
                }
                op::I64_LE_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(if a <= b { 1 } else { 0 })?;
                }
                op::I64_GE_S => {
                    let b = self.pop()? as i64;
                    let a = self.pop()? as i64;
                    self.push(if a >= b { 1 } else { 0 })?;
                }
                op::I64_GE_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(if a >= b { 1 } else { 0 })?;
                }

                // ── i32 arithmetic ────────────────────────────

                op::I32_CLZ => {
                    let a = self.pop_u32()?;
                    self.push(a.leading_zeros() as u64)?;
                }
                op::I32_CTZ => {
                    let a = self.pop_u32()?;
                    self.push(a.trailing_zeros() as u64)?;
                }
                op::I32_ADD => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.wrapping_add(b) as u64)?;
                }
                op::I32_SUB => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.wrapping_sub(b) as u64)?;
                }
                op::I32_MUL => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.wrapping_mul(b) as u64)?;
                }
                op::I32_DIV_S => {
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    if a == i32::MIN && b == -1 { return Err(TrapKind::IntegerOverflow); }
                    self.push(a.wrapping_div(b) as u32 as u64)?;
                }
                op::I32_DIV_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    self.push((a / b) as u64)?;
                }
                op::I32_REM_S => {
                    let b = self.pop_i32()?;
                    let a = self.pop_i32()?;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    self.push(if a == i32::MIN && b == -1 { 0u64 } else { (a % b) as u32 as u64 })?;
                }
                op::I32_REM_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    self.push((a % b) as u64)?;
                }
                op::I32_AND => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push((a & b) as u64)?;
                }
                op::I32_OR => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push((a | b) as u64)?;
                }
                op::I32_XOR => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push((a ^ b) as u64)?;
                }
                op::I32_SHL => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.wrapping_shl(b & 31) as u64)?;
                }
                op::I32_SHR_S => {
                    let b = self.pop_u32()?;
                    let a = self.pop_i32()?;
                    self.push((a.wrapping_shr(b & 31)) as u32 as u64)?;
                }
                op::I32_SHR_U => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.wrapping_shr(b & 31) as u64)?;
                }
                op::I32_ROTL => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.rotate_left(b & 31) as u64)?;
                }
                op::I32_ROTR => {
                    let b = self.pop_u32()?;
                    let a = self.pop_u32()?;
                    self.push(a.rotate_right(b & 31) as u64)?;
                }

                // ── i64 arithmetic ────────────────────────────

                op::I64_CLZ => {
                    let a = self.pop()?;
                    self.push(a.leading_zeros() as u64)?;
                }
                op::I64_CTZ => {
                    let a = self.pop()?;
                    self.push(a.trailing_zeros() as u64)?;
                }
                op::I64_ADD => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.wrapping_add(b))?;
                }
                op::I64_SUB => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.wrapping_sub(b))?;
                }
                op::I64_MUL => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.wrapping_mul(b))?;
                }
                op::I64_DIV_S => {
                    let b = self.pop()? as i64;
                    let a = self.pop()? as i64;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    if a == i64::MIN && b == -1 { return Err(TrapKind::IntegerOverflow); }
                    self.push(a.wrapping_div(b) as u64)?;
                }
                op::I64_DIV_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    self.push(a / b)?;
                }
                op::I64_REM_S => {
                    let b = self.pop()? as i64;
                    let a = self.pop()? as i64;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    self.push(if a == i64::MIN && b == -1 { 0 } else { (a % b) as u64 })?;
                }
                op::I64_REM_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if b == 0 { return Err(TrapKind::DivisionByZero); }
                    self.push(a % b)?;
                }
                op::I64_AND => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a & b)?;
                }
                op::I64_OR => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a | b)?;
                }
                op::I64_XOR => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a ^ b)?;
                }
                op::I64_SHL => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.wrapping_shl((b & 63) as u32))?;
                }
                op::I64_SHR_S => {
                    let b = self.pop()?;
                    let a = self.pop()? as i64;
                    self.push(a.wrapping_shr((b & 63) as u32) as u64)?;
                }
                op::I64_SHR_U => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.wrapping_shr((b & 63) as u32))?;
                }
                op::I64_ROTL => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.rotate_left((b & 63) as u32))?;
                }
                op::I64_ROTR => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(a.rotate_right((b & 63) as u32))?;
                }

                // ── Conversions ───────────────────────────────

                op::I32_WRAP_I64 => {
                    let a = self.pop()?;
                    self.push(a as u32 as u64)?;
                }
                op::I64_EXTEND_I32_S => {
                    let a = self.pop()? as i32;
                    self.push(a as i64 as u64)?;
                }
                op::I64_EXTEND_I32_U => {
                    let a = self.pop()? as u32;
                    self.push(a as u64)?;
                }

                _ => return Err(TrapKind::UnknownOpcode(opcode)),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm;

    /// Helper: build a minimal WASM module with one function and execute it.
    fn run_wasm(body: &[u8]) -> Result<Option<u64>, TrapKind> {
        run_wasm_with_type(body, 0, 1) // () -> i32 by default
    }

    /// Helper: build a module with specified param/result counts.
    fn run_wasm_with_type(body: &[u8], params: u8, results: u8) -> Result<Option<u64>, TrapKind> {
        // Build a WASM module: type section + function section + code section
        let mut wasm_bytes: Vec<u8> = Vec::new();

        // Header
        wasm_bytes.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);

        // Type section
        let mut type_sec = vec![0x01]; // 1 type
        type_sec.push(0x60); // functype
        // Params
        type_sec.push(params);
        for _ in 0..params {
            type_sec.push(0x7F); // i32
        }
        // Results
        type_sec.push(results);
        for _ in 0..results {
            type_sec.push(0x7F); // i32
        }
        wasm_bytes.push(0x01); // type section id
        push_leb128(&mut wasm_bytes, type_sec.len() as u32);
        wasm_bytes.extend_from_slice(&type_sec);

        // Function section
        wasm_bytes.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]); // 1 func, type 0

        // Code section
        let mut code_sec = vec![0x01]; // 1 body
        // Body: size + locals + code
        let body_size = 1 + body.len(); // 1 byte for "0 local groups" + body
        push_leb128(&mut code_sec, body_size as u32);
        code_sec.push(0x00); // 0 local declarations
        code_sec.extend_from_slice(body);
        wasm_bytes.push(0x0A); // code section id
        push_leb128(&mut wasm_bytes, code_sec.len() as u32);
        wasm_bytes.extend_from_slice(&code_sec);

        // Parse
        let module = wasm::parse(&wasm_bytes).expect("parse failed");
        let mut interp = Interpreter::new(&module, &wasm_bytes, 0);
        let mut memory = [0u8; 256];
        let mut host = NoHost;

        interp.call_function(0, &[], &mut memory, &mut host)
    }

    fn push_leb128(buf: &mut Vec<u8>, mut val: u32) {
        loop {
            let mut byte = (val & 0x7F) as u8;
            val >>= 7;
            if val != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if val == 0 { break; }
        }
    }

    // ── Basic tests ───────────────────────────────────────────

    #[test]
    fn test_i32_const() {
        // i32.const 42, end
        let result = run_wasm(&[0x41, 0x2A, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_i32_add() {
        // i32.const 10, i32.const 32, i32.add, end
        let result = run_wasm(&[0x41, 0x0A, 0x41, 0x20, 0x6A, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_i32_sub() {
        // i32.const 50, i32.const 8, i32.sub, end
        let result = run_wasm(&[0x41, 0x32, 0x41, 0x08, 0x6B, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_i32_mul() {
        // i32.const 6, i32.const 7, i32.mul, end
        let result = run_wasm(&[0x41, 0x06, 0x41, 0x07, 0x6C, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_i32_div_s() {
        // i32.const 84, i32.const 2, i32.div_s, end
        let result = run_wasm(&[0x41, 0xD4, 0x00, 0x41, 0x02, 0x6D, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_div_by_zero() {
        // i32.const 1, i32.const 0, i32.div_u, end
        let result = run_wasm(&[0x41, 0x01, 0x41, 0x00, 0x6E, 0x0B]);
        assert_eq!(result, Err(TrapKind::DivisionByZero));
    }

    #[test]
    fn test_i32_and_or_xor() {
        // i32.const 0xFF, i32.const 0x0F, i32.and → 0x0F
        let result = run_wasm(&[0x41, 0xFF, 0x01, 0x41, 0x0F, 0x71, 0x0B]).unwrap();
        assert_eq!(result, Some(0x0F));
    }

    #[test]
    fn test_i32_eqz() {
        // i32.const 0, i32.eqz → 1
        let result = run_wasm(&[0x41, 0x00, 0x45, 0x0B]).unwrap();
        assert_eq!(result, Some(1));
        // i32.const 5, i32.eqz → 0
        let result = run_wasm(&[0x41, 0x05, 0x45, 0x0B]).unwrap();
        assert_eq!(result, Some(0));
    }

    #[test]
    fn test_i32_lt_s() {
        // i32.const -1, i32.const 1, i32.lt_s → 1
        let result = run_wasm(&[0x41, 0x7F, 0x41, 0x01, 0x48, 0x0B]).unwrap();
        assert_eq!(result, Some(1));
    }

    // ── Control flow tests ────────────────────────────────────

    #[test]
    fn test_block() {
        // block(i32) { i32.const 42 } end → 42
        let result = run_wasm(&[0x02, 0x7F, 0x41, 0x2A, 0x0B, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_if_true() {
        // i32.const 1, if(i32) { i32.const 42 } else { i32.const 0 } end
        let result = run_wasm(&[
            0x41, 0x01,       // i32.const 1
            0x04, 0x7F,       // if (result i32)
            0x41, 0x2A,       // i32.const 42
            0x05,             // else
            0x41, 0x00,       // i32.const 0
            0x0B,             // end (if)
            0x0B,             // end (func)
        ]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_if_false() {
        // i32.const 0, if(i32) { i32.const 42 } else { i32.const 99 } end
        let result = run_wasm(&[
            0x41, 0x00,       // i32.const 0
            0x04, 0x7F,       // if (result i32)
            0x41, 0x2A,       // i32.const 42
            0x05,             // else
            0x41, 0xE3, 0x00, // i32.const 99
            0x0B,             // end (if)
            0x0B,             // end (func)
        ]).unwrap();
        assert_eq!(result, Some(99));
    }

    #[test]
    fn test_br_if() {
        // block(i32) { i32.const 42, i32.const 1, br_if 0, drop, i32.const 0 } end
        let result = run_wasm(&[
            0x02, 0x7F,       // block (result i32)
            0x41, 0x2A,       // i32.const 42
            0x41, 0x01,       // i32.const 1
            0x0D, 0x00,       // br_if 0
            0x1A,             // drop
            0x41, 0x00,       // i32.const 0
            0x0B,             // end
            0x0B,             // end
        ]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_loop_counter() {
        // Count from 0 to 10 using a loop:
        //   local i: i32
        //   loop {
        //     local.get 0
        //     i32.const 1
        //     i32.add
        //     local.tee 0
        //     i32.const 10
        //     i32.lt_u
        //     br_if 0
        //   }
        //   local.get 0
        //   end

        // Build manually with 1 local
        let mut wasm_bytes: Vec<u8> = Vec::new();
        wasm_bytes.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
        // Type: () -> i32
        wasm_bytes.extend_from_slice(&[0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7F]);
        // Function
        wasm_bytes.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
        // Code section
        let body: &[u8] = &[
            0x01,             // 1 local group
            0x01, 0x7F,       // 1 local of type i32
            0x03, 0x40,       // loop (void)
            0x20, 0x00,       // local.get 0
            0x41, 0x01,       // i32.const 1
            0x6A,             // i32.add
            0x22, 0x00,       // local.tee 0
            0x41, 0x0A,       // i32.const 10
            0x49,             // i32.lt_u
            0x0D, 0x00,       // br_if 0
            0x0B,             // end (loop)
            0x20, 0x00,       // local.get 0
            0x0B,             // end (func)
        ];
        let body_len = body.len();
        wasm_bytes.extend_from_slice(&[0x0A]);
        push_leb128(&mut wasm_bytes, (1 + body_len + 1) as u32); // code sec size
        wasm_bytes.push(0x01); // 1 body
        push_leb128(&mut wasm_bytes, body_len as u32); // body size
        wasm_bytes.extend_from_slice(body);

        let module = wasm::parse(&wasm_bytes).expect("parse");
        let mut interp = Interpreter::new(&module, &wasm_bytes, 0);
        let mut memory = [0u8; 256];
        let mut host = NoHost;

        let result = interp.call_function(0, &[], &mut memory, &mut host).unwrap();
        assert_eq!(result, Some(10));
    }

    // ── Memory tests ──────────────────────────────────────────

    #[test]
    fn test_i32_store_load() {
        // i32.const 0 (addr), i32.const 42, i32.store, i32.const 0, i32.load, end
        let result = run_wasm(&[
            0x41, 0x00,             // i32.const 0 (addr)
            0x41, 0x2A,             // i32.const 42
            0x36, 0x02, 0x00,       // i32.store align=2 offset=0
            0x41, 0x00,             // i32.const 0 (addr)
            0x28, 0x02, 0x00,       // i32.load align=2 offset=0
            0x0B,                   // end
        ]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_i32_store8_load8() {
        // Store 0xFF at addr 10, load back as unsigned
        let result = run_wasm(&[
            0x41, 0x0A,             // i32.const 10 (addr)
            0x41, 0xFF, 0x01,       // i32.const 255
            0x38, 0x00, 0x00,       // i32.store8 align=0 offset=0
            0x41, 0x0A,             // i32.const 10 (addr)
            0x2D, 0x00, 0x00,       // i32.load8_u align=0 offset=0
            0x0B,                   // end
        ]).unwrap();
        assert_eq!(result, Some(255));
    }

    #[test]
    fn test_memory_out_of_bounds() {
        // i32.const 300 (beyond 256-byte memory), i32.load → OOB
        let result = run_wasm(&[
            0x41, 0xAC, 0x02,       // i32.const 300
            0x28, 0x02, 0x00,       // i32.load
            0x0B,
        ]);
        assert_eq!(result, Err(TrapKind::OutOfBounds));
    }

    // ── Locals tests ──────────────────────────────────────────

    #[test]
    fn test_local_get_set() {
        // Build with 1 param (i32) → i32
        // local.get 0, i32.const 10, i32.add, end
        let mut wasm_bytes: Vec<u8> = Vec::new();
        wasm_bytes.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
        // Type: (i32) -> i32
        wasm_bytes.extend_from_slice(&[0x01, 0x06, 0x01, 0x60, 0x01, 0x7F, 0x01, 0x7F]);
        // Function
        wasm_bytes.extend_from_slice(&[0x03, 0x02, 0x01, 0x00]);
        // Code: local.get 0, i32.const 10, i32.add, end
        let body: &[u8] = &[
            0x00,             // 0 local declarations
            0x20, 0x00,       // local.get 0 (param)
            0x41, 0x0A,       // i32.const 10
            0x6A,             // i32.add
            0x0B,             // end
        ];
        wasm_bytes.extend_from_slice(&[0x0A]);
        push_leb128(&mut wasm_bytes, (2 + body.len()) as u32);
        wasm_bytes.push(0x01); // 1 body
        push_leb128(&mut wasm_bytes, body.len() as u32);
        wasm_bytes.extend_from_slice(body);

        let module = wasm::parse(&wasm_bytes).expect("parse");
        let mut interp = Interpreter::new(&module, &wasm_bytes, 0);
        let mut memory = [0u8; 256];
        let mut host = NoHost;

        let result = interp.call_function(0, &[32], &mut memory, &mut host).unwrap();
        assert_eq!(result, Some(42));
    }

    // ── Select/Drop tests ─────────────────────────────────────

    #[test]
    fn test_select() {
        // i32.const 42, i32.const 99, i32.const 1, select → 42
        let result = run_wasm(&[
            0x41, 0x2A,       // i32.const 42
            0x41, 0x63,       // i32.const 99
            0x41, 0x01,       // i32.const 1 (cond: true)
            0x1B,             // select
            0x0B,
        ]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_nop() {
        let result = run_wasm(&[0x01, 0x41, 0x2A, 0x0B]).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_unreachable() {
        let result = run_wasm(&[0x00, 0x0B]);
        assert_eq!(result, Err(TrapKind::Unreachable));
    }

    // ── Function call test ────────────────────────────────────

    #[test]
    fn test_call_function() {
        // Module with 2 functions:
        //   func 0: () -> i32 = call func 1 with args (20, 22)
        //   func 1: (i32, i32) -> i32 = param0 + param1
        let mut wasm_bytes: Vec<u8> = Vec::new();
        wasm_bytes.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);

        // Type section: 2 types
        // type 0: () -> i32
        // type 1: (i32, i32) -> i32
        wasm_bytes.extend_from_slice(&[
            0x01, 0x0B,       // type section, 11 bytes
            0x02,             // 2 types
            0x60, 0x00, 0x01, 0x7F,             // () -> i32
            0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F, // (i32, i32) -> i32
        ]);

        // Function section: 2 functions
        wasm_bytes.extend_from_slice(&[
            0x03, 0x03,       // func section, 3 bytes
            0x02, 0x00, 0x01, // func 0 = type 0, func 1 = type 1
        ]);

        // Code section: 2 bodies
        // Body 0: i32.const 20, i32.const 22, call 1, end
        let body0: &[u8] = &[0x00, 0x41, 0x14, 0x41, 0x16, 0x10, 0x01, 0x0B];
        // Body 1: local.get 0, local.get 1, i32.add, end
        let body1: &[u8] = &[0x00, 0x20, 0x00, 0x20, 0x01, 0x6A, 0x0B];

        let mut code_sec: Vec<u8> = Vec::new();
        code_sec.push(0x02); // 2 bodies
        push_leb128(&mut code_sec, body0.len() as u32);
        code_sec.extend_from_slice(body0);
        push_leb128(&mut code_sec, body1.len() as u32);
        code_sec.extend_from_slice(body1);

        wasm_bytes.push(0x0A);
        push_leb128(&mut wasm_bytes, code_sec.len() as u32);
        wasm_bytes.extend_from_slice(&code_sec);

        let module = wasm::parse(&wasm_bytes).expect("parse");
        let mut interp = Interpreter::new(&module, &wasm_bytes, 0);
        let mut memory = [0u8; 256];
        let mut host = NoHost;

        let result = interp.call_function(0, &[], &mut memory, &mut host).unwrap();
        assert_eq!(result, Some(42));
    }
}
