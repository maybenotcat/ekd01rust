OUTPUT_FORMAT("elf32-littlearm")
OUTPUT_ARCH(arm)
ENTRY(ekd_entry)
MEMORY {
    FLASH (rx) : ORIGIN = 0x10050520, LENGTH = 0x4afae0
    SRAM (rwx) : ORIGIN = 0x70008000, LENGTH = 0x1000
    PSRAM (rw) : ORIGIN = 0x00100000, LENGTH = 0x5e0000
}
SECTIONS {
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { *(.text .text.*) } > FLASH
    .rodata : { *(.rodata .rodata.*) } > FLASH
    .ramfunc : { . = ALIGN(4); __ramfunc_start = .; *(.ramfunc .ramfunc.*); . = ALIGN(4); __ramfunc_end = .; } > SRAM AT> FLASH
    __ramfunc_load = LOADADDR(.ramfunc);
    .data : { . = ALIGN(8); __data_start = .; *(.data .data.*); . = ALIGN(4); __data_end = .; } > PSRAM AT> FLASH
    __data_load = LOADADDR(.data);
    __flash_image_end = LOADADDR(.data) + SIZEOF(.data);
    .bss (NOLOAD) : { . = ALIGN(8); __bss_start = .; *(.bss .bss.* COMMON); . = ALIGN(8); __bss_end = .; } > PSRAM
    .heap (NOLOAD) : { . = ALIGN(8); __heap_start = .; . += 0x200000; __heap_end = .; } > PSRAM
    __irq_stack_top = 0x70009800;
    __stack_top = 0x006f0000;
    ASSERT(SIZEOF(.vectors) == 64, "ARM vectors must contain eight absolute targets")
    ASSERT(__flash_image_end <= 0x10500000, "Image overlaps private settings partition")
    ASSERT(__heap_end <= 0x006e0000, "Heap overlaps application stack")
    /* panic=abort: no stack unwinder. PREL31 tables cannot span the >1GiB
     * distance between XIP and the original internal SRAM execution window. */
    /DISCARD/ : { *(.comment .note .note.* .ARM.exidx* .ARM.extab*) }
}
