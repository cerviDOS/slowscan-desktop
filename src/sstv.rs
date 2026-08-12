use itertools::Itertools;

use crate::PixelRGBA;
use crate::demod::demodulate_frequencies;

mod decoder;
use decoder::SSTVDecoder;

mod signatures;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SSTVMode {
    MartinM1
}

impl std::fmt::Display for SSTVMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SSTVMode::MartinM1 => {
                f.write_str("Martin M1")
            }
        }
    }
}

pub struct SSTVProgress {

    // samples,
    // sample rate
    // frequencies (fft result),
    // scanline (optional)
    // decoder state
}

pub fn decode<const FFT_SIZE: usize, I, F>(samples: I, sample_rate: u32, mut processing_callback: F) where
    I: IntoIterator<Item = f32>,
    F: FnMut(&[f32], Option<Vec<PixelRGBA>>)
{
    // Computing instantaneous frequency with a hilbert transform introduces
    // edge effects and causes the beginning and end of the returned frequencies
    // to be highly instable.
    //
    // Trimming off the edges will isolate the more accurate frequencies in the
    // center whereas iterating with an overlap of FFT_SIZE - trim_size * 2 will
    // close the gaps caused by removing samples from the end.
    //
    // Trimming a larger chunk off the edges will improve overall accuracy at the cost
    // of repeated computations, though cutting the outermost 4th seems to be the
    // sweet spot.
    let trim_size = FFT_SIZE / 4;
    let stride = FFT_SIZE - trim_size * 2;

    let mut sstv_decoder = SSTVDecoder::new(sample_rate);

    for sample_window in samples.into_iter().array_windows::<FFT_SIZE>().step_by(stride) {
        let frequencies = demodulate_frequencies::<FFT_SIZE>(&sample_window, sample_rate);

        let scanline = sstv_decoder.process(&frequencies[trim_size..FFT_SIZE-trim_size]);
        processing_callback(
            &sample_window[trim_size..FFT_SIZE-trim_size],
            scanline
        );
    }
}
