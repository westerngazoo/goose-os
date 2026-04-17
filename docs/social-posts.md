# Social Posts — "Bigger, Stronger, Faster" Blog Entry

---

## LinkedIn

I built a RISC-V operating system from scratch. In Rust. On real hardware. As one person.

Not a toy — a kernel that boots on a StarFive VisionFive 2, handles interrupts, talks over UART, and deploys in 30 seconds from code change to running on silicon.

The secret? AI as a co-engineer.

I wrote about what actually works and what doesn't when you pair an AI with bare-metal embedded development:

- It pulled register addresses from 500-page hardware manuals in seconds
- It scaffolded drivers, linker scripts, and build pipelines in minutes
- It drafted entire book chapters while the code was still warm
- It also almost bricked my board when I let it drive flash operations unsupervised

The honest breakdown: an 8x speed multiplier on the tasks AI handles well (information retrieval, pattern code, documentation) — and zero help on the tasks it can't (architecture decisions, physical debugging, knowing when to stop).

The key insight: AI amplifies capability. It doesn't create it. You still need to understand what a PLIC context is. You still need to own the architecture. You still need to be the one who says "no, don't flash that — the binary doesn't match this board revision."

But if you bring the knowledge and the judgment, AI brings the speed. One developer, team-sized output.

I'm documenting the entire journey — code, tutorial book, hardware quirks, and all — in the open:

Blog: [link]
Source: https://github.com/westerngazoo/goose-os
Tutorial book: https://github.com/westerngazoo/TheGooseFactor

Next up: Sv39 virtual memory — the door to userspace and the microkernel architecture.

If you're into RISC-V, embedded Rust, OS internals, or just curious how far AI-assisted development can go — follow along. Questions, pushback, and war stories welcome.

#RISCV #EmbeddedSystems #Rust #AI #OperatingSystems #BareMetal #OpenSource

---

## X (Thread)

**Tweet 1:**
I built a RISC-V OS from scratch as one developer.

Boots on real hardware. Deploys in 30 seconds. Has a companion tutorial book.

The performance enhancer? AI as a co-engineer.

Here's what actually works and what almost bricked my board.

A thread:

**Tweet 2:**
The project: GooseOS — a bare-metal RISC-V kernel in Rust targeting the StarFive VisionFive 2.

Interrupt-driven I/O, platform abstraction, automated deployment, and a full tutorial book.

Solo. Days of active development, not months.

**Tweet 3:**
Where AI crushed it:

- Extracting register addresses from 500-page hardware manuals (seconds, not hours)
- Scaffolding UART drivers, linker scripts, Makefiles
- Debugging "TX works but RX doesn't" → MCR OUT2 bit. A datasheet footnote I'd have missed for hours
- Writing book chapters while the code was fresh

**Tweet 4:**
Where AI almost killed my board:

SPI flash recovery. AI suggested sf erase / sf write commands. Syntactically correct. Wrong SPL binary for my board revision.

Result: SPI boot permanently bricked.

Rule: if the command touches non-volatile storage, YOU verify every parameter.

**Tweet 5:**
The model that works:

You: architecture, priorities, hardware, risk assessment
AI: register lookups, boilerplate, debugging patterns, docs, automation

You are the architect. AI is the construction crew.

Don't ask it what to build. Tell it what to build and why.

**Tweet 6:**
The speed multiplier is real:

Reading a SoC manual for UART regs: 2hrs → 5min
Writing a stride-based UART driver: 3hrs → 20min
Debugging MCR OUT2 on real HW: 4hrs → 15min
Writing one book chapter: 6hrs → 45min

~8x on the right tasks. Same time on the rest.

**Tweet 7:**
AI amplifies capability. It doesn't create it.

If you don't know what a PLIC context is, the right number won't help you debug when it's wrong.

The foundation matters. AI makes building faster, not learning optional.

**Tweet 8:**
Full blog post, source code, and the tutorial book — all open:

Blog: [link]
Code: github.com/westerngazoo/goose-os
Book: github.com/westerngazoo/TheGooseFactor

Next: Sv39 page tables → userspace → microkernel.

Follow @theg00sefactor for the ride.
