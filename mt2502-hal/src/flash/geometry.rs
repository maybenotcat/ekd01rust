//! Separate the flash page boundary from the MT2502 manual-command data budget.
pub const NOR_PAGE_BYTES: usize = 256;
pub const PROGRAM_DATA_MAX: usize = 128;
pub const PROGRAM_COMMAND_BYTES: usize = 4;
pub const SFI_GPRAM_BYTES: usize = 256;

pub fn program_chunk(address: u32, remaining: usize) -> usize {
    // SDK flash_mtd_sf_dal.c uses 128-byte programming for MT2502. A 256-byte
    // payload plus command/address overruns GPRAM and overwrites the opcode.
    let page_remaining = NOR_PAGE_BYTES - (address as usize & (NOR_PAGE_BYTES - 1));
    remaining.min(page_remaining).min(PROGRAM_DATA_MAX)
}
