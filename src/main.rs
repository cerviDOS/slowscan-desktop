#![allow(unused)]

use std::f32::consts::PI;
use std::fs::File;
use std::iter::zip;
use std::ops::Div;
use iced::advanced::graphics::core::window;
use iced::advanced::graphics::text::cosmic_text::skrifa::raw::types::newtype_scalar;
use iced::task::{Sipper, sipper};
use iced::{Application, Program, Task};
use iced::widget::{button, column, image, row, text_input};
use iced::Element;
use bytes::Bytes;
use itertools::Itertools;
use num::{Complex, iter};
use rand::RngExt;
use rand::seq::index::sample;
use rodio::{Decoder, Source};
use rustfft::{FftPlanner, num_traits};

enum SSTVMode {
    MartinM1
}

struct SSTVSignature {
    hsync_pulse_duration_ms: f32,
    hsync_porch_duration_ms: f32,
    color_scan_duration_ms: f32,
    color_separator_duration_ms: f32,
    ms_per_pixel: f32,
    // vis code
    // hsync length
    // color scan length
    // color order
}

const HSYNC_HZ: u32 = 1200;
const COLOR_LOW_HZ: u32 = 1500;
const COLOR_HIGH_HZ: u32 = 2300;

const HSYNC_FREQUENCY_TOLERANCE_HZ: u32 = 50;
const HSYNC_TIMING_TOLERANCE_MS: f32 = 0.5;

const NUM_SCANLINES: usize = 256;
const PIXELS_PER_SCANLINE: usize = 320;

const MARTIN_M1_SIG: SSTVSignature = SSTVSignature {
    hsync_pulse_duration_ms: 4.862,
    hsync_porch_duration_ms: 0.572,
    color_scan_duration_ms: 146.432,
    color_separator_duration_ms: 0.572,
    ms_per_pixel: 0.4576
};

#[derive(PartialEq, Clone, Copy, Debug)]
enum ColorChannel {
    Red,
    Green,
    Blue
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum DecoderState {
    AwaitingHSync,
    WithinHSync,
    Decoding(ColorChannel)
}

struct SSTVDecoder {
    scanline: [PixelRGBA; 320],

    state: DecoderState,
    state_duration_samples: u32,

    sample_rate: u32,

    curr_pixel: usize,

    holdover_frequencies: Vec<f32>
}

impl SSTVDecoder {

    pub fn new(sample_rate: u32) -> Self {
        Self {
            scanline: [PixelRGBA::default(); 320],
            state: DecoderState::AwaitingHSync,
            state_duration_samples: 0,
            sample_rate,
            curr_pixel: 0,
            holdover_frequencies: Vec::new()
        }
    }

    pub fn reset(&mut self) {
        //self.scanline.clear();
        self.change_state(DecoderState::AwaitingHSync);
        self.curr_pixel = 0;
    }

