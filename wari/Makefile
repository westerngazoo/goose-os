# Wari — build + test + deploy.
#
# Patterned on ../goose-os/Makefile (which reliably produced both
# QEMU-bootable kernels and VF2 hardware images across ~100 builds).
# Build numbers continue monotonically from goose-os (see CLAUDE.md
# §PR Workflow → Build numbering).

KERNEL_CRATE := wari-kernel
KERNEL_ELF   := kernel/target/riscv64gc-unknown-none-elf/release/wari
KERNEL_BIN   := build/wari.bin
HELLO_WASM   := apps/hello/target/wasm32-unknown-unknown/release/wari_hello.wasm
QEMU         := qemu-system-riscv64
QEMU_ARGS    := -machine virt -nographic -bios default

# llvm-objcopy from Rust toolchain (install with: rustup component add llvm-tools)
OBJCOPY := $(shell find $${HOME}/.rustup -name llvm-objcopy -type f 2>/dev/null | head -1)

# Build-number ratchet — continuous with goose-os.
BUILD_FILE := .build_number
BUILD_NUM  := $(shell cat $(BUILD_FILE) 2>/dev/null || cat ../goose-os/.build_number 2>/dev/null || echo 101)
NEXT_BUILD := $(shell echo $$(($(BUILD_NUM) + 1)))

# VF2 deploy target (IP of VisionFive 2 on local network — inherited from goose-os)
VF2_IP ?= 192.168.86.237

# File sets committed by each deploy target
DEPLOY_FILES := $(KERNEL_BIN) kernel/ abi-shared/ wasi/ apps/ drivers/ \
                platform/ scripts/ docs/ Makefile Cargo.toml Cargo.lock \
                rust-toolchain.toml CLAUDE.md README.md .build_number

.PHONY: help build build-hello build-all test run debug objdump clean \
        kernel-vf2 flash-sd deploy \
        test-unit test-integration test-security test-fuzz \
        clippy fmt check audit

help:
	@echo "Wari — make targets"
	@echo ""
	@echo "  Build:"
	@echo "    build              kernel (qemu target)"
	@echo "    build-hello        Tier-1 hello.wasm"
	@echo "    build-all          kernel + hello + drivers (Phase 1+)"
	@echo "    kernel-vf2         kernel binary for VF2 hardware"
	@echo ""
	@echo "  Run / test:"
	@echo "    run                build + boot in QEMU"
	@echo "    test               timed QEMU boot (Phase 0 smoke)"
	@echo "    debug              QEMU with GDB server :1234"
	@echo "    test-unit          host-side cargo test --workspace"
	@echo "    test-integration   QEMU-driven integration tests"
	@echo "    test-security      adversarial WASM tests"
	@echo "    test-fuzz          cargo-fuzz targets (long-running)"
	@echo ""
	@echo "  Quality gates:"
	@echo "    clippy             cargo clippy -- -D warnings"
	@echo "    fmt                cargo fmt --check"
	@echo "    check              clippy + fmt + test-unit"
	@echo "    audit              cargo-audit on Cargo.lock"
	@echo ""
	@echo "  Deploy:"
	@echo "    deploy             build + commit + push for VF2"
	@echo "    flash-sd SD=/path  copy kernel.bin to SD card"
	@echo ""
	@echo "  Misc:"
	@echo "    objdump            disassemble kernel .text"
	@echo "    clean              cargo clean"

# ── Build ──────────────────────────────────────────────────────

build:
	cd kernel && WARI_BUILD=$(NEXT_BUILD) cargo build --release --features qemu
	@echo $(NEXT_BUILD) > $(BUILD_FILE)

build-hello:
	cd apps/hello && cargo build --release

build-all: build build-hello

# ── Run / test ─────────────────────────────────────────────────

run: build
	@echo ">>> Exit QEMU: Ctrl-A then X  (two separate presses)"
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF)

test: build
	timeout 5 $(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) || true

debug: build
	@echo "Connect: riscv64-linux-gnu-gdb -ex 'target remote :1234' $(KERNEL_ELF)"
	$(QEMU) $(QEMU_ARGS) -kernel $(KERNEL_ELF) -s -S

objdump: build
	rust-objdump -d $(KERNEL_ELF) | head -80

# ── Host-side tests ────────────────────────────────────────────

test-unit:
	cargo test --workspace

test-integration: build build-hello
	cd tests/integration && cargo test --release

test-security: build build-hello
	cd tests/security && cargo test --release

test-fuzz:
	@echo ">>> Long-running. Use cargo fuzz run <target> for a specific target."
	cd tests/fuzz && cargo fuzz list

# ── Quality gates ──────────────────────────────────────────────

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all --check

check: fmt clippy test-unit
	@echo ">>> check OK"

audit:
	@which cargo-audit >/dev/null || (echo "install: cargo install cargo-audit" && exit 1)
	cargo audit

# ── VisionFive 2 ───────────────────────────────────────────────

kernel-vf2:
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	cd kernel && WARI_BUILD=$(NEXT_BUILD) RUSTFLAGS="-C link-arg=-Tlinker-vf2.ld" \
	  cargo build --release --features vf2 --no-default-features
	$(OBJCOPY) -O binary $(KERNEL_ELF) $(KERNEL_BIN)
	@ls -lh $(KERNEL_BIN)
	@echo ">>> $(KERNEL_BIN) ready — build $(NEXT_BUILD)"

# One-command deploy: build, commit, push. Matches goose-os flow so
# the VF2 device-side `wari go` script (Phase 1a port) pulls + flashes.
deploy: kernel-vf2
	git add $(DEPLOY_FILES)
	git commit -m "Build $(NEXT_BUILD) (wari deploy)" --allow-empty || true
	git push
	@echo ""
	@echo "========================================="
	@echo "  DEPLOYED: build $(NEXT_BUILD)"
	@echo "========================================="
	@echo "  On VF2, run:   wari go"
	@echo "========================================="

flash-sd: kernel-vf2
ifndef SD
	$(error Set SD= to mounted FAT32 partition: make flash-sd SD=/media/goose/boot)
endif
	cp $(KERNEL_BIN) $(SD)/kernel.bin
	sync
	@echo ">>> Copied $(KERNEL_BIN) to $(SD)"

# ── Housekeeping ───────────────────────────────────────────────

clean:
	cargo clean
	rm -f $(KERNEL_BIN)
