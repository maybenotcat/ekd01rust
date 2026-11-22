//! Original stop-and-wait block transfer, including complete post-write readback.
use super::settings::{BlockId, ParameterBlock, Snapshot};
use crate::protocol::CanFrame;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferStatus {
    #[default]
    Idle,
    Reading,
    Ready,
    Writing,
    Verifying,
    Verified,
    Timeout,
    BadBlock,
    Mismatch,
    Offline,
    Neutral,
    Busy,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestResult {
    #[default]
    Accepted,
    Busy,
    Offline,
    NeedsNeutral,
    Invalid,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    ReadHeader,
    ReadData,
    WriteHeaderAck,
    WriteChunkAck,
    WriteFinalAck,
}
#[derive(Clone, Copy, Default)]
pub struct AdvancedView {
    pub limits: Snapshot,
    pub start: Snapshot,
    pub state: TransferStatus,
    pub progress: usize,
    pub block_id: Option<BlockId>,
    pub request_result: RequestResult,
    pub requested_block: Option<BlockId>,
}
#[derive(Default)]
pub struct BlockTransfer {
    pub view: AdvancedView,
    incoming: ParameterBlock,
    outgoing: ParameterBlock,
    phase: Phase,
    bytes: usize,
    sequence: u32,
    verify: bool,
    activity: u32,
    tx: Option<CanFrame>,
}
impl BlockTransfer {
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle || self.tx.is_some()
    }
    fn finish(&mut self, state: TransferStatus) {
        self.phase = Phase::Idle;
        self.tx = None;
        self.view.state = state;
    }
    fn queue(&mut self, id: u32, data: &[u8]) {
        self.tx = CanFrame::new(id, data);
    }
    fn snapshot(&self, id: BlockId) -> &Snapshot {
        if id == BlockId::StartResponse {
            &self.view.start
        } else {
            &self.view.limits
        }
    }
    fn progress_reset(&mut self, now: u32) {
        self.bytes = 0;
        self.sequence = 0;
        self.activity = now;
        self.view.progress = 0;
    }
    pub fn read(&mut self, id: BlockId, now: u32) {
        if self.busy() {
            return;
        }
        self.view.block_id = Some(id);
        self.verify = false;
        self.progress_reset(now);
        self.phase = Phase::ReadHeader;
        self.view.state = TransferStatus::Reading;
        self.queue(0x03110000 | id as u32, &[]);
    }
    pub fn apply(&mut self, id: BlockId, draft: &ParameterBlock, revision: u32, now: u32) -> bool {
        let original = self.snapshot(id);
        if self.busy() || !original.valid || revision != original.revision {
            return false;
        }
        let Some(block) = original.block.prepare(id, draft) else {
            return false;
        };
        self.outgoing = block;
        self.view.block_id = Some(id);
        self.progress_reset(now);
        self.phase = Phase::WriteHeaderAck;
        self.view.state = TransferStatus::Writing;
        self.queue(0x03100000 | id as u32, &[64]);
        true
    }
    fn write_chunk(&mut self) {
        let id = self.view.block_id.unwrap();
        let last = self.bytes == 56;
        let frame_id = if self.bytes == 0 {
            0x03140000 | id as u32
        } else {
            let sequence = self.sequence;
            self.sequence += 1;
            (if last { 0x03160000 } else { 0x03150000 }) | sequence
        };
        self.phase = if last {
            Phase::WriteFinalAck
        } else {
            Phase::WriteChunkAck
        };
        self.tx = CanFrame::new(frame_id, &self.outgoing.bytes[self.bytes..self.bytes + 8]);
        self.bytes += 8;
        self.view.progress = self.bytes;
    }
    fn accept_block(&mut self) -> bool {
        let id = self.view.block_id.unwrap();
        if !self.incoming.valid(id) {
            self.finish(TransferStatus::BadBlock);
            return false;
        }
        let matches = self.incoming == self.outgoing;
        let snapshot = if id == BlockId::StartResponse {
            &mut self.view.start
        } else {
            &mut self.view.limits
        };
        snapshot.block = self.incoming;
        snapshot.valid = true;
        snapshot.revision = snapshot.revision.wrapping_add(1);
        self.finish(if self.verify {
            if matches {
                TransferStatus::Verified
            } else {
                TransferStatus::Mismatch
            }
        } else {
            TransferStatus::Ready
        });
        true
    }
    pub fn receive(&mut self, frame: &CanFrame, now: u32) -> bool {
        if self.tx.is_some() {
            return false;
        }
        let Some(id) = self.view.block_id else {
            return false;
        };
        match self.phase {
            Phase::WriteHeaderAck | Phase::WriteChunkAck | Phase::WriteFinalAck => {
                if frame.id != (0x02fa0000 | id as u32) || frame.length != 0 {
                    return false;
                }
                self.activity = now;
                if self.phase != Phase::WriteFinalAck {
                    self.write_chunk();
                } else {
                    self.verify = true;
                    self.progress_reset(now);
                    self.phase = Phase::ReadHeader;
                    self.view.state = TransferStatus::Verifying;
                    self.queue(0x03110000 | id as u32, &[]);
                }
                true
            }
            Phase::ReadHeader => {
                if frame.id != (0x02fc0000 | id as u32) {
                    return false;
                }
                if frame.length != 1 || frame.data[0] != 64 {
                    self.finish(TransferStatus::BadBlock);
                    return false;
                }
                self.progress_reset(now);
                self.phase = Phase::ReadData;
                self.queue(0x03120000 | id as u32, &[]);
                true
            }
            Phase::ReadData => {
                let kind = (frame.id >> 16) & 7;
                if frame.id & 0xfff80000 != 0x02f80000 || !matches!(kind, 5 | 6) {
                    return false;
                }
                if frame.id & 0xffff != self.sequence || frame.length != 8 || self.bytes + 8 > 64 {
                    self.finish(TransferStatus::BadBlock);
                    return false;
                }
                self.incoming.bytes[self.bytes..self.bytes + 8].copy_from_slice(&frame.data);
                self.bytes += 8;
                self.sequence += 1;
                self.activity = now;
                self.view.progress = self.bytes;
                let final_chunk = kind == 6;
                if final_chunk != (self.bytes == 64) {
                    self.finish(TransferStatus::BadBlock);
                    return false;
                }
                if final_chunk && !self.accept_block() {
                    return false;
                }
                self.queue(0x03120000 | id as u32, &[]);
                true
            }
            Phase::Idle => false,
        }
    }
    pub fn pending(&self) -> Option<CanFrame> {
        self.tx
    }
    pub fn sent(&mut self, now: u32) {
        self.tx = None;
        self.activity = now;
    }
    pub fn poll(&mut self, now: u32) {
        if self.busy() && now.wrapping_sub(self.activity) >= 256 {
            self.finish(TransferStatus::Timeout);
        }
    }
    pub fn cancel(&mut self) {
        self.finish(TransferStatus::Idle);
        self.view.start.valid = false;
        self.view.limits.valid = false;
    }
}
