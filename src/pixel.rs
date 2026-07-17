#[derive(Clone, Copy)]
pub struct PixelRGBA {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8
}

impl From<PixelRGBA> for Vec<u8> {
    fn from(pixel: PixelRGBA) -> Vec<u8> {
        vec![pixel.r, pixel.g, pixel.b, pixel.a]
    }
}

impl Default for PixelRGBA {
    fn default() -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a: 255
        }
    }
}

