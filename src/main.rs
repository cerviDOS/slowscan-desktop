use std::collections::VecDeque;
use std::fs::File;
use std::num::NonZero;
use iced::task::{Sipper, sipper};
use iced::{Application, Program, Task};
use iced::widget::{button, checkbox, column, image, row, text_input};
use iced::Element;
use bytes::{Bytes};
use itertools::Itertools;
use rodio::buffer::SamplesBuffer;
use rodio::microphone::{MicrophoneBuilder};
use rodio::source::EmptyCallback;
use rodio::{Decoder, Player, Source};

mod sstv;
use sstv::SSTVDecoder;

mod pixel;
use pixel::PixelRGBA;

mod demod;
use demod::demodulate_frequencies;

// TODO:
// width and height of the image may change between modes,
// need a better way to handle than defining constants here.

const FFT_SIZE: usize = 1024;

const WIDTH: usize = 320;
const HEIGHT: usize = 256;

#[derive(Clone)]
pub enum Message {
    FilePathChanged(String),
    PlayToggled(bool),
    StartDecode,
    DecodeProgress(Box<[PixelRGBA; WIDTH]>),
    DecodeComplete(Result<(), ()>)
}

struct SlowScan {
    play_while_decoding: bool,
    filepath: String,
    pixels: Vec<PixelRGBA>,
    curr_scanline: usize,
}

impl SlowScan {
    pub fn new() -> Self {
        Self {
            play_while_decoding: false,
            filepath: String::from(""),
            pixels: vec![PixelRGBA::default(); WIDTH * HEIGHT],
            curr_scanline: 0,
        }
    }

    pub fn title(&self) -> String {
        String::from("slowscan")
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
                column![
                    button("decode").on_press(Message::StartDecode),
                    checkbox(self.play_while_decoding)
                        .label("play audio?").on_toggle(Message::PlayToggled),
                ]
            ],
            image(handle),

        ].into()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FilePathChanged(new_path) => {
                self.filepath = new_path;
                Task::none()
            }
            Message::PlayToggled(should_play_audio) => {
                self.play_while_decoding = should_play_audio;
                Task::none()
            }
            Message::StartDecode => {
                //let file = File::open(&self.filepath);

                let file = File::open("sstv_meow.wav");
                if file.is_err() {
                    println!("File at path \"{}\" does not exist", self.filepath);
                    return Task::none();
                }

                let file = file.unwrap();

                if self.play_while_decoding {
                    Task::sip(
                        SlowScan::decode_and_play_file(file),
                        Message::DecodeProgress,
                        Message::DecodeComplete
                    )
                } else {
                    // TODO: Create function for instantaneous decoding,
                    // abstract away decoding logic.
                    Task::none()
                }
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

    fn decode_and_play_file(file: File) -> impl Sipper<Result<(),()>, Box<[PixelRGBA; WIDTH]>> {
        let decoder = Decoder::try_from(file).unwrap();
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();

        // TODO:
        // Stop loading the entire file into memory...
        // Experiment with chunk sizes to balance out
        // performance hit from using tiny chunks.
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
            let handle = rodio::DeviceSinkBuilder::open_default_sink().unwrap();
            let player = Player::connect_new(handle.mixer());

            player.pause();

            let mut scanline_backlog = VecDeque::new();

            let (tx, rx) = flume::bounded(0);

            for samples in waveform.array_windows::<FFT_SIZE>().step_by(stride) {
                let to_play = SamplesBuffer::new(
                    channels,
                    sample_rate,
                    &samples[trim_size..FFT_SIZE-trim_size]);

                player.append(to_play);

                let frequencies = demodulate_frequencies::<FFT_SIZE>(samples, sample_rate.into());

                let scanline = sstv_decoder.process(&frequencies[trim_size..FFT_SIZE-trim_size]);
                if let Some(scanline) = scanline {
                    let scanline = Box::new(scanline);
                    scanline_backlog.push_back(scanline);

                    let tx = tx.clone();
                    let callback = EmptyCallback::new(Box::new(move || {
                        let _ = tx.send(());
                    }));

                    player.append(callback);
                }
            }

            player.play();

            while rx.recv_async().await.is_ok() {
                if let Some(scanline) = scanline_backlog.pop_front() {
                    sender.send(scanline).await;
                }
            }

            player.sleep_until_end();
            Ok(())
        })

    }

    //fn decode_from_microphone() -> impl Sipper<Result<(), ()>, Box<[PixelRGBA; WIDTH]>> {
    fn decode_from_microphone() {
        let mic = MicrophoneBuilder::new()
            .default_device().unwrap()
            .default_config().unwrap()
            .try_channels(NonZero::new(1).unwrap()).unwrap()
            .open_stream().unwrap();

        // FIXME
    }

    fn update_scanline(&mut self, new_scanline: &[PixelRGBA; WIDTH]) {
        println!("Updating scanline {}", self.curr_scanline);
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


