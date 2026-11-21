//! A picture of the game, as the host captured it.

/// One frame of the game picture, RGBA (4 bytes per pixel, rows `w * 4` bytes).
pub struct FrameData {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    /// the host's frame number (pages scripts tell frames apart with it)
    pub seq: u64,
}
