//! GD25LQ64C driver. Manual SFI transactions must execute entirely from SRAM.
use super::nor_geometry::{
    PROGRAM_COMMAND_BYTES, PROGRAM_DATA_MAX, SFI_GPRAM_BYTES, program_chunk,
};
use super::{DebugUart, mmio::*, registers::*};

pub const CLAIM: u32 = 0x500000;
pub const STORAGE: u32 = 0x501000;
pub const STORAGE_END: u32 = 0x560000;
pub const SECTOR_BYTES: usize = 4096;
const NOR_BASE: u32 = 0x10000000;
const NOR_BYTES: u32 = 0x800000;

#[derive(Default)]
#[repr(C)]
struct ProgramTrace {
    header: u32,
    first_data: u32,
    output_bytes: u32,
    mode: u32,
    status_after: u32,
}
impl ProgramTrace {
    fn report(&self) {
        DebugUart::hex("[FLASH] PROGRAM_GPRAM_HEADER", self.header);
        DebugUart::hex("[FLASH] PROGRAM_GPRAM_DATA", self.first_data);
        DebugUart::hex("[FLASH] PROGRAM_OUTL", self.output_bytes);
        DebugUart::hex("[FLASH] PROGRAM_MODE", self.mode);
        DebugUart::hex("[FLASH] PROGRAM_SR1_AFTER", self.status_after);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    Range,
    UnknownDevice,
    NeedsErase,
    Protected,
    Verify,
    Suspended,
}
pub struct Flash {
    writable: bool,
    pub jedec: u32,
}
impl Flash {
    pub fn initialize() -> Self {
        let jedec = critical_section::with(|_| unsafe {
            operation_ram(0, 0, core::ptr::null(), 0, core::ptr::null_mut())
        }) as u32;
        let magic = read32(NOR_BASE + 0x50000) & 0xffffff;
        DebugUart::hex("[FLASH] JEDEC", jedec);
        let writable = jedec == NOR_JEDEC_GD25LQ64 && magic == 0x4d4d4d;
        Self { writable, jedec }
    }
    pub fn available(&self) -> bool {
        self.writable
    }
    pub fn read(&self, offset: u32, output: &mut [u8]) -> Result<(), FlashError> {
        if offset > NOR_BYTES || output.len() > (NOR_BYTES - offset) as usize {
            return Err(FlashError::Range);
        }
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = read8(NOR_BASE + offset + index as u32);
        }
        Ok(())
    }
    fn check_write(&self, offset: u32, size: usize) -> Result<(), FlashError> {
        if offset < CLAIM || offset > NOR_BYTES || size > (NOR_BYTES - offset) as usize {
            return Err(FlashError::Range);
        }
        if !self.writable {
            return Err(FlashError::UnknownDevice);
        }
        Ok(())
    }
    pub fn program(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        self.check_write(offset, bytes.len())?;
        if let Some((index, &wanted)) = bytes
            .iter()
            .enumerate()
            .find(|(i, b)| read8(NOR_BASE + offset + *i as u32) & **b != **b)
        {
            DebugUart::hex("[FLASH] NEEDS_ERASE_ADDRESS", offset + index as u32);
            DebugUart::hex(
                "[FLASH] EXISTING_REQUESTED",
                ((read8(NOR_BASE + offset + index as u32) as u32) << 8) | wanted as u32,
            );
            return Err(FlashError::NeedsErase);
        }
        let mut written = 0;
        while written < bytes.len() {
            let address = offset + written as u32;
            let count = program_chunk(address, bytes.len() - written);
            let mut page = [0u8; PROGRAM_DATA_MAX];
            page[..count].copy_from_slice(&bytes[written..written + count]);
            let mut trace = ProgramTrace::default();
            let result = critical_section::with(|_| unsafe {
                operation_ram(1, address, page.as_ptr(), count, &mut trace)
            });
            Self::result(result)?;
            for i in 0..count {
                if read8(NOR_BASE + address + i as u32) != page[i] {
                    DebugUart::hex("[FLASH] VERIFY_ADDRESS", address + i as u32);
                    DebugUart::hex(
                        "[FLASH] EXPECTED_ACTUAL",
                        ((page[i] as u32) << 8) | read8(NOR_BASE + address + i as u32) as u32,
                    );
                    trace.report();
                    return Err(FlashError::Verify);
                }
            }
            written += count;
        }
        Ok(())
    }
    pub fn erase(&mut self, offset: u32) -> Result<(), FlashError> {
        self.check_write(offset, SECTOR_BYTES)?;
        if offset & 4095 != 0 {
            return Err(FlashError::Range);
        }
        let result = critical_section::with(|_| unsafe {
            operation_ram(2, offset, core::ptr::null(), 0, core::ptr::null_mut())
        });
        Self::result(result)?;
        if let Some(index) = (0..SECTOR_BYTES).find(|i| read8(NOR_BASE + offset + *i as u32) != 255)
        {
            DebugUart::hex("[FLASH] ERASE_VERIFY_ADDRESS", offset + index as u32);
            DebugUart::hex(
                "[FLASH] ERASE_VERIFY_ACTUAL",
                read8(NOR_BASE + offset + index as u32) as u32,
            );
            return Err(FlashError::Verify);
        }
        Ok(())
    }
    fn result(result: i32) -> Result<(), FlashError> {
        if result == 0 {
            Ok(())
        } else if result == -6 {
            Err(FlashError::Suspended)
        } else {
            Err(FlashError::Protected)
        }
    }
}

