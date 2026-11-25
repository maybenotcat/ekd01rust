pub mod data;
pub mod isink;
use self::data::*;
use crate::environment::Trace;
use crate::{mmio::*, registers::*};
use core::marker::PhantomData;

/// PMU calibration state is owned by board power management, never synthesized.
pub struct Pmu<P: Trace> {
    _environment: PhantomData<P>,
    cv_code: Option<usize>,
}
impl<P: Trace> Pmu<P> {
    pub fn initialize() -> Self {
        P::log("[PMU] INIT_ENTER\n");
        let boot = read16(EKD_BOOT_FLAGS) as u32;
        let efuse = read32(EKD_PMU_EFUSE);
        let code = ((efuse >> EKD_EFUSE_CV_CODE_SHIFT) & EKD_EFUSE_CV_CODE_MASK) as usize;
        let pmu = Self {
            _environment: PhantomData,
            cv_code: if efuse & (1 << EKD_EFUSE_CV_VALID_SHIFT) != 0 && code < 27 {
                Some(code)
            } else {
                None
            },
        };
        critical_section::with(|_| {
            if efuse & 4 != 0 {
                update16(
                    TEST_CON1,
                    RG_IBIAS_TRIM_MASK,
                    ((efuse >> 3) & 15) << RG_IBIAS_TRIM_SHIFT,
                );
            }
            for (register, row) in [
                (VRF_CON0, 2),
                (VA_CON0, 2),
                (VIO28_CON0, 3),
                (VUSB_CON0, 0),
                (VRTC_CON0, 3),
                (VMC_CON0, 0),
                (VIO18_CON0, 4),
                (PMU_base + EKD_PMU_TRIM_BANK_220, 5),
            ] {
                pmu.calibrate(register, row);
            }
            if boot & EKD_BOOT_USB_WITHOUT_BATTERY == 0 {
                for &(offset, mask, bits) in STARTUP {
                    update16(PMU_base + offset, mask, bits);
                }
            }
            write16(EKD_ADIE_CLOCK_CLEAR, EKD_ADIE_ISINK_26M as u16);
            update16(TEST_CON0, RG_PWRKEY_RST_EN_MASK, RG_PWRKEY_RST_EN_MASK);
            update16(
                TEST_CON0,
                RG_LNGP_SHUTDOWN_SEL_MASK,
                RG_LNGP_SHUTDOWN_SEL_MASK,
            );
            update16(TEST_CON0, RG_PWRKEY_RST_FUNC_SET_MASK, 0);
            update16(
                TEST_CON0,
                RG_PWRKEY_RST_FUNC_SET_MASK,
                RG_PWRKEY_RST_FUNC_SET_MASK,
            );
            update16(TEST_CON0, RG_PWRKEY_RST_FUNC_SET_MASK, 0);
        });
        P::hex("[PMU] BOOT_FLAGS", boot);
        P::hex("[PMU] EFUSE", efuse);
        P::log("[PMU] INIT_EXIT\n");
        pmu
    }
    fn calibrate(&self, register: u32, row: usize) {
        if let Some(code) = self.cv_code {
            update16(
                register + 4,
                LDO_CAL_MASK,
                CALIBRATION[row][code] << LDO_CAL_SHIFT,
            );
        }
    }
    pub fn enable_lcd_supply(&mut self) {
        critical_section::with(|_| {
            update16(VSIM1_CON0, LDO_BUCK_VOL_SEL_MASK, 0);
            self.calibrate(VSIM1_CON0, 5);
            update16(VSIM1_CON2, VSIM1_GPLDO_EN_MASK, VSIM1_GPLDO_EN_MASK);
            update16(VSIM1_CON0, RG_VSIM1_EN_MASK, RG_VSIM1_EN_MASK);
        });
        P::hex("[PMU] VSIM1_CON0", read16(VSIM1_CON0) as u32);
    }
    pub fn power_pressed(&self) -> bool {
        read16(EKD_ADIE_POWER_KEY_STATUS) as u32 & EKD_ADIE_POWER_KEY_RELEASED == 0
    }
}
