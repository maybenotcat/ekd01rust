use super::*;

struct MetadataRelocationPlan {
    relocated_program_block: u32,
    relocated_active_block: u32,
    relocated_block: Vec<u8>,
    pointer_block: u32,
    pointer_update_block: MetadataProgram,
    erase_pointer_block: bool,
    pointer_update: RelocationPointerUpdate,
    needs_orphan_repair: bool,
}

struct MetadataProgram {
    off: usize,
    data: Vec<u8>,
}

#[derive(Clone)]
enum RelocationPointerUpdate {
    SplitTail {
        predecessor: MetadataPair,
    },
    RootChild {
        parent_pair: MetadataPair,
        entry_id: u16,
        marker: GlobalState,
    },
}

include!("mutable/api.rs");
include!("mutable/file_ops.rs");
include!("mutable/dir_ops.rs");
include!("mutable/global_state.rs");
include!("mutable/entry_ops.rs");
include!("mutable/commit.rs");
include!("mutable/layout.rs");
include!("mutable/streaming.rs");
include!("mutable/helpers.rs");
include!("mutable/tests.rs");
