pub struct Configuration {
    pub jedec: [u8; 3],
    pub commands: &'static [(bool, &'static [u8])],
    pub hardware: [u32; 7],
}
pub struct Sfi;
use crate::{flash::transfer_ram, mmio::*, registers::*};
impl Sfi {
    /// # Safety
    /// Execute from SRAM with SRAM-resident profile/command data and exclusive SFI ownership.
    pub unsafe fn configure(
        profile: &Configuration,
        jedec: u32,
    ) -> Result<(), crate::error::Error> {
        if jedec != u32::from_be_bytes([0, profile.jedec[0], profile.jedec[1], profile.jedec[2]]) {
            return Err(crate::error::Error::SfiUnsupportedDevice);
        }
        let saved_mac = read32(RW_SFI_MAC_CTL);
        let saved_direct = read32(RW_SFI_DIRECT_CTL);
        let mode = saved_mac & !SFI_MANUAL_MODE_CLEAR_MASK;
        for &(qpi, command) in profile.commands {
            update32(
                RW_SFI_DIRECT_CTL,
                SFI_QPI_EN,
                if qpi { SFI_QPI_EN } else { 0 },
            );
            unsafe {
                transfer_ram(
                    mode | if qpi { SFI_MAC_SIO_SEL } else { 0 },
                    command.as_ptr(),
                    command.len(),
                    core::ptr::null_mut(),
                    0,
                );
            }
        }
        write32(RW_SFI_DIRECT_CTL, saved_direct);
        const RW_SFI_DLY_CTL: u32 = SFI_base + 0x0c;
        const RW_SFI_MISC_CTL2: u32 = SFI_base + 0x24;
        const RW_SFI_DLY_CTL3: u32 = SFI_base + 0x28;
        for (register, value) in [
            RW_SFI_MAC_CTL,
            RW_SFI_DIRECT_CTL,
            RW_SFI_MISC_CTL,
            RW_SFI_DLY_CTL,
            RW_SFI_MISC_CTL2,
            RW_SFI_DLY_CTL3,
        ]
        .into_iter()
        .zip(profile.hardware)
        {
            write32(register, value);
        }
        const MCU_BUS_DCM_CONTROL: u32 = 0xa001_0600;
        for offset in [0, 4, 8] {
            update32(
                MCU_BUS_DCM_CONTROL + offset,
                0x0007_0007,
                profile.hardware[6],
            );
        }
        let divider = match (read16(MPLL_CON0) >> 8) & 0x3f {
            0x14 => 1,
            0x18 => 2,
            _ => return Err(crate::error::Error::SfiPllUnsupported),
        };
        update32(CLK_CONDH, 0x0f, 0);
        update32(CLK_CONDH, 0, divider);
        update32(CLK_CONDC, 1, 0);
        update32(CLK_CONDC, 0, 1);
        Ok(())
    }
}

pub fn direct_control() -> u32 {
    read32(RW_SFI_DIRECT_CTL)
}
