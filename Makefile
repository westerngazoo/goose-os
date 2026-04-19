KERNEL_ELF := kernel/target/riscv64gc-unknown-none-elf/release/goose-os
KERNEL_BIN := build/kernel.bin
USER_ELF   := userspace/hello/target/riscv64gc-unknown-none-elf/release/hello
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

# File sets committed by each deploy target
DEPLOY_FILES := $(KERNEL_BIN) kernel/ userspace/ platform/ scripts/ docs/ Makefile .build_number

.PHONY: build run test debug objdump clean build-vf2 kernel-vf2 flash-sd deploy

# ── QEMU (default) ──────────────────────────────────────────

build:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release
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
#
# KNOWN ISSUE (as of nightly 2026-04): this target hits a rustc ICE in
# lint_mod/check_mod_deathness when the `debug-kernel` feature is on.
# The #![allow(dead_code)] workaround in main.rs dodges the default
# build but not this one. Fix options: pin rust-toolchain to an older
# nightly, or wait for upstream rustc fix.
run-debug:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features debug-kernel
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo ">>> Debug kernel — kdebug/kdump macros active"
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF)

test-debug:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features debug-kernel
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

# Deploy debug kernel to VF2
deploy-debug:
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features "vf2 debug-kernel" --no-default-features
	$(OBJCOPY) -O binary $(KERNEL_ELF) $(KERNEL_BIN)
	@ls -lh $(KERNEL_BIN)
	git add $(DEPLOY_FILES)
	git commit -m "Build $(NEXT_BUILD) (debug)" --allow-empty || true
	git push
	@echo ">>> DEPLOYED debug kernel build $(NEXT_BUILD)"

# WASM/WASI test: runs the WASM interpreter with a hand-crafted Hello World.
# Expected output: "Hello from WASM!" then exit code 0.
run-wasm:
	cd kernel && GOOSE_BUILD=wasm cargo build --release --features "qemu wasm-test" --no-default-features
	@echo "=== WASM/WASI Test ==="
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF)

test-wasm:
	cd kernel && GOOSE_BUILD=wasm cargo build --release --features "qemu wasm-test" --no-default-features
	@echo "=== WASM/WASI Test ==="
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

# Deploy WASM test kernel to VF2
deploy-wasm:
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features "vf2 wasm-test" --no-default-features
	$(OBJCOPY) -O binary $(KERNEL_ELF) $(KERNEL_BIN)
	@ls -lh $(KERNEL_BIN)
	git add $(DEPLOY_FILES)
	git commit -m "Build $(NEXT_BUILD) (wasm-test)" --allow-empty || true
	git push
	@echo ">>> DEPLOYED wasm-test kernel build $(NEXT_BUILD)"

# Rust userspace: boots a compiled Rust ELF binary as PID 1.
build-user:
	cd userspace/hello && CARGO_ENCODED_RUSTFLAGS='-Clink-arg=-Tlinker.ld' cargo build --release

# Same ELF, but with the `net` feature enabled so main.rs runs the
# NET_STATUS / NET_SOCKET_UDP / NET_BIND / NET_CLOSE exercise.
# Must be paired with a kernel built with `net` — see test-net-user.
build-user-net:
	cd userspace/hello && CARGO_ENCODED_RUSTFLAGS='-Clink-arg=-Tlinker.ld' cargo build --release --features net

run-rust-user: build-user
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu rust-user" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo ">>> Rust userspace — compiled Rust ELF as PID 1"
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF)

test-rust-user: build-user
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu rust-user" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo "=== Rust Userspace Test ==="
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

deploy-rust-user: build-user
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features "vf2 rust-user" --no-default-features
	$(OBJCOPY) -O binary $(KERNEL_ELF) $(KERNEL_BIN)
	@ls -lh $(KERNEL_BIN)
	git add $(DEPLOY_FILES)
	git commit -m "Build $(NEXT_BUILD) (rust-user)" --allow-empty || true
	git push
	@echo ">>> DEPLOYED rust-user kernel build $(NEXT_BUILD)"

# ── Networking ──────────────────────────────────────────────────

# QEMU args for virtio-net (user-mode networking)
# force-legacy=false makes QEMU present VirtIO MMIO v2 (modern) transport
# instead of v1 (legacy), which our driver expects.
NET_ARGS := -global virtio-mmio.force-legacy=false \
            -netdev user,id=net0,hostfwd=tcp::2222-:22 \
            -device virtio-net-device,netdev=net0

# Run with networking enabled
run-net:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu net" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo ">>> Network enabled: virtio-net + user-mode networking"
	@echo ">>> Host port 2222 -> guest port 22"
	$(QEMU) $(QEMU_ARGS) $(NET_ARGS) -kernel $(KERNEL_ELF)

test-net:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu net" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo "=== Network Test ==="
	timeout 5 $(QEMU) $(QEMU_ARGS) $(NET_ARGS) -kernel $(KERNEL_ELF) || true

# Run with TAP networking (requires: sudo ip tuntap add dev tap0 mode tap)
run-tap:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu net" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo ">>> TAP networking — requires: sudo ip tuntap add dev tap0 mode tap"
	$(QEMU) $(QEMU_ARGS) \
	  -netdev tap,id=net0,ifname=tap0,script=no,downscript=no \
	  -device virtio-net-device,netdev=net0 \
	  -kernel $(KERNEL_ELF)

# Run with packet capture for network debugging
run-net-debug:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu net debug-kernel" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo ">>> Packet capture to build/goose-net.pcap"
	$(QEMU) $(QEMU_ARGS) $(NET_ARGS) \
	  -object filter-dump,id=f1,netdev=net0,file=build/goose-net.pcap \
	  -kernel $(KERNEL_ELF)

# Run with rust-user + net + pcap capture (full userspace net test)
test-net-user: build-user-net
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release --features "qemu rust-user net" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo "=== Rust Userspace Net Test ==="
	timeout 6 $(QEMU) $(QEMU_ARGS) $(NET_ARGS) \
	  -object filter-dump,id=f1,netdev=net0,file=build/goose-net.pcap \
	  -kernel $(KERNEL_ELF) || true

# Security test: boots a malicious process that tests all attack vectors.
# Expected output: P1..P8 (pass), K9 (attempt), then "Process fault" + kill.
# Any "F<n>" or "!!!" in the output means a security check is broken.
test-security:
	cd kernel && GOOSE_BUILD=sec cargo build --release --features security-test --no-default-features
	@echo "=== Security Test ==="
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

# ── VisionFive 2 ────────────────────────────────────────────

build-vf2:
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features vf2 --no-default-features

# Extract raw binary for U-Boot (strips ELF headers)
kernel-vf2: build-vf2
	$(OBJCOPY) -O binary $(KERNEL_ELF) $(KERNEL_BIN)
	@ls -lh $(KERNEL_BIN)
	@echo ">>> $(KERNEL_BIN) ready — build $(NEXT_BUILD)"

# One-command deploy: build, push to git, print instructions
deploy: kernel-vf2
	git add $(DEPLOY_FILES)
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
	cp $(KERNEL_BIN) $(SD)/kernel.bin
	sync
	@echo ">>> Copied $(KERNEL_BIN) to $(SD)"

# ── Common ──────────────────────────────────────────────────

clean:
	cd kernel && cargo clean
	cd userspace/hello && cargo clean
	rm -f $(KERNEL_BIN)
