enum SSTVMode {
    MartinM1,
}

enum ScanOrder {
    // FIXME
}

// Probably an anti-pattern, refactor later
pub struct ModeSignature {
    pub num_scanlines: usize,
    pub pixels_per_scanline: usize,
    pub hsync_pulse_ms: f32,
    pub hsync_porch_ms: f32,
    pub color_scan_ms: f32,
    pub color_separator_ms: f32,
    pub ms_per_pixel: f32,
    pub vis: u8,
}

pub const MARTIN_M1: ModeSignature = ModeSignature {
    num_scanlines: 320,
    pixels_per_scanline: 256,
    hsync_pulse_ms: 4.862,
    hsync_porch_ms: 0.572,
    color_scan_ms: 146.432,
    color_separator_ms: 0.572,
    ms_per_pixel: 0.4576,
    vis: 0b0010_1100,

};

pub const MARTIN_M2: ModeSignature = ModeSignature {
    num_scanlines: 320,
    pixels_per_scanline: 256,
    hsync_pulse_ms: 4.862,
    hsync_porch_ms: 0.572,
    color_scan_ms: 73.216,
    color_separator_ms: 0.572,
    ms_per_pixel: 0.2288,
    vis: 0b0010_1000,

};




