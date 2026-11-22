.syntax unified
.arch armv5te
.arm
.section .vectors,"ax",%progbits
.global ekd_entry
.type ekd_entry,%function
ekd_entry:
.rept 8
    ldr pc,[pc,#24]
.endr
    .word reset, undefined, swi, prefetch_abort, data_abort, reserved, irq, fiq
.size ekd_entry,.-ekd_entry

.section .text.startup,"ax",%progbits
reset:
    msr cpsr_c,#0xd2
    ldr sp,=__irq_stack_top
    msr cpsr_c,#0xd3
    ldr sp,=__stack_top
    ldr r0,=0xa0030008
    ldr r1,=0x1971
    strh r1,[r0]
    ldr r0,=__ramfunc_load
    ldr r1,=__ramfunc_start
    ldr r2,=__ramfunc_end
1:  cmp r1,r2
    ldrlo r3,[r0],#4
    strlo r3,[r1],#4
    blo 1b
    ldr r0,=__data_load
    ldr r1,=__data_start
    ldr r2,=__data_end
2:  cmp r1,r2
    ldrlo r3,[r0],#4
    strlo r3,[r1],#4
    blo 2b
    ldr r1,=__bss_start
    ldr r2,=__bss_end
    mov r3,#0
3:  cmp r1,r2
    strlo r3,[r1],#4
    blo 3b
    ldr r0,=ekd_entry
    mov r1,#0
    mov r2,#64
4:  ldr r3,[r0],#4
    str r3,[r1],#4
    subs r2,r2,#4
    bne 4b
    bl firmware_main
5:  b 5b

irq:
    sub lr,lr,#4
    stmdb sp!,{r0-r12,lr}
    mrs r0,spsr
    stmdb sp!,{r0,r1}
    bl irq_dispatch
    ldmia sp!,{r0,r1}
    msr spsr_cxsf,r0
    ldmia sp!,{r0-r12,pc}^

.macro FAULT name,number
\name:
    mov r0,#\number
    mov r1,lr
    mrs r2,spsr
    msr cpsr_c,#0xd3
    ldr sp,=__stack_top
    b fault_report
.endm
FAULT undefined,1
FAULT swi,2
FAULT prefetch_abort,3
FAULT data_abort,4
FAULT reserved,5
FAULT fiq,7
.ltorg
.section .note.GNU-stack,"",%progbits
