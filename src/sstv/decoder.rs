use crate::PixelRGBA;
use crate::sstv::SSTVMode;
use crate::sstv::signatures;

const HSYNC_HZ: u32 = 1200;
const COLOR_LOW_HZ: u32 = 1500;
const COLOR_HIGH_HZ: u32 = 2300;

const HSYNC_FREQUENCY_TOLERANCE_HZ: u32 = 50;
const HSYNC_TIMING_TOLERANCE_MS: f32 = 0.5;

const MAX_PIXELS_PER_SCANLINE: usize = 320;

#[derive(PartialEq, Clone, Copy, Debug)]
enum ColorChannel {
    Red,
    Green,
    Blue
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum DecoderState {
    AwaitingHSync,
    WithinHSync,
    Decoding(ColorChannel)
}

pub struct SSTVConfig {
    mode: SSTVMode,
    sample_rate: u32
}

pub struct SSTVDecoder {
    pub signature: signatures::ModeSignature,
    scanline: Vec<PixelRGBA>,
    state: DecoderState,
    state_duration_samples: u32,
    sample_rate: u32,
    curr_pixel: usize,
    holdover_frequencies: Vec<f32>
}

impl SSTVDecoder {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            signature: signatures::MARTIN_M1,
            scanline: vec![PixelRGBA::default(); MAX_PIXELS_PER_SCANLINE],
            state: DecoderState::AwaitingHSync,
            state_duration_samples: 0,
            sample_rate,
            curr_pixel: 0,
            holdover_frequencies: Vec::new()
        }
    }

    pub fn reset(&mut self) {
        self.change_state(DecoderState::AwaitingHSync);
        self.curr_pixel = 0;
    }

    pub fn get_state(&self) -> DecoderState {
        self.state
    }

    fn await_hsync<'a>(&mut self, frequencies: &'a [f32]) -> &'a [f32] {
        for (idx, freq) in frequencies.iter().enumerate() {
            let is_hsync_hz = SSTVDecoder::is_within_tolerance(*freq,
                HSYNC_HZ as f32,
                HSYNC_FREQUENCY_TOLERANCE_HZ as f32);

            if is_hsync_hz {
                self.change_state(DecoderState::WithinHSync);
                return &frequencies[idx..frequencies.len()];
            }
        }

        &[]
    }

    fn verify_hsync<'a>(&mut self, frequencies: &'a [f32]) -> &'a [f32] {
        for (idx, freq) in frequencies.iter().enumerate() {
            let is_hsync_hz = SSTVDecoder::is_within_tolerance(*freq,
                HSYNC_HZ as f32,
                HSYNC_FREQUENCY_TOLERANCE_HZ as f32);

            if !is_hsync_hz {
                let hsync_candidate_duration_ms = (self.state_duration_samples as f32 / self.sample_rate as f32) * 1000.0;

                let is_hsync_duration = SSTVDecoder::is_within_tolerance(hsync_candidate_duration_ms,
                    self.signature.hsync_pulse_ms,
                    HSYNC_TIMING_TOLERANCE_MS);

                if is_hsync_duration {
                    //println!("hsync detected -- {} ms", hsync_candidate_duration_ms);
                    self.change_state(DecoderState::Decoding(ColorChannel::Green));
                } else {
                    self.change_state(DecoderState::AwaitingHSync);
                }
                return &frequencies[idx..frequencies.len()];
            }

            self.state_duration_samples += 1;
        }

        &[]
    }

    fn decode_color_scan<'a>(&mut self, frequencies: &'a [f32]) -> (&'a [f32], bool) {
        let samples_per_pixel = ((self.signature.ms_per_pixel / 1000.0) * self.sample_rate as f32) as usize;

        // TODO:
        // - Handle case where frequencies run out before pixel is finished.
        // - Deal with timing errors between color scans.

        /*
        if !holdover_frequencies.is_empty() {
        ...
        }
        */

        for (chunk_num, pixel_freqs) in frequencies.chunks(samples_per_pixel).enumerate() {
            if pixel_freqs.len() < samples_per_pixel {
                // store frequencies for next call
                //self.holdover_frequencies.copy_from_slice(pixel_freqs);
                return (&[], false);
            }

            let avg_freq = pixel_freqs.iter().sum::<f32>() / samples_per_pixel as f32;
            let decoded_color_val = SSTVDecoder::frequency_to_color_val(avg_freq);

            //println!("pixel={}, state={:?}, freq={}, val={}", self.curr_pixel, self.state, avg_freq, decoded_color_val);
            if let DecoderState::Decoding(curr_channel) = &self.state {
                let pixel = &mut self.scanline[self.curr_pixel];

                let (target_subpixel, next_state) = match curr_channel {
                    ColorChannel::Red => {
                        (&mut pixel.r, DecoderState::AwaitingHSync)
                    }
                    ColorChannel::Green => {
                        (&mut pixel.g, DecoderState::Decoding(ColorChannel::Blue))
                    }
                    ColorChannel::Blue => {
                        (&mut pixel.b, DecoderState::Decoding(ColorChannel::Red))
                    }
                };

                *target_subpixel = decoded_color_val;

                self.curr_pixel += 1;

                if self.curr_pixel == self.signature.pixels_per_scanline {
                    self.change_state(next_state);

                    if next_state == DecoderState::AwaitingHSync {

                        let offset = (chunk_num * samples_per_pixel) + samples_per_pixel;
                        //println!("chunk_num={}, input size={}, offset={}", chunk_num, frequencies.len(), offset);
                        return (&frequencies[offset..], true);
                    }
                }

            }
        }


        (&[], false)
    }

    fn frequency_to_color_val(frequency: f32) -> u8 {
        let color_low_f32 = COLOR_LOW_HZ as f32;
        let color_high_f32 = COLOR_HIGH_HZ as f32;

        let frequency = frequency.clamp(color_low_f32, color_high_f32);

        let ratio = (frequency - color_low_f32) / (color_high_f32 - color_low_f32);

        (ratio * 255.0) as u8
    }

    fn is_within_tolerance<T: num::Num + PartialOrd + Copy>(value: T, target: T, tolerance: T) -> bool {
        value >= (target-tolerance) && value <= (target+tolerance)
    }

    fn change_state(&mut self, new_state: DecoderState) {
        self.state = new_state;
        self.state_duration_samples = 0;
        self.curr_pixel = 0;
    }

    pub fn process(&mut self, frequencies: &[f32]) -> Option<Vec<PixelRGBA>> {
        // functions return the advanced iterator so next loop continues
        // with the remaining values
        let mut frequencies_remaining = frequencies;
        let mut is_scanline_finished = false;
        while !frequencies_remaining.is_empty() {
            match self.state {
                DecoderState::AwaitingHSync => {
                    frequencies_remaining = self.await_hsync(frequencies_remaining);
                    //println!("await hsync: slice remaining: {}", frequencies_remaining.len());
                }
                DecoderState::WithinHSync => {
                    frequencies_remaining = self.verify_hsync(frequencies_remaining);
                    //println!("verify hsync: slice remaining: {}", frequencies_remaining.len());
                }
                DecoderState::Decoding(_) => {
                    (frequencies_remaining, is_scanline_finished) = self.decode_color_scan(frequencies_remaining);
                    //println!("decode: slice remaining: {}", frequencies_remaining.len());
                }
            }
        }

        if is_scanline_finished {
            let scanline_width = self.signature.pixels_per_scanline;

            let mut scanline = Vec::with_capacity(scanline_width);
            scanline.extend_from_slice(&self.scanline[..scanline_width]);

            Some(scanline)
        } else {
            None
        }
    }
}

impl std::fmt::Display for DecoderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecoderState::AwaitingHSync => {
                f.write_str("Awaiting H-Sync")
            }
            DecoderState::WithinHSync => {
                f.write_str("Within H-Sync")
            }
            DecoderState::Decoding(color) => {
                match color {
                    ColorChannel::Red => {
                        f.write_str("Decoding Color Scan (R)")
                    }
                    ColorChannel::Green => {
                         f.write_str("Decoding Color Scan (G)")
                    }
                    ColorChannel::Blue => {
                        f.write_str("Decoding Color Scan (B)")
                    }
                }
            }
        }
    }
}
