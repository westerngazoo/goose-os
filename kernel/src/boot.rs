//! Kernel boot orchestration.
//!
//! `kmain` (in main.rs) called from boot.S hands off immediately to
//! `run()` here. `run()` executes the boot sequence as a list of
//! named stages, each with a documented precondition and
//! postcondition. Each stage is a standalone function, so the whole
//! boot sequence can be read as a 10-line table of contents.
//!
//! Staging rule: **a stage cannot depend on any stage that hasn't
//! run yet, and must leave the system in a state usable by every
//! stage below.** If a new stage needs something no stage above it
//! provides, the new thing either goes earlier, or a new stage is
//! created for it. No reorderings "because it also works this way" —
//! the sequence here is load-bearing.
//!
//! The stages, in order:
//!
//!   1. uart         — early console, so panics during boot print
//!   2. banner       — welcome screen, build info, hart id
//!   3. interrupts   — trap vector, PLIC, timer, S-mode IRQs on
//!   4. memory       — physical page allocator + self-test
//!   5. mmu          — Sv39 page table built and active
//!   6. network      — [feature=net] VirtIO probe + smoltcp init
//!   7. wasm_test    — [feature=wasm-test] run WASM test, then idle
//!   8. user         — spawn init/PID 1 and enter the scheduler
//!
//! Stages 6 and 7 are feature-gated. Stage 8 is the terminal stage;
//! control enters the scheduler and does not return to `run()`.

use crate::{kvm, page_alloc, platform, plic, println, process, trap, uart};

/// Boot sequence entry point. Called from `kmain` in main.rs.
///
/// Never returns — the final stage hands off to the scheduler (or to
/// the idle loop for feature-gated test modes).
#[no_mangle]
pub fn run(hart_id: usize, dtb_addr: usize) -> ! {
    stage_uart();
    stage_banner(hart_id, dtb_addr);
    stage_interrupts();
    stage_memory();
    stage_mmu();

    #[cfg(feature = "net")]
    stage_network();

    #[cfg(feature = "wasm-test")]
    stage_wasm_test();

    #[cfg(not(feature = "wasm-test"))]
    stage_user();
}

// ── Stage 1: UART ─────────────────────────────────────────────

/// Bring up the platform console UART in polling mode.
///
/// Precondition:  the CPU is in S-mode; boot.S has given us a stack.
///                No memory safety guarantees beyond that.
/// Postcondition: `println!` prints to a terminal.
fn stage_uart() {
    uart::Uart::platform().init();
}

// ── Stage 2: Banner ───────────────────────────────────────────

/// Print the GooseOS banner and boot diagnostics.
///
/// Precondition:  `stage_uart()` has run.
/// Postcondition: the operator sees we booted and on which hart.
fn stage_banner(hart_id: usize, dtb_addr: usize) {
    println!();
    println!("          __");
    println!("       __( o)>     GooseOS v0.1.0 build {}", option_env!("GOOSE_BUILD").unwrap_or("dev"));
    println!("      \\  _/        RISC-V 64-bit");
    println!("       \\\\\\         Written in Rust");
    println!("        \\\\\\        Platform: {}", platform::PLATFORM_NAME);
    println!("         \\   )_    Honk.");
    println!("      ~~~^~~~~");
    println!();

    println!("  Booted on hart {}", hart_id);
    println!("  DTB address:   {:#010x}", dtb_addr);
    println!("  Kernel entry:  {:#010x}", run as *const () as usize);
    println!();
}

// ── Stage 3: Interrupts ───────────────────────────────────────

/// Install the trap vector, configure the PLIC, enable UART RX
/// interrupts on the chip, arm the timer, and finally unmask S-mode
/// interrupts.
///
/// Precondition:  console works (panics during IRQ setup need to
///                print); no user state exists yet.
/// Postcondition: `ecall` from anywhere traps; timer fires on a
///                schedule; UART RX is routed at the chip but NOT
///                YET at the PLIC — the PLIC enable bit is set
///                when PID 2 calls SYS_IRQ_REGISTER. This prevents
///                a pre-userspace IRQ flood.
fn stage_interrupts() {
    trap::trap_init();
    plic::init();
    uart::Uart::platform().enable_rx_interrupt();
    println!("  [uart] RX interrupts enabled (PLIC routing deferred)");
    trap::timer_init();
    trap::interrupts_enable();
}

// ── Stage 4: Memory ───────────────────────────────────────────

