use std::collections::VecDeque;
use std::fs::File;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use iced::Center;
use iced::Fill;
use iced::Length::FillPortion;
use iced::task::{Sipper, sipper};
use iced::widget::pick_list;
use iced::{Application, Program, Task, Theme};
use iced::widget::{button, checkbox, column, container, image, radio, row, rule, space, text, text_input};
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
// - Width and height of the image may change between modes, 
// need a better way to handle than defining constants here
// - Stop loading the entire file into memory...
// Experiment with chunk sizes to balance out
// performance hit from using tiny chunks.


const FFT_SIZE: usize = 1024;

const WIDTH: usize = 320;
const HEIGHT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderSource {
    File,
    Microphone,
    Soundcard
}

struct DecoderProgressInfo {
    scanline: Option<Box<[PixelRGBA; WIDTH]>>,
    // TODO:
    //  Time elapsed / Time expected
    //      0m:0s / 1m:55s for file
    //      0m:0s / - for mic
    //  Bytes/s
}

#[derive(Clone)]
enum Message {
    FilePathChanged(String),
    PlayToggled(bool),
    DecoderSourceChanged(DecoderSource),
    ModeChanged(sstv::SSTVMode),
    StartDecode,
    DecodeProgress(Box<[PixelRGBA; WIDTH]>),
    DecodeComplete(Result<(), ()>)
}

struct SlowScan {
    play_while_decoding: bool,

    selected_source: Option<DecoderSource>,
    should_display_file_widgets: bool,
    
    selected_mode: Option<sstv::SSTVMode>,

    filepath: String,
    pixels: Vec<PixelRGBA>,
    curr_scanline: usize,
}

impl SlowScan {
    pub fn new() -> Self {
        Self {
            play_while_decoding: false,

            selected_source: None,
            should_display_file_widgets: false,
            
            selected_mode: None,

            filepath: String::from(""),
            pixels: vec![PixelRGBA::default(); WIDTH * HEIGHT],
            curr_scanline: 0,
        }
    }

    pub fn title(&self) -> String {
        String::from("slowscan")
    }

    pub fn theme(&self) -> Theme {
        Theme::CatppuccinMacchiato
    }

    pub fn view(&self) -> Element<'_, Message> {
        let handle = iced::advanced::image::Handle::from_rgba(
            WIDTH as u32,
            HEIGHT as u32,
            self.pixels_to_bytes());
        
        column![
            row![ // Top panel
                container(column![
                    text("Config").width(Fill).center(),

                    text("Signal Source:"),
                    column![
                        radio("File", DecoderSource::File, self.selected_source, Message::DecoderSourceChanged),
                        (self.should_display_file_widgets).then_some(
                            column![
                                text_input("Path to signal...", &self.filepath)
                                    .on_input(Message::FilePathChanged)
                                    .size(12)
                                    .width(FillPortion(5)),

                                checkbox(self.play_while_decoding)
                                    .label("sync decoding with audio?")
                                    .text_size(12)
                                    .size(12)
                                    .width(Fill)
                                    .on_toggle(Message::PlayToggled),
                            ].padding(5)
                        ),
                    ],

                    row![
                        text("Mode:"),
                        pick_list(
                            [sstv::SSTVMode::MartinM1],
                            self.selected_mode,
                            Message::ModeChanged
                        ).text_size(12)
                    ].align_y(Center)

                ]).width(FillPortion(2)).height(FillPortion(3)),

                rule::vertical(1),

                container(column![
                    text("Status").width(Fill).center(),

                    text("Scanline Progress:").size(12),
                    text(format!("{}/{}", self.curr_scanline, HEIGHT)).size(12),

                    space().height(Fill),

                    container(column![
                        button("decode").on_press(Message::StartDecode),
                        space().height(5)
                    ]
                    ).width(Fill).align_x(Center)
                    ,
                ]).width(FillPortion(1)).height(Fill),

                rule::vertical(1),

                image(handle),
            ],

            rule::horizontal(1),

            row![ // Bottom panel
                // Spectrogram & spectrum
                text("Spectrogram").width(FillPortion(2)).align_y(Center).align_x(Center),
                rule::vertical(1),
                text("Spectrum Analyzer").width(Fill).align_y(Center).align_x(Center),
            ].height(Fill)
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
            Message::DecoderSourceChanged(source) => {
                self.should_display_file_widgets = source == DecoderSource::File;
                self.selected_source = Some(source);
                Task::none()
            }
            Message::ModeChanged(mode) => {
                self.selected_mode = Some(mode);
                Task::none()
            }
            Message::StartDecode => {
                let file = File::open(&self.filepath);
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
                    Task::sip(
                        SlowScan::decode_file(file),
                        Message::DecodeProgress,
                        Message::DecodeComplete
                    )
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

    fn decode<I, F>(samples: I, sample_rate: u32, mut processing_callback: F) where
        I: IntoIterator<Item = f32>,
        F: FnMut(&[f32], Option<[PixelRGBA; WIDTH]>)
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

            processing_callback(&sample_window[trim_size..FFT_SIZE-trim_size],
                scanline
            );
        }
    }

    fn decode_file(file: File) -> impl Sipper<Result<(),()>, Box<[PixelRGBA; WIDTH]>> {
        let decoder = Decoder::try_from(file).unwrap();
        let sample_rate = decoder.sample_rate();

               let waveform = decoder.collect_vec();

        sipper(async move |mut sender| {
            let (tx, rx) = flume::bounded(0);

            thread::spawn(move || {
                SlowScan::decode(
                    waveform,
                    sample_rate.into(),
                    |_, scanline| {
                        if let Some(scanline) = scanline {
                            let _ = tx.clone().send(Box::new(scanline));
                        }
                    }
                );
            });

            while let Ok(scanline) = rx.recv_async().await {
                sender.send(scanline).await;
            }

            Ok(())
        })
    }

    fn decode_and_play_file(file: File) -> impl Sipper<Result<(),()>, Box<[PixelRGBA; WIDTH]>> {
        let decoder = Decoder::try_from(file).unwrap();
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();

        let waveform = decoder.collect_vec();

        sipper(async move |mut sender| {
            let handle = rodio::DeviceSinkBuilder::open_default_sink().unwrap();
            let player = Arc::new(Mutex::new(Player::connect_new(handle.mixer())));
            let (tx, rx) = flume::bounded(0);

            let player_clone = player.clone();
            thread::spawn(move || {
                SlowScan::decode(
                    waveform,
                    sample_rate.into(),
                    |samples, scanline| {

                        let player = player_clone.lock().unwrap();
                        let to_play = SamplesBuffer::new(
                            channels,
                            sample_rate,
                            samples);

                        player.append(to_play);

                        if let Some(scanline) = scanline {
                            let tx = tx.clone();

                            let callback = EmptyCallback::new(Box::new(move || {
                                let _ = tx.send(Box::new(scanline));
                            }));

                            player.append(callback);
                        }
                    }
                );
            });

            while let Ok(scanline) = rx.recv_async().await {
                sender.send(scanline).await;
            }

            player.lock().unwrap().sleep_until_end();

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
        .theme(SlowScan::theme)
        .window_size((640, 480))
}

fn main() -> iced::Result {
    init_app().run()
}
