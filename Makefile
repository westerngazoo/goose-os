KERNEL_ELF := target/riscv64gc-unknown-none-elf/release/goose-os
QEMU := qemu-system-riscv64
QEMU_ARGS := -machine virt -nographic -bios default

.PHONY: build run debug objdump clean

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

# Disassemble to verify _start is at 0x80200000
objdump: build
	rust-objdump -d $(KERNEL_ELF) | head -80

clean:
	cargo clean