#[unsafe(link_section = ".ramfunc")]
#[inline(never)]
fn fatal_ram(code: u32) -> ! {
    // No format strings, lookups, panic paths or non-SRAM calls while NOR is busy.
    for shift in (0..8).rev() {
        let value = (code >> (shift * 4)) & 15;
        let byte = if value < 10 { 48 + value } else { 55 + value };
        for _ in 0..100_000 {
            if read16(UART_LSR(UART1_base)) as u32 & UART_LSR_THRE != 0 {
                write16(UART_THR(UART1_base), byte as u16);
                break;
            }
        }
    }
    loop {
        write16(WDT_RESTART, WDT_RESTART_KEY as u16);
    }
}

#[unsafe(link_section = ".ramfunc")]
#[inline(never)]
unsafe fn transfer_ram(mode: u32, tx: *const u8, out_len: usize, rx: *mut u8, in_len: usize) {
    if out_len > SFI_GPRAM_BYTES || in_len > SFI_GPRAM_BYTES - out_len {
        fatal_ram(8);
    }
    let mut i = 0;
    while i < out_len {
        let mut word = 0;
        let mut j = 0;
        while j < 4 && i + j < out_len {
            word |= (unsafe { tx.add(i + j).read() } as u32) << (8 * j);
            j += 1;
        }
        write32(RW_SFI_GPRAM_DATA + i as u32, word);
        i += 4;
    }
    write32(RW_SFI_MAC_OUTL, out_len as u32);
    write32(RW_SFI_MAC_INL, in_len as u32);
    write32(RW_SFI_MAC_CTL, mode | SFI_MAC_EN);
    let mut budget = 100_000;
    while read32(RW_SFI_MAC_CTL) & SFI_TRANSFER_STATUS_MASK != 0 && budget != 0 {
        budget -= 1;
    }
    if budget == 0 {
        fatal_ram(7);
    }
    update32(RW_SFI_MAC_CTL, 0, SFI_MAC_EN | SFI_TRIG);
    budget = 100_000;
    while read32(RW_SFI_MAC_CTL) & SFI_TRANSFER_STATUS_MASK != SFI_WIP_READY && budget != 0 {
        budget -= 1;
    }
    if budget == 0 {
        fatal_ram(1);
    }
    write32(
        RW_SFI_MAC_CTL,
        (mode | SFI_MAC_EN) & !SFI_TRIGGER_CLEAR_MASK,
    );
    budget = 100_000;
    while read32(RW_SFI_MAC_CTL) & SFI_WIP_READY != 0 && budget != 0 {
        budget -= 1;
    }
    if budget == 0 {
        fatal_ram(2);
    }
    write32(
        RW_SFI_MAC_CTL,
        mode & !(SFI_MAC_SEL | SFI_MAC_SIO_SEL | SFI_MAC_EN | SFI_TRIG),
    );
    budget = 100_000;
    while read32(RW_SFI_MAC_CTL) & SFI_MAC_EN != 0 && budget != 0 {
        budget -= 1;
    }
    if budget == 0 {
        fatal_ram(3);
    }
    let mut i = 0;
    while i < in_len {
        unsafe {
            rx.add(i)
                .write(read8(RW_SFI_GPRAM_DATA + (out_len + i) as u32));
        }
        i += 1;
    }
}

#[unsafe(link_section = ".ramfunc")]
#[inline(never)]
fn ready_ram(mode: u32) -> u8 {
    let command = NOR_CMD_RDSR1 as u8;
    let mut status = NOR_SR1_WIP as u8;
    for _ in 0..10_000_000 {
        unsafe {
            transfer_ram(mode, &command, 1, &mut status, 1);
        }
        if status as u32 & NOR_SR1_WIP == 0 {
            return status;
        }
        write16(WDT_RESTART, WDT_RESTART_KEY as u16);
    }
    fatal_ram(4)
}

