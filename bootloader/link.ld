OUTPUT_FORMAT("elf32-littlearm")
OUTPUT_ARCH(arm)
ENTRY(boot_entry)
MEMORY {
    /* Original BL SRAM load address; original SDK reserves 0x9000..0xa000
       for its stack. No external memory is used before the PSRAM probe. */
    SRAM (rwx) : ORIGIN = 0x70006140, LENGTH = 0x2ec0
}
SECTIONS {
    .vectors : { KEEP(*(.vectors)) } > SRAM
    .text : { *(.text .text.*) *(.ramfunc .ramfunc.*) } > SRAM
    .rodata : { *(.rodata .rodata.*) } > SRAM
    .data : { . = ALIGN(8); *(.data .data.*); . = ALIGN(4); } > SRAM
    __boot_load_end = .;
    ASSERT(__boot_load_end + 32 <= 0x70009000, "Bootloader including hash overlaps SRAM stack")
    .bss (NOLOAD) : { . = ALIGN(8); __bss_start = .; *(.bss .bss.* COMMON); . = ALIGN(8); __bss_end = .; } > SRAM
    __stack_top = 0x7000a000;
    ASSERT(__bss_end <= 0x70009000, "Bootloader overlaps internal SRAM stack")
    /DISCARD/ : { *(.comment .note .note.* .ARM.exidx* .ARM.extab*) }
}
