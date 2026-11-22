#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockId {
    StartResponse = 0x6010,
    AssistLimits = 0x6011,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameter {
    Current,
    Speed,
    Start,
}
impl Parameter {
    pub fn offset(self, levels: u8, level: u8) -> Option<usize> {
        if level == 0 || level > levels {
            return None;
        }
        let index = match levels {
            3 => [2, 4, 8][level as usize - 1],
            5 => [1, 3, 5, 7, 8][level as usize - 1],
            9 => level as usize - 1,
            _ => return None,
        };
        Some(
            match self {
                Self::Current => 40,
                Self::Speed => 49,
                Self::Start => 1,
            } + index,
        )
    }
    pub fn maximum(self) -> u8 {
        if self == Self::Start { 8 } else { 100 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParameterBlock {
    pub bytes: [u8; 64],
}
impl Default for ParameterBlock {
    fn default() -> Self {
        Self { bytes: [0; 64] }
    }
}
impl ParameterBlock {
    pub fn checksum(&self) -> u8 {
        self.bytes[..63]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
    }
    pub fn valid(&self, id: BlockId) -> bool {
        id == BlockId::StartResponse || self.checksum() == self.bytes[63]
    }
    pub fn get(&self, parameter: Parameter, levels: u8, level: u8) -> Option<u8> {
        Some(self.bytes[parameter.offset(levels, level)?])
    }
    pub fn set(&mut self, parameter: Parameter, levels: u8, level: u8, value: u8) -> bool {
        let Some(offset) = parameter.offset(levels, level) else {
            return false;
        };
        if value == 0 || value > parameter.maximum() {
            return false;
        }
        self.bytes[offset] = value;
        true
    }
    fn range(id: BlockId) -> core::ops::Range<usize> {
        if id == BlockId::StartResponse {
            1..10
        } else {
            40..58
        }
    }
    pub fn prepare(&self, id: BlockId, draft: &Self) -> Option<Self> {
        let mut out = *self;
        let max = if id == BlockId::StartResponse { 8 } else { 100 };
        for index in Self::range(id) {
            if draft.bytes[index] != self.bytes[index] && !(1..=max).contains(&draft.bytes[index]) {
                return None;
            }
            out.bytes[index] = draft.bytes[index];
        }
        if id == BlockId::AssistLimits {
            out.bytes[63] = out.checksum();
        }
        Some(out)
    }
    pub fn parameters_equal(&self, id: BlockId, other: &Self) -> bool {
        self.bytes[Self::range(id)] == other.bytes[Self::range(id)]
    }
}
#[derive(Clone, Copy, Default)]
pub struct Snapshot {
    pub block: ParameterBlock,
    pub valid: bool,
    pub revision: u32,
}
#[derive(Clone, Copy, Default)]
pub struct Draft {
    pub block: ParameterBlock,
    pub valid: bool,
    pub revision: u32,
    pub dirty: bool,
}
impl Draft {
    pub fn synchronize(
        &mut self,
        confirmed: &Snapshot,
        id: BlockId,
        editing: bool,
        verified: bool,
    ) {
        if !confirmed.valid {
            self.valid = false;
            return;
        }
        if self.dirty
            && verified
            && self.revision != confirmed.revision
            && self.block.parameters_equal(id, &confirmed.block)
        {
            self.dirty = false;
        }
        if editing || self.dirty || self.valid && self.revision == confirmed.revision {
            return;
        }
        self.block = confirmed.block;
        self.valid = true;
        self.revision = confirmed.revision;
    }
}
