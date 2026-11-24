.syntax unified
.arch armv5te
.arm
.section .vectors,"ax",%progbits
.global boot_entry
boot_entry:
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    ldr pc, [pc, #24]
    .word boot_reset, boot_undefined, boot_swi, boot_prefetch_abort
    .word boot_data_abort, boot_reserved, boot_irq, boot_fiq
.section .text.boot_reset,"ax",%progbits
boot_reset:
    msr cpsr_c, #0xd3
    ldr sp, =__stack_top
    mov r5, r4
    ldr r0, =__bss_start
    ldr r1, =__bss_end
    mov r2, #0
1:  cmp r0, r1
    strlo r2, [r0], #4
    blo 1b
    mov r0, r5
    bl boot_main
boot_undefined: mov r0, #1
    b boot_exception
boot_swi: mov r0, #2
    b boot_exception
boot_prefetch_abort: mov r0, #3
    b boot_exception
boot_data_abort: mov r0, #4
    b boot_exception
boot_reserved: mov r0, #5
    b boot_exception
boot_irq: mov r0, #6
    b boot_exception
boot_fiq: mov r0, #7
boot_exception:
    mov r1, lr
    mrs r2, spsr
    msr cpsr_c, #0xd3
    ldr sp, =__stack_top
    bl boot_fault
2:  b 2b

.global boot_jump
.type boot_jump,%function
boot_jump:
    msr cpsr_c, #0xd3
    mov r12, r0
    ldr r4, =BOOT_HANDOFF_MAGIC
    mvn r5, r4
    mov r6, #BOOT_HANDOFF_VERSION
    mov r7, r1
    bx r12

.global boot_acquire_irq_mask
.type boot_acquire_irq_mask,%function
boot_acquire_irq_mask:
    mrs r0, cpsr
    orr r1, r0, #0xc0
    msr cpsr_c, r1
    bx lr

.global boot_restore_irq_mask
.type boot_restore_irq_mask,%function
boot_restore_irq_mask:
    msr cpsr_c, r0
    bx lr