/// Initialize the physical page allocator over [_end, _heap_end).
///
/// Precondition:  MMU is OFF (we're still identity-mapping by
///                hardware default); no allocator consumer has run.
/// Postcondition: `page_alloc::get().alloc()` returns PAs in the
///                kernel-writable range. Self-test has exercised
///                alloc/free at least once.
fn stage_memory() {
    page_alloc::init();
    page_alloc::self_test();
}

// ── Stage 5: MMU ──────────────────────────────────────────────

/// Build the identity-mapped Sv39 page table and enable the MMU.
///
/// Precondition:  `stage_memory()` has run (we need the allocator
///                to create PTE pages). MMU is still OFF.
/// Postcondition: `satp` is loaded with the root; every subsequent
///                instruction fetch and data access goes through
///                the page tables. Kernel code/data/stack, UART
///                MMIO, PLIC MMIO, and (if `net`) VirtIO MMIO are
///                all mapped — the next instruction after
///                `csrw satp` must be fetchable.
fn stage_mmu() {
    let root_pt = kvm::init();
    // SAFETY: INV-1 + INV-7. kvm::init has just installed the root,
    // and every page the kernel will touch is mapped in it.
    unsafe {
        kvm::enable_mmu(root_pt);
    }

    let alloc = unsafe { page_alloc::get() };
    println!(
        "  [page_alloc] {} pages used for page tables, {} free",
        alloc.allocated_count(),
        alloc.free_count()
    );
    println!();
}

// ── Stage 6: Network (feature-gated) ──────────────────────────

/// Probe VirtIO MMIO slots for a virtio-net device; if one is found,
/// bring up the smoltcp stack with a static IP config.
///
/// Precondition:  MMU is active (VirtIO MMIO must be mapped; kvm::init
///                does this when the `net` feature is enabled).
/// Postcondition: On success, `net::init()` has run, a MAC address
///                has been printed, and the VirtIO IRQ is enabled at
///                the PLIC. On failure, a diagnostic is printed and
///                the kernel continues with networking disabled.
///                Either outcome leaves the next stage safe to run.
#[cfg(feature = "net")]
fn stage_network() {
    use crate::driver::NetworkDevice;

    println!("  [virtio] Probing VirtIO MMIO devices...");
    match crate::virtio::probe_all() {
        Some((slot, irq)) => {
            println!("  [virtio] Found virtio-net at slot {} (IRQ {})", slot, irq);
            match crate::virtio::init_device() {
                Ok(()) => {
                    // SAFETY: INV-8. init_device just completed; the
                    // singleton is now valid.
                    let mac = unsafe { crate::virtio::get().mac_address() };
                    println!(
                        "  [virtio] virtio-net initialized (MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
                    );
                    plic::enable_irq(irq);
                    crate::net::init();
                    println!("  [net] Network stack initialized (10.0.2.15/24, gw 10.0.2.2)");
                    // Note: net::smoke_test() is deliberately disabled — a
                    // boot-time UDP to 10.0.2.2 poisons the ARP cache before
                    // userspace gets a chance, and slirp doesn't reliably
                    // ARP-reply for the gateway itself. The userspace
                    // net-test drives the TX/RX path instead.
                }
                Err(e) => println!("  [virtio] Init failed: {:?}", e),
            }
        }
        None => println!("  [virtio] No virtio-net device found"),
    }
    println!();
}

// ── Stage 7: WASM test mode (feature-gated, terminal) ─────────

/// Run the in-kernel WASM interpreter's Hello World test and then
/// enter the post-exit idle loop.
///
/// This stage never returns; when the `wasm-test` feature is on, it
/// replaces Stage 8. It's a test-harness mode only — production boots
/// go through `stage_user()`.
#[cfg(feature = "wasm-test")]
fn stage_wasm_test() -> ! {
    println!("  [wasm] === WASM/WASI Test ===");
    println!();
    let code = crate::wasi::run_wasm_test();
    println!();
    println!("  [wasm] Exit code: {}", code);
    if code == 0 {
        println!("  [wasm] PASSED");
    } else {
        println!("  [wasm] FAILED");
    }
    println!();
    trap::post_process_exit(); // enter idle loop (Ctrl-R to reboot)
}

// ── Stage 8: User (terminal) ──────────────────────────────────

/// Create init (PID 1) and the UART server (PID 2), then enter the
/// scheduler by sret'ing into PID 1.
///
/// This stage never returns: control transfers to userspace, and when
/// userspace makes syscalls or traps occur, the handlers run inside
/// the interrupt context without returning to `run()`.
#[cfg(not(feature = "wasm-test"))]
fn stage_user() -> ! {
    process::launch()
}
