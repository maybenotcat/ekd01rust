use crate::{mmio::*, registers::*};
pub struct Gpt1;
impl Gpt1 {
    pub fn initialize_128hz() {
        write32(IRQ_MASK_SET0, u32::MAX);
        write32(IRQ_MASK_SET1, u32::MAX);
        let _ = read32(IRQ_MASK0);
        for index in 0..5 {
            write32(IRQ_SEL(index), EKD_IRQ_PATCH_DISABLED);
        }
        write32(IRQ_FSEL0, 0);
        write32(IRQ_FSEL1, 0);
        write32(IRQ_SOFT_CLR0, u32::MAX);
        write32(IRQ_SOFT_CLR1, u32::MAX);
        write16(GPT1_CTRL, 0);
        write16(GPT2_CTRL, 0);
        let _ = read16(GPT_STS);
        write32(IRQ_EOIOH0, u32::MAX);
        write32(IRQ_EOIOH1, u32::MAX);
        write32(IRQ_SENS_SET0, 1 << EKD_IRQ_GPT_LINE);
        write16(GPT1_PRESCALER, GPT_PRESCALER_div2 as u16);
        write16(GPT1_LEN, 128);
        write16(GPT1_CTRL, (GPT_CTRL_AUTOMODE | GPT_CTRL_Enable) as u16);
        write32(IRQ_MASK_CLR0, 1 << EKD_IRQ_GPT_LINE);
    }
    pub fn acknowledge() -> bool {
        let status = read16(GPT_STS);
        write32(IRQ_EOIOH0, 1 << EKD_IRQ_GPT_LINE);
        let _ = read32(IRQ_MASK0);
        status as u32 & GPT_STS_1 != 0
    }
}
