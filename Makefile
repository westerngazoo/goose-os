KERNEL_ELF := target/riscv64gc-unknown-none-elf/release/goose-os
QEMU := qemu-system-riscv64
QEMU_ARGS := -machine virt -nographic -bios default

# llvm-objcopy from Rust toolchain (install with: rustup component add llvm-tools)
OBJCOPY := $(shell find $${HOME}/.rustup -name llvm-objcopy -type f 2>/dev/null | head -1)

.PHONY: build run test debug objdump clean build-vf2 kernel-vf2 flash-sd

# ── QEMU (default) ──────────────────────────────────────────

build:
	cargo build --release

run: build
	@echo ">>> Exit QEMU: Ctrl-A then X  (two separate presses)"
	@echo ">>> Or from another terminal: pkill qemu-system"
	@echo ""
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF)

# Run with auto-timeout (good for testing)
test: build
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

# Start QEMU paused with GDB server on port 1234
debug: build
	@echo "Connect GDB with: riscv64-linux-gnu-gdb -ex 'target remote :1234' $(KERNEL_ELF)"
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) -s -S

# Disassemble to verify _start is at correct address
objdump: build
	rust-objdump -d $(KERNEL_ELF) | head -80

# ── VisionFive 2 ────────────────────────────────────────────

build-vf2:
	RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features vf2 --no-default-features

# Extract raw binary for U-Boot (strips ELF headers)
kernel-vf2: build-vf2
	$(OBJCOPY) -O binary $(KERNEL_ELF) kernel.bin
	@ls -lh kernel.bin
	@echo ">>> kernel.bin ready for VisionFive 2"

# Copy kernel.bin to SD card FAT32 partition
# Usage: make flash-sd SD=/media/sdcard
flash-sd: kernel-vf2
ifndef SD
	$(error Set SD= to the mounted FAT32 partition, e.g.: make flash-sd SD=/media/goose/boot)
endif
	cp kernel.bin $(SD)/kernel.bin
	sync
	@echo ">>> Copied kernel.bin to $(SD)"
	@echo ">>> In U-Boot, run:"
	@echo ">>>   fatload mmc 1:1 0x40200000 kernel.bin"
	@echo ">>>   go 0x40200000"

# ── Common ──────────────────────────────────────────────────

clean:
	cargo clean
	rm -f kernel.bin
