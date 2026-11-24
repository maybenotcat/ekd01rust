//! Original EXT_BL custom_setSFIExt: runtime 70005142, configuration 70005098.
//! N22 COMBO_MEM_SFI entry 2 is selected by JEDEC C8 60 17.
use crate::{
    sfi_profile::{DEVICE_COMMANDS, HW_CONF_130M, JEDEC},
    soc::{
        DebugUart,
        flash::{Flash, transfer_ram},
        mmio::*,
        registers::*,
    },
    stop,
};

pub fn initialize() {
    DebugUart::log("[BOOT] SFI_INIT_ENTER\n");
    // custom_setSFIExt repeats pad/DCM setup after EMI calibration.
    crate::soc::initialize_bus_pads();
    if Flash::initialize().jedec != u32::from_be_bytes([0, JEDEC[0], JEDEC[1], JEDEC[2]]) {
        stop("[BOOT] SFI_UNSUPPORTED_DEVICE\n");
    }
    let saved_mac = read32(RW_SFI_MAC_CTL);
    let saved_direct = read32(RW_SFI_DIRECT_CTL);
    let mode = saved_mac & !SFI_MANUAL_MODE_CLEAR_MASK;

    // DevInit_130M from the original GD25LQ64 entry. Types 1/2 in the SDK
    // command list clear QPI_EN; type 3 sets it before issuing the command.
    // VSREN makes the status-register change volatile, as in the original.
    for &(qpi, command) in DEVICE_COMMANDS {
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
    // The six HWConf_130M words, in the original write order.
    for (register, value) in [
        RW_SFI_MAC_CTL,
        RW_SFI_DIRECT_CTL,
        RW_SFI_MISC_CTL,
        RW_SFI_DLY_CTL,
        RW_SFI_MISC_CTL2,
        RW_SFI_DLY_CTL3,
    ]
    .into_iter()
    .zip(HW_CONF_130M)
    {
        write32(register, value);
    }
    const MCU_BUS_DCM_CONTROL: u32 = 0xa001_0600;
    for offset in [0, 4, 8] {
        update32(MCU_BUS_DCM_CONTROL + offset, 0x0007_0007, HW_CONF_130M[6]);
    }
    // MPLL is 520MHz (0x1401), therefore SFC source is MPLL/4, 130MHz.
    let divider = match (read16(MPLL_CON0) >> 8) & 0x3f {
        0x14 => 1,
        0x18 => 2,
        _ => stop("[BOOT] SFI_PLL_UNSUPPORTED\n"),
    };
    // Original code clears selection in a separate write before setting it.
    update32(CLK_CONDH, 0x0f, 0);
    update32(CLK_CONDH, 0, divider);
    update32(CLK_CONDC, 1, 0);
    update32(CLK_CONDC, 0, 1);

    // This code and its header execute/read from SRAM. Compare the physical
    // boot header after switching QPI and clocks before trusting mapped NOR.
    for offset in (0..0x140).step_by(4) {
        if read32(EKD_NOR_BASE + 0x800 + offset) != read32(0x7000_6000 + offset) {
            DebugUart::hex("[BOOT] SFI_READBACK_OFFSET", offset);
            stop("[BOOT] SFI_READBACK_FAILED\n");
        }
    }
    DebugUart::hex("[BOOT] SFI_DIRECT_CTL", read32(RW_SFI_DIRECT_CTL));
    DebugUart::hex("[BOOT] CLK_CONDC", read32(CLK_CONDC));
    DebugUart::log("[BOOT] SFI_INIT_EXIT\n");
}
