//! EKD01 partition policy. Trait implementations live in the shared HAL.
use crate::{
    soc::flash::{FlashController, FlashRegion, RegionBounds},
    stop,
};
use ekd01_boot_contract::Partition;

pub fn partition(controller: &FlashController, region: Partition) -> FlashRegion<&FlashController> {
    let bounds = RegionBounds::new(region.start, region.end)
        .unwrap_or_else(|_| stop("[BOOT] PARTITION_FAILED\n"));
    FlashRegion::new(controller, bounds)
}
