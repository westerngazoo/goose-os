/* GooseOS userspace entry point — netsrv
 *
 * Identical to userspace/hello/src/start.S. Candidate for extraction
 * into userspace/lib/gooseos/start.S once we have two programs that
 * share this file (see follow-up in docs/unsafe-audit.md and Ch 44
 * closing note about the userspace lib split).
 *
 * The kernel sets sp before sret, so no stack setup needed here.
 * We set gp, zero BSS, call main(), then exit.
 */

.section .text.entry
.global _start

_start:
    /* Set global pointer (for efficient sdata/sbss access) */
    .option push
    .option norelax
    la      gp, __global_pointer$
    .option pop

    /* Zero BSS (defensive — kernel zeroes pages, but be safe) */
    la      t0, _bss_start
    la      t1, _bss_end
1:  beq     t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:

    /* Call Rust main() */
    call    main

    /* If main returns, exit with code 0 */
    li      a0, 0
    li      a7, 1           /* SYS_EXIT */
    ecall

    /* Should never reach here */
3:  j       3b