#[unsafe(link_section = ".ramfunc")]
#[inline(never)]
unsafe fn operation_ram(
    operation: u32,
    address: u32,
    data: *const u8,
    length: usize,
    trace: *mut ProgramTrace,
) -> i32 {
    let saved = read32(RW_SFI_MAC_CTL);
    let misc = read32(RW_SFI_MISC_CTL3);
    let mut mode = saved & !SFI_MANUAL_MODE_CLEAR_MASK;
    if read32(RW_SFI_DIRECT_CTL) & SFI_QPI_EN != 0 {
        mode |= SFI_MAC_SIO_SEL;
    }
    write32(RW_SFI_MISC_CTL3, misc | SFI_CH2_TRANS_MASK);
    let mut budget = 100_000;
    while read32(RW_SFI_MISC_CTL3) & SFI_CH2_TRANS_IDLE == 0 && budget != 0 {
        budget -= 1;
    }
    if budget == 0 {
        fatal_ram(5);
    }
    budget = 100_000;
    while read32(RW_SFI_MISC_CTL) & SFI_REQ_IDLE == 0 && budget != 0 {
        budget -= 1;
    }
    if budget == 0 {
        fatal_ram(6);
    }
    let mut packet =
        core::mem::MaybeUninit::<[u8; PROGRAM_DATA_MAX + PROGRAM_COMMAND_BYTES]>::uninit();
    let p = packet.as_mut_ptr().cast::<u8>();
    let mut id = [0u8; 3];
    let mut result = 0;
    if operation == 0 {
        unsafe {
            p.write(NOR_CMD_RDID as u8);
            transfer_ram(mode, p, 1, id.as_mut_ptr(), 3);
        }
        result = ((id[0] as u32) << 16 | (id[1] as u32) << 8 | id[2] as u32) as i32;
    } else {
        ready_ram(mode);
        unsafe {
            p.write(NOR_CMD_RDSR2 as u8);
            transfer_ram(mode, p, 1, id.as_mut_ptr(), 1);
        }
        if id[0] as u32 & NOR_SR2_SUSPEND_MASK != 0 {
            result = -6;
        } else {
            unsafe {
                p.write(NOR_CMD_WREN as u8);
                transfer_ram(mode, p, 1, core::ptr::null_mut(), 0);
            }
            if ready_ram(mode) as u32 & NOR_SR1_WEL == 0 {
                result = -4;
            } else {
                unsafe {
                    p.write(if operation == 1 {
                        NOR_CMD_PAGE_PROGRAM as u8
                    } else {
                        NOR_CMD_SECTOR_ERASE as u8
                    });
                    p.add(1).write((address >> 16) as u8);
                    p.add(2).write((address >> 8) as u8);
                    p.add(3).write(address as u8);
                    // Keep LLVM from outlining this loop to an XIP memcpy helper
                    // while the SFI manual/channel-mask state is active.
                    let mut i = 0;
                    while i < length {
                        p.add(i + 4).write_volatile(data.add(i).read_volatile());
                        i += 1;
                    }
                    transfer_ram(mode, p, length + 4, core::ptr::null_mut(), 0);
                    if !trace.is_null() {
                        core::ptr::addr_of_mut!((*trace).header)
                            .write_volatile(read32(RW_SFI_GPRAM_DATA));
                        core::ptr::addr_of_mut!((*trace).first_data)
                            .write_volatile(read32(RW_SFI_GPRAM_DATA + 4));
                        core::ptr::addr_of_mut!((*trace).output_bytes)
                            .write_volatile(read32(RW_SFI_MAC_OUTL));
                        core::ptr::addr_of_mut!((*trace).mode).write_volatile(mode);
                    }
                }
                let status = ready_ram(mode);
                if !trace.is_null() {
                    unsafe {
                        core::ptr::addr_of_mut!((*trace).status_after)
                            .write_volatile(status as u32);
                    }
                }
            }
            unsafe {
                p.write(NOR_CMD_WRDI as u8);
                transfer_ram(mode, p, 1, core::ptr::null_mut(), 0);
            }
        }
    }
    write32(RW_SFI_MAC_CTL, saved & !SFI_MANUAL_MODE_CLEAR_MASK);
    write32(RW_SFI_MISC_CTL3, misc);
    let _ = read32(RW_SFI_MISC_CTL3);
    result
}
