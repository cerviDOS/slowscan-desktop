#![allow(unused)]

use std::fs::File;
use iced::task::{Sipper, sipper};
use iced::{Application, Program, Task};
use iced::widget::{button, column, image, row, text_input};
use iced::Element;
use bytes::Bytes;
use itertools::Itertools;
use rodio::{Decoder, Source};

mod sstv;
use sstv::SSTVDecoder;

mod pixel;
use pixel::PixelRGBA;

mod demod;
use demod::demodulate_frequencies;

// TODO:
// width and height of the image may change between modes,
// need a better way to handle than defining constants here.

#[derive(Clone)]
pub enum Message {
    FilePathChanged(String),
    StartDecode,
    DecodeProgress(Box<[PixelRGBA; 320]>),
    DecodeComplete(Result<(), ()>)
}

const FFT_SIZE: usize = 1024;

const WIDTH: usize = 320;
const HEIGHT: usize = 256;

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
            HEIGHT as u32,
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

    fn decode(file: File) -> impl Sipper<Result<(),()>, Box<[PixelRGBA; WIDTH]>> {
        // TODO: validate filepath before calling fn

        let decoder = Decoder::try_from(file).unwrap();
        let sample_rate = decoder.sample_rate();
        let waveform = decoder.collect_vec();

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
                let frequencies = demodulate_frequencies::<FFT_SIZE>(samples, sample_rate.into());

                let scanline = sstv_decoder.process(&frequencies[trim_size..FFT_SIZE-trim_size]);
                if let Some(scanline) = scanline {
                    sender.send(Box::new(scanline)).await;
                }
            }

            Ok(())
        })
    }

    fn update_scanline(&mut self, new_scanline: &[PixelRGBA; WIDTH]) {
        if self.curr_scanline >= HEIGHT {
            return;
        }

        let scanline_start = WIDTH * self.curr_scanline;
        let scanline_end = scanline_start + WIDTH;

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
