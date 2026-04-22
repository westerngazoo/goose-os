//! GooseOS — A RISC-V operating system written in Rust
//!
//! Phase 12: Preemptive Scheduling — timer-driven context switches.

// When running `cargo test`, use host std library.
// When building for RISC-V, use no_std/no_main.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
// Workaround: rustc nightly ICE on dead_code lint analysis in process module.
// https://github.com/rust-lang/rust/issues/ — crashes in lint_mod/check_mod_deathness.
// Also moved #[cfg(feature)] from fn dump_procs item to inner body to avoid the ICE.
#![allow(dead_code)]

#[cfg(not(test))]
mod console;
#[cfg(not(test))]
mod platform;
#[cfg(not(test))]
mod plic;
#[cfg(not(test))]
mod trap;
#[cfg(not(test))]
mod uart;

mod page_alloc;
#[allow(dead_code)]
mod page_table;
#[cfg(not(test))]
mod kvm;
#[cfg(not(test))]
mod process;
#[cfg(not(test))]
mod ipc;
#[cfg(not(test))]
mod syscall;
#[cfg(not(test))]
mod elf;
#[allow(dead_code)]
mod security;
#[allow(dead_code)]
mod wasm;
#[allow(dead_code)]
mod interp;
#[allow(dead_code)]
mod wasi;

// Phase B: Networking modules
#[cfg(not(test))]
mod driver;
#[cfg(all(not(test), feature = "net"))]
mod virtio;
#[cfg(all(not(test), feature = "net"))]
mod net;


// ── Kernel code (only compiled for RISC-V target, not during host tests) ──

#[cfg(not(test))]
mod kernel {
    use core::arch::{asm, global_asm};
    use crate::{page_alloc, kvm, process, println, platform, trap, plic, uart, wasi};
    #[cfg(feature = "net")]
    use crate::driver::NetworkDevice;

    // Include the RISC-V assembly boot code.
    global_asm!(include_str!("boot.S"));

    /// Call SBI System Reset extension to reboot the machine.
    ///
    /// SBI SRST extension:
    ///   EID = 0x53525354 ("SRST")
    ///   FID = 0
    ///   a0  = reset_type  (0=shutdown, 1=cold reboot, 2=warm reboot)
    ///   a1  = reset_reason (0=no reason, 1=system failure)
    pub fn sbi_system_reset() -> ! {
        unsafe {
            asm!(
                "ecall",
                in("a0") 1usize,       // reset_type = cold reboot
                in("a1") 0usize,       // reset_reason = no reason
                in("a6") 0usize,       // FID = 0
                in("a7") 0x53525354usize, // EID = SRST
                options(noreturn)
            );
        }
    }

    /// Kernel main — called from boot.S after stack setup.
    ///
    /// # Arguments
    /// * `hart_id`  - Hardware thread ID (from OpenSBI via a0)
    /// * `dtb_addr` - Device tree blob address (from OpenSBI via a1)
    #[no_mangle]
    pub extern "C" fn kmain(hart_id: usize, dtb_addr: usize) -> ! {
        // === Phase 1: UART init (polling) ===
        let uart = uart::Uart::platform();
        uart.init();

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
        println!("  Kernel entry:  {:#010x}", kmain as *const () as usize);
        println!();

        // === Phase 2: Set up trap vector (but don't enable IRQs yet) ===
        trap::trap_init();

        // === Phase 3: Configure PLIC ===
        plic::init();

        // === Phase 4: Enable UART RX interrupts on the chip ===
        // The UART chip will assert IRQ when data arrives, but the PLIC
        // does NOT route it yet — plic::init() only sets priority/threshold.
        // The PLIC enable bit is set later when PID 2 calls SYS_IRQ_REGISTER.
        // This prevents an IRQ flood before any process owns the interrupt.
        uart.enable_rx_interrupt();
        println!("  [uart] RX interrupts enabled (PLIC routing deferred)");

        // === Phase 5: Arm the timer ===
        trap::timer_init();

        // === Phase 6: Go live — enable interrupts globally ===
        trap::interrupts_enable();

        // === Phase 7: Initialize page allocator ===
        page_alloc::init();
        page_alloc::self_test();

        // === Phase 8: Build kernel page table + enable MMU ===
        let root_pt = kvm::init();
        unsafe { kvm::enable_mmu(root_pt); }

        let alloc = unsafe { page_alloc::get() };
        println!("  [page_alloc] {} pages used for page tables, {} free",
            alloc.allocated_count(), alloc.free_count());
        println!();

        // === Phase B: VirtIO device discovery + network init ===
        #[cfg(feature = "net")]
        {
            println!("  [virtio] Probing VirtIO MMIO devices...");
            if let Some((slot, irq)) = crate::virtio::probe_all() {
                println!("  [virtio] Found virtio-net at slot {} (IRQ {})", slot, irq);
                match crate::virtio::init_device() {
                    Ok(()) => {
                        let mac = unsafe { crate::virtio::get().mac_address() };
                        println!("  [virtio] virtio-net initialized (MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
                            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

                        // Enable VirtIO IRQ at PLIC
                        plic::enable_irq(irq);

                        // Initialize smoltcp
                        crate::net::init();
                        println!("  [net] Network stack initialized (10.0.2.15/24, gw 10.0.2.2)");
                        // Note: `net::smoke_test()` is disabled — sending a
                        // boot-time UDP to 10.0.2.2 poisons the ARP cache
                        // before userspace gets a chance, and slirp doesn't
                        // reliably ARP-reply for the gateway itself. The
                        // userspace net-test now drives the TX/RX path.
                    }
                    Err(e) => println!("  [virtio] Init failed: {:?}", e),
                }
            } else {
                println!("  [virtio] No virtio-net device found");
            }
            println!();
        }

        // === Phase 16: WASM test mode ===
        // If wasm-test feature is active, run the WASM interpreter instead
        // of launching normal user processes. Tests WASI Hello World.
        #[cfg(feature = "wasm-test")]
        {
            println!("  [wasm] === WASM/WASI Test ===");
            println!();
            let code = wasi::run_wasm_test();
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

        // === Phase 12: Create processes + launch scheduler ===
        // Creates init (PID 1) + UART server (PID 2), then srets to PID 1.
        // RPC between processes — init calls server, server prints + replies.
        // After all processes exit, control returns to post_process_exit().
        #[cfg(not(feature = "wasm-test"))]
        process::launch();
    }

    /// Panic handler — prints location and message, then halts.
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        // Disable interrupts so panic output isn't interleaved
        unsafe { asm!("csrc sstatus, {}", in(reg) 1usize << 1); }

        println!();
        println!("!!! KERNEL PANIC !!!");

        if let Some(location) = info.location() {
            println!(
                "  at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }

        if let Some(message) = info.message().as_str() {
            println!("  {}", message);
        } else {
            println!("  {}", info.message());
        }

        println!();
        println!("System halted.");

        loop {
            unsafe { asm!("wfi") };
        }
    }
}