    pub fn set_sample_rate(&mut self, new_sample_rate: u32) {
        self.sample_rate = new_sample_rate;
        self.reset();
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
                    MARTIN_M1_SIG.hsync_pulse_duration_ms,
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
        let samples_per_pixel = ((MARTIN_M1_SIG.ms_per_pixel / 1000.0) * self.sample_rate as f32) as usize;

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

                if self.curr_pixel == PIXELS_PER_SCANLINE {
                    self.change_state(next_state);

                    if next_state == DecoderState::AwaitingHSync {

                        let mut offset = (chunk_num * samples_per_pixel) + samples_per_pixel;
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

    pub fn process(&mut self, frequencies: &[f32]) -> Option<[PixelRGBA; 320]> {
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
            Some(self.scanline)
        } else {
            None
        }
    }
}

const FFT_SIZE: usize = 1024;

fn compute_analytic_signal(waveform_complex: &mut [Complex<f32>; FFT_SIZE]) {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    fft.process(waveform_complex);

    // Thank you DSP StackExchange user Cesar, who disappeared
    // from the web prompty after this explanation.
    // https://dsp.stackexchange.com/a/63772

    let nyquist = FFT_SIZE / 2;

    // Zero out bins at 0
    waveform_complex[0].re = 0.0;
    waveform_complex[0].im = 0.0;

    // Multiply 1 to N/2 by 2
    for val in waveform_complex.iter_mut().take(nyquist) {
        val.re *= 2.0;
        val.im *= 2.0;
    }

    // Zero out N/2 + 1 to N - 1
    for val in waveform_complex.iter_mut().take(FFT_SIZE).skip(nyquist+1) {
        val.re = 0.0;
        val.im = 0.0;
    }

    let fft_inv = planner.plan_fft_inverse(FFT_SIZE);
    fft_inv.process(waveform_complex);

    for x in waveform_complex.iter_mut() {
        *x = x.div(FFT_SIZE as f32);
    }
}

fn inst_frequency(a: Complex<f32>, b: Complex<f32>, sample_rate: u32) -> f32 {
    // Phase unwrapping using integer overflows, neat!
    // https://www.site2241.net/march2025.htm
    let phase_a = ((a.arg() / PI) * i16::MAX as f32) as i16;
    let phase_b = ((b.arg() / PI) * i16::MAX as f32) as i16;

    let phase_diff_i16 = (phase_b.wrapping_sub(phase_a));

    let phase_diff_f32= (phase_diff_i16 as f32 / i16::MAX as f32);

    (phase_diff_f32 / 2.0) * sample_rate as f32
}

fn apply_window_fn(waveform: &mut [f32; FFT_SIZE]) {
    // TODO: apply windowing function
    let window_type = windowfunctions::WindowFunction::BlackmanHarris;
    let symmetry = windowfunctions::Symmetry::Symmetric;
    let window_fn = windowfunctions::window::<f32>(FFT_SIZE,
        window_type,
        symmetry);

    for (sample, window_term) in zip(waveform, window_fn) {
        *sample *= window_term;
    }
}

fn demodulate_frequencies(waveform: &[f32; FFT_SIZE], sample_rate: u32) -> Vec<f32> {
    let mut windowed_waveform = *waveform;
    apply_window_fn(&mut windowed_waveform);

    let mut waveform_complex = windowed_waveform.iter()
        .map_into::<Complex<f32>>()
        .collect_vec();

    compute_analytic_signal(
        waveform_complex.as_mut_slice()
            .try_into()
            .unwrap()
    );

    let mut frequencies = Vec::new();

    for (idx, pair) in waveform_complex.windows(2).enumerate() {
        frequencies.push(inst_frequency(pair[0], pair[1], sample_rate));
    }

    frequencies
}

#[derive(Clone, Copy)]
pub struct PixelRGBA {
    r: u8,
    g: u8,
    b: u8,
    a: u8
}

#[derive(Clone)]
pub enum Message {
    FilePathChanged(String),
    StartDecode,
    DecodeProgress(Box<[PixelRGBA; 320]>),
    DecodeComplete(Result<(), ()>)
}

const WIDTH: usize = 320;
const HEIGHT: usize = 256;

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

struct SlowScan {
    filepath: String,
    pixels: Vec<PixelRGBA>,
    curr_scanline: usize,
    sstv_decoder: SSTVDecoder
}

impl SlowScan {

    pub fn new() -> Self {
        Self {
            filepath: String::from(""),
            pixels: vec![PixelRGBA::default(); WIDTH * HEIGHT],
            curr_scanline: 0,
            sstv_decoder: SSTVDecoder::new(44100)
        }
    }

    pub fn title(&self) -> String {
        String::from("meowy interesting")
    }

    pub fn view(&self) -> Element<'_, Message> {
        let handle = iced::advanced::image::Handle::from_rgba(
            WIDTH as u32,
            NUM_SCANLINES as u32,
            self.pixels_to_bytes());

        column![
            row![
                text_input("enter file path of signal", &self.filepath)
                    .on_input(Message::FilePathChanged),
                button("decode").on_press(Message::StartDecode)
            ],
            image(handle)
        ].into()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FilePathChanged(new_path) => {
                self.filepath = new_path;
                Task::none()
            }
            Message::StartDecode => {
                let file = File::open(&self.filepath);

                if file.is_err() {
                    println!("File at path \"{}\" does not exist", self.filepath);
                    return Task::none();
                }
                let file = file.unwrap();

                Task::sip(
                    SlowScan::decode(file),
                    Message::DecodeProgress,
                    Message::DecodeComplete
                )
            }
            Message::DecodeProgress(scanline) => {
                self.update_scanline(&scanline);
                Task::none()
            }
            Message::DecodeComplete(_) => {
                self.curr_scanline = 0;
                println!("Decoding complete.");
                Task::none()
            }
        }
    }

    fn decode(file: File) -> impl Sipper<Result<(),()>, Box<[PixelRGBA; 320]>> {
        // TODO: validate filepath before calling fn

        let decoder = Decoder::try_from(file).unwrap();
        let sample_rate = decoder.sample_rate();
        let mut waveform = decoder.collect_vec();

        let mut sstv_decoder = SSTVDecoder::new(sample_rate.into());

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

        sipper(async move |mut sender| {
            for samples in waveform.array_windows::<FFT_SIZE>().step_by(stride) {
                let frequencies = demodulate_frequencies(samples, sample_rate.into());

                let scanline = sstv_decoder.process(&frequencies[trim_size..FFT_SIZE-trim_size]);
                if let Some(scanline) = scanline {
                    sender.send(Box::new(scanline)).await;
                }
            }

            Ok(())
        })
    }

    fn update_scanline(&mut self, new_scanline: &[PixelRGBA; 320]) {
        println!("Updating scanline {}...", self.curr_scanline);
        if self.curr_scanline >= HEIGHT {
            return;
        }

        let scanline_start = PIXELS_PER_SCANLINE * self.curr_scanline;
        let scanline_end = scanline_start + PIXELS_PER_SCANLINE;

        let scanline = &mut self.pixels[scanline_start..scanline_end];

        for (pixel_to_update, new_pixel) in scanline.iter_mut().zip(new_scanline) {
            *pixel_to_update = *new_pixel;
        }

        self.curr_scanline += 1;
    }

    fn pixels_to_bytes(&self) -> Bytes {
        let mut pixels_split: Vec<u8> = Vec::new();
        for pixel in self.pixels.clone() {
            pixels_split.append(&mut pixel.into());
        }

        Bytes::from(pixels_split)
    }
}

fn init_app() -> Application<impl Program> {
    iced::application(SlowScan::new, SlowScan::update, SlowScan::view)
        .title(SlowScan::title)
        .window_size((640, 480))
}

fn main() -> iced::Result {
    init_app().run()
}
