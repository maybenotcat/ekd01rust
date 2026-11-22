use crate::types::{Config, Error, Result};

/// Returns one mutable logical block from the in-memory image.
///
/// Writer code should go through this helper instead of slicing directly so
/// corrupt allocator state becomes a normal error instead of a panic.
pub(super) fn image_block_mut(image: &mut [u8], cfg: Config, block: u32) -> Result<&mut [u8]> {
    let block = block as usize;
    if block >= cfg.block_count {
        return Err(Error::OutOfBounds);
    }
    let start = block
        .checked_mul(cfg.block_size)
        .ok_or(Error::InvalidConfig)?;
    let end = start
        .checked_add(cfg.block_size)
        .ok_or(Error::InvalidConfig)?;
    image.get_mut(start..end).ok_or(Error::OutOfBounds)
}

/// Erases one logical block in the in-memory image.
///
/// This matters once `ImageEditor` allocates blocks from an imported image.
/// A block can be unreachable from the current metadata graph but still contain
/// old programmed CTZ bytes. Reusing it without an erase would make the later
/// NOR-style `program` operation compute `old & new` and silently corrupt the
/// new file payload.
pub(super) fn erase_block(image: &mut [u8], cfg: Config, block: u32) -> Result<()> {
    image_block_mut(image, cfg, block)?.fill(0xff);
    Ok(())
}

/// Programs bytes into an erased flash block using NOR-style `old & new`.
///
/// Fresh images are initialized to `0xff`, so this is equivalent to copying for
/// now. Keeping the flash operation explicit helps future append/update tests
/// catch accidental attempts to set a programmed bit back to 1.
pub(super) fn program(block: &mut [u8], off: usize, data: &[u8]) -> Result<()> {
    let end = off.checked_add(data.len()).ok_or(Error::NoSpace)?;
    if end > block.len() {
        return Err(Error::NoSpace);
    }
    for (dst, src) in block[off..end].iter_mut().zip(data) {
        *dst &= *src;
    }
    Ok(())
}
