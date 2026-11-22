// Original N22 calibration and startup register data.
use super::registers::*;
pub const CALIBRATION: [[u32; 27]; 6] = [
    [
        0, 0, 0, 0, 0, 0, 10, 12, 13, 14, 14, 15, 15, 0, 0, 0, 1, 1, 2, 2, 3, 4, 5, 7, 8, 8, 0,
    ],
    [
        0, 0, 0, 0, 0, 8, 10, 12, 13, 14, 14, 15, 15, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 0,
    ],
    [
        0, 0, 0, 0, 0, 10, 12, 13, 14, 14, 15, 15, 15, 0, 0, 0, 1, 1, 1, 2, 2, 3, 3, 5, 5, 6, 0,
    ],
    [
        0, 0, 0, 0, 0, 9, 11, 13, 13, 14, 14, 15, 15, 0, 0, 0, 1, 1, 2, 2, 2, 3, 4, 6, 6, 7, 0,
    ],
    [
        0, 0, 0, 0, 9, 11, 13, 14, 14, 15, 15, 15, 15, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 4, 4, 5, 0,
    ],
    [
        0, 0, 0, 0, 9, 12, 13, 14, 14, 15, 15, 15, 15, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 4, 4, 5, 0,
    ],
];
pub const STARTUP: &[(u32, u32, u32)] = &[
    (PMU_REG_OFFSET(LDO_CON0), EKD_LDO_CON0_STARTUP_BIT, 0),
    (
        PMU_REG_OFFSET(VIBR_CON0),
        RG_VIBR_STB_SEL_MASK,
        RG_VIBR_STB_SEL_MASK,
    ),
    (
        PMU_REG_OFFSET(VMC_CON0),
        RG_VMC_STB_SEL_MASK,
        RG_VMC_STB_SEL_MASK,
    ),
    (
        PMU_REG_OFFSET(VCORE_CON3),
        VCORE_VOSEL_SLEEP_MASK,
        (18 << VCORE_VOSEL_SLEEP_SHIFT),
    ),
    (
        PMU_REG_OFFSET(VCORE_CON3),
        VCORE_SFSTREN_MASK,
        VCORE_SFSTREN_MASK,
    ),
    (
        PMU_REG_OFFSET(CHR_CON0),
        RG_VCDT_HV_VTH_MASK,
        (11 << RG_VCDT_HV_VTH_SHIFT),
    ),
    (
        PMU_REG_OFFSET(CHR_CON0),
        RG_VCDT_HV_EN_MASK,
        RG_VCDT_HV_EN_MASK,
    ),
    (
        PMU_REG_OFFSET(CHR_CON1),
        RG_VBAT_CC_VTH_MASK,
        (2 << RG_VBAT_CC_VTH_SHIFT),
    ),
    (PMU_REG_OFFSET(CHR_CON4), RG_CSDAC_STP_MASK, 0),
    (
        PMU_REG_OFFSET(CHR_CON5),
        RG_BATON_TDET_EN_MASK,
        RG_BATON_TDET_EN_MASK,
    ),
    (
        PMU_REG_OFFSET(CHR_CON5),
        RG_VBAT_OV_VTH_MASK,
        (1 << RG_VBAT_OV_VTH_SHIFT),
    ),
    (
        PMU_REG_OFFSET(CHR_CON9),
        RG_CHRWDT_TD_MASK,
        (3 << RG_CHRWDT_TD_SHIFT),
    ),
    (PMU_REG_OFFSET(CHR_CON10), RG_ADCIN_CHR_EN_MASK, 0),
    (PMU_REG_OFFSET(CHR_CON10), RG_ADCIN_VSEN_EN_MASK, 0),
    (PMU_REG_OFFSET(CHR_CON10), RG_ADCIN_VBAT_EN_MASK, 0),
    (
        PMU_REG_OFFSET(CHR_CON11),
        RG_BC11_RST_MASK,
        RG_BC11_RST_MASK,
    ),
    (
        PMU_REG_OFFSET(CHR_CON11),
        RG_BC11_BB_CTRL_MASK,
        RG_BC11_BB_CTRL_MASK,
    ),
    (
        PMU_REG_OFFSET(CHR_CON12),
        RG_ULC_DET_EN_MASK,
        RG_ULC_DET_EN_MASK,
    ),
    (PMU_REG_OFFSET(CHR_CON12), RG_HWCV_EN_MASK, RG_HWCV_EN_MASK),
    (
        PMU_REG_OFFSET(CHR_CON12),
        RG_CSDAC_MODE_MASK,
        RG_CSDAC_MODE_MASK,
    ),
    (
        PMU_REG_OFFSET(STRUP_CON0),
        THR_HWPDN_EN_MASK,
        THR_HWPDN_EN_MASK,
    ),
    (
        PMU_REG_OFFSET(STRUP_CON1),
        RG_PMU_PGDET_DIS_MASK,
        RG_PMU_PGDET_DIS_MASK,
    ),
    (
        PMU_REG_OFFSET(SPK_CON3),
        RG_SPK_IN_FLOAT_B_MASK,
        RG_SPK_IN_FLOAT_B_MASK,
    ),
    (
        PMU_REG_OFFSET(SPK_CON3),
        RG_SPK_OUT_FLOAT_B_MASK,
        RG_SPK_OUT_FLOAT_B_MASK,
    ),
    (
        PMU_REG_OFFSET(SPK_CON6),
        RG_SPK_RSV_MASK,
        (8 << RG_SPK_RSV_SHIFT),
    ),
];
