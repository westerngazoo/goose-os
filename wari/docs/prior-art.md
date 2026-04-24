# Wari — Prior Art

> We are not a research project. We stand on specific, cited work.
> Every pattern in Wari either inherits from someone named here
> (with credit in the code), deliberately rejects one with written
> justification, or is labeled as our own bet with risk mitigation.

---

## Commercial cloud compute

### Cloudflare Workers (2017–) — V8 isolates

**Source**: Kenton Varda's public writing on the Workers architecture.
Notably: ["Fine-grained Sandboxing with V8 Isolates"](https://blog.cloudflare.com/cloud-computing-without-containers/)
(2018) and follow-ups.

**Key insight**: process-per-tenant is too expensive for per-request
isolation. Shared V8 runtime + separate heaps gives isolation without
the context-switch cost. Millions of tenants on shared infrastructure.

**What we inherit**: the shared-runtime density model — confirmed our
decision to run Tier-2 drivers with no MMU barrier between them.

**What we reject**: V8 as the runtime. JavaScript isn't our primary;
V8 is ~50 MB of Google-controlled C++ we can't audit at LATAM-sovereign-
procurement standard; RISC-V support is young. JS-to-WASM compilers
(Javy, AssemblyScript, Porffor) let customers ship JS and we run WASM.

### Fastly Compute@Edge (2019–) — pure Wasmtime

**Source**: Fastly engineering blogs, the Lucet → Wasmtime transition
writeup, the Bytecode Alliance's work.

**Key insight**: WASM as the process boundary, fresh instance per
request, microsecond cold start when JIT is available.

**What we inherit**: WASM-only user model. Validates wasmi as the
conservative start (~ms cold start) with JIT migration later.

### AWS Lambda + Firecracker (2018–) — Rust microVM

**Source**: Firecracker's open-source code and [NSDI 2020 paper](https://www.usenix.org/conference/nsdi20/presentation/agache)
("Firecracker: Lightweight Virtualization for Serverless Applications").

**Key insight**: narrow-purpose Rust VMM beats general-purpose
hypervisors for serverless workloads. Tight scope = small attack
surface.

**What we inherit**: the narrow-purpose Rust kernel discipline —
Tier 0 is ~5–10 KLOC. Firecracker is ~50 KLOC; we aim smaller because
we don't carry a VMM.

**What we reject**: microVM-per-invocation. Too heavy for our density
goal (10k–50k tenants per board vs. hundreds for Firecracker).

### AWS Nitro (2017–) — HW/SW co-design

**Source**: AWS re:Invent talks on Nitro architecture, patents.

**Key insight**: offload network, storage, and security to custom
hardware. Reduces hypervisor TCB dramatically.

**What we inherit**: HW/SW co-design as strategy. Our analog is the
GAPU FPGA coprocessor (Phase 3) and eventually MMU-free custom silicon
(Phase 4).

### Google gVisor (2018–) — userspace syscall shim

**Source**: gVisor's open-source code and design docs.

**Key insight**: interpose on syscalls in userspace, reduce kernel
attack surface available to containers.

**What we reject**: the interposition layer. We control the whole
stack; no legacy kernel to defend against. Tier 0 is small enough
not to need a shim.

### KataContainers (2017–) — VM per container

**Source**: OpenStack Summit presentations, Kata's architecture docs.

**What we reject**: OCI compatibility as an architectural constraint.
Retrofitting strong isolation to Docker images defeats the density
advantage. Our answer is `tools/oci2wasm/` (Phase 2): customer brings
Docker image → host tooling compiles to WASM → Wari runs WASM.

---

## Academic + research OSes

### seL4 (2009–) — formally verified microkernel

**Source**: [Klein et al., SOSP 2009 "seL4: Formal verification of an
OS kernel"](https://trustworthy.systems/publications/nicta_full_text/1852.pdf)
and the Isabelle/HOL proof corpus.

**Key insights**:
  - Capability-based access control scales better than ambient authority.
  - Synchronous rendezvous IPC has no buffers, no kernel allocation,
    and is verifiable.
  - Formal verification of ~10 KLOC kernel is possible with discipline.

**What we inherit**: capability system (Phase 1), synchronous IPC
(Phase 0, from goose-os's implementation), the formal-verification
target (Phase 3–4). The Wari IPC implementation is directly modeled
on seL4's with credit in the doc comments.

### Singularity OS (MSR, 2003–08) — managed-code OS

**Source**: [Hunt & Larus, "Singularity: Rethinking the Software
Stack"](https://www.microsoft.com/en-us/research/publication/singularity-rethinking-the-software-stack/)
(MSR-TR-2007-49) and the subsequent SOSP papers.

**Key insight**: language-enforced isolation (C# SIPs) without hardware
page tables between processes. Proves the architectural move is sound.

**What we inherit**: the architectural endpoint — Phase 4's
MMU-free custom silicon option is Singularity's dream with WASM +
wasmi as the 2026 enablers (smaller runtime than CLR, cross-language,
machine-verifiable type system).

**Why Singularity didn't become mainstream**: business (Windows
backward compat, heavy C# runtime), not technical. We learn from the
post-mortem, not the failure mode.

### Tock OS (Stanford, 2015–) — Rust embedded kernel

**Source**: [Levy et al., SOSP 2017 "The case for writing a kernel in
Rust"](https://www.cs.stanford.edu/~aldenhilton/tock-sosp17.pdf) and
production use in Signal's secure messaging hardware.

**Key insight**: Rust type system replaces the MMU for process
isolation in embedded. Production-deployed in security-critical
hardware.

**What we inherit**: the proof that Rust type system can carry OS-
scale isolation. Gives confidence in the Phase 4 MMU-free direction.

### RedLeaf (UCI, 2020) — Rust domains

**Source**: [Narayanan et al., SOSP 2020 "RedLeaf: Isolation and
Communication in a Safe OS Kernel"](https://www.usenix.org/system/files/osdi20-narayanan_vikram.pdf).

**Key insight**: Rust-enforced "domains" with language-level isolation
between kernel components. Zero-copy IPC via type system. Closest
academic sibling to Wari's two-tier model.

**What we inherit**: the language-enforced-isolation precedent cited
directly. Phase 1 capability design adopts their terminology.

### MirageOS (Cambridge, 2013–) — unikernels

**Source**: [Madhavapeddy et al., ASPLOS 2013 "Unikernels: Library
Operating Systems for the Cloud"](http://anil.recoil.org/papers/2013-asplos-mirage.pdf).

**Key insight**: single-address-space OS-as-binary. Extreme
specialization = extreme size reduction.

**What we inherit (Phase 4+)**: the specialization direction. A Wari
module + libc + wasmi linked into a single image for latency-critical
workloads is a MirageOS-flavored future.

### Hubris (Oxide Computer, 2021–) — Rust embedded microkernel

**Source**: [Oxide's Hubris documentation](https://hubris.oxide.computer/)
and Cliff Biffle's writing.

**Key insight**: static task set, no heap in the kernel, simple
scheduling. Rust microkernel for real production hardware (Oxide rack).

**What we inherit**: the "no heap in dispatch" rule (Wari R2) and
static-everywhere discipline. Hubris is the most visible production-
Rust-kernel team of the 2020s.

---

## Confidential computing

### RISC-V CoVE (2024–) — confidential VM extension

**Source**: RISC-V Confidential VM Extension specification (ratified
2024). Silicon landing 2026–27.

**What we inherit**: Phase 3 integration target. Analog to Intel TDX
and AMD SEV-SNP but on open ISA.

**What we reject**: Intel SGX lineage — proprietary silicon isolation,
deprecated by Intel itself. Cautionary tale about betting on closed
silicon features.

---

## WASM standards

### WASI Preview 1 / Preview 2 / Component Model

**Source**: WASI specs at the Bytecode Alliance.

**Phase 0–1 baseline**: Preview 1 subset. Mature, widely implemented,
compatible with wasi-libc.

**Phase 2 migration target**: Preview 2 + Component Model. Released
2024, ecosystem adopting. Our Phase 0–1 interface design chooses
boundaries that slot into Component Model cleanly.

**What we reject**: WASIX (Wasmer's competing superset). Fragmentary,
vendor-controlled.

### WASI-NN

**Source**: WASI-NN draft spec + Wasmtime's experimental implementation.

**Phase 2 target**: `wari_ai_infer` follows the WASI-NN shape so
Wari-built inference modules port to other WASI-NN hosts.

---

## What's genuinely our bet

After all the credit: what's original to Wari?

1. **Two-tier WASM** — Tier 1 (MMU + WASM) vs. Tier 2 (WASM-only,
   ring 0). Closer to Singularity than to any commercial cloud. Our
   defensible moat if it works; requires `wasmi` to be highly correct.

2. **GAPU FPGA as architectural peer to GPU** — not just "we happen
   to also support FPGA." Phase 3 treats GAPU as the canonical
   AI-inference driver path for workloads where sovereignty matters
   more than per-TOPS cost.

3. **LATAM-sovereign positioning** — not a technical innovation, a
   market one. The stack we build is auditable by governments that
   can't (or won't) audit x86 + Nvidia.

4. **Formal verification from day one, not a retrofit** — most commercial
   clouds defer this indefinitely. seL4 did it right. We shape the
   code for it now.

---

## Reading list for contributors

Minimum reading before contributing to Wari architecture:

1. seL4 SOSP'09 paper (capabilities, IPC)
2. Firecracker NSDI'20 paper (narrow Rust VMM discipline)
3. RedLeaf SOSP'20 paper (Rust domain isolation)
4. Cloudflare's "Cloud Computing Without Containers" (2018 blog — density bet)
5. Fastly's Lucet/Wasmtime architecture (WASM process boundary)
6. Tock SOSP'17 paper (Rust in production OS)

Everything else is optional and will make sense after those six.
