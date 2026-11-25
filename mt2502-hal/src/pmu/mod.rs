//! MT2502 PMU calibration and supply mechanisms, independent of board policy.
pub mod control;
pub mod data;
pub mod isink;
use crate::{mmio::*, registers::*};
use data::CALIBRATION;

pub struct Pmu {
    efuse: u32,
    cv_code: Option<usize>,
}
impl Pmu {
    pub fn load_calibration() -> Self {
        let efuse = read32(EKD_PMU_EFUSE);
        let code = ((efuse >> EKD_EFUSE_CV_CODE_SHIFT) & EKD_EFUSE_CV_CODE_MASK) as usize;
        Self {
            efuse,
            cv_code: if efuse & (1 << EKD_EFUSE_CV_VALID_SHIFT) != 0 && code < 27 {
                Some(code)
            } else {
                None
            },
        }
    }
    pub fn efuse(&self) -> u32 {
        self.efuse
    }
    pub fn apply_calibration(&self) {
        if self.efuse & 4 != 0 {
            update16(
                TEST_CON1,
                RG_IBIAS_TRIM_MASK,
                ((self.efuse >> 3) & 15) << RG_IBIAS_TRIM_SHIFT,
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
            self.calibrate(register, row);
        }
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
    pub fn enable_vsim1(&mut self, voltage_selection: u32) {
        critical_section::with(|_| {
            update16(VSIM1_CON0, LDO_BUCK_VOL_SEL_MASK, voltage_selection);
            self.calibrate(VSIM1_CON0, 5);
            update16(VSIM1_CON2, VSIM1_GPLDO_EN_MASK, VSIM1_GPLDO_EN_MASK);
            update16(VSIM1_CON0, RG_VSIM1_EN_MASK, RG_VSIM1_EN_MASK);
        });
    }
    pub fn vsim1_control(&self) -> u16 {
        read16(VSIM1_CON0)
    }
    pub fn power_pressed(&self) -> bool {
        read16(EKD_ADIE_POWER_KEY_STATUS) as u32 & EKD_ADIE_POWER_KEY_RELEASED == 0
    }
}
