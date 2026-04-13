KERNEL_ELF := target/riscv64gc-unknown-none-elf/release/goose-os
QEMU := qemu-system-riscv64
QEMU_ARGS := -machine virt -nographic -bios default

# llvm-objcopy from Rust toolchain (install with: rustup component add llvm-tools)
OBJCOPY := $(shell find $${HOME}/.rustup -name llvm-objcopy -type f 2>/dev/null | head -1)

# Auto-incrementing build number
BUILD_FILE := .build_number
BUILD_NUM := $(shell cat $(BUILD_FILE) 2>/dev/null || echo 0)
NEXT_BUILD := $(shell echo $$(($(BUILD_NUM) + 1)))

# VF2 deploy target (IP of VisionFive 2 on local network)
VF2_IP ?= 192.168.86.237

.PHONY: build run test debug objdump clean build-vf2 kernel-vf2 flash-sd deploy

# ── QEMU (default) ──────────────────────────────────────────

build:
	GOOSE_BUILD=$(NEXT_BUILD) cargo build --release
	@echo $(NEXT_BUILD) > $(BUILD_FILE)

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

# Debug kernel: enables kdebug!/kdump macros for verbose tracing.
run-debug:
	GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features debug-kernel
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo ">>> Debug kernel — kdebug/kdump macros active"
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF)

test-debug:
	GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features debug-kernel
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

# Deploy debug kernel to VF2
deploy-debug:
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	GOOSE_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features "vf2 debug-kernel" --no-default-features
	$(OBJCOPY) -O binary $(KERNEL_ELF) kernel.bin
	@ls -lh kernel.bin
	git add kernel.bin src/ Makefile Cargo.toml linker.ld linker-vf2.ld .build_number goose-upgrade.sh
	git commit -m "Build $(NEXT_BUILD) (debug)" --allow-empty || true
	git push
	@echo ">>> DEPLOYED debug kernel build $(NEXT_BUILD)"

# Security test: boots a malicious process that tests all attack vectors.
# Expected output: P1..P8 (pass), K9 (attempt), then "Process fault" + kill.
# Any "F<n>" or "!!!" in the output means a security check is broken.
test-security:
	GOOSE_BUILD=sec cargo build --release --features security-test --no-default-features
	@echo "=== Security Test ==="
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

# ── VisionFive 2 ────────────────────────────────────────────

build-vf2:
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	GOOSE_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features vf2 --no-default-features

# Extract raw binary for U-Boot (strips ELF headers)
kernel-vf2: build-vf2
	$(OBJCOPY) -O binary $(KERNEL_ELF) kernel.bin
	@ls -lh kernel.bin
	@echo ">>> kernel.bin ready — build $(NEXT_BUILD)"

# One-command deploy: build, push to git, print instructions
deploy: kernel-vf2
	git add kernel.bin src/ Makefile Cargo.toml linker.ld linker-vf2.ld .build_number goose-upgrade.sh
	git commit -m "Build $(NEXT_BUILD)" --allow-empty || true
	git push
	@echo ""
	@echo "========================================="
	@echo "  DEPLOYED: build $(NEXT_BUILD)"
	@echo "========================================="
	@echo "  On VF2 Debian, run:"
	@echo "    goose go"
	@echo "========================================="

# Copy kernel.bin to SD card FAT32 partition
# Usage: make flash-sd SD=/media/sdcard
flash-sd: kernel-vf2
ifndef SD
	$(error Set SD= to the mounted FAT32 partition, e.g.: make flash-sd SD=/media/goose/boot)
endif
	cp kernel.bin $(SD)/kernel.bin
	sync
	@echo ">>> Copied kernel.bin to $(SD)"

# ── Common ──────────────────────────────────────────────────

clean:
	cargo clean
	rm -f kernel.bin
