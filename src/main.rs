//#![allow(unused)]

use std::fs::File;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use iced::Center;
use iced::Color;
use iced::Fill;
use iced::Length::FillPortion;
use iced::task::{Sipper, sipper};
use iced::widget::pick_list;
use iced::{Application, Program, Task, Theme};
use iced::widget::{button, checkbox, column, container, image, radio, row, rule, space, text, text_input};
use iced::Element;
use bytes::{Bytes};
use iced_plot::LineStyle;
use iced_plot::PlotWidget;
use iced_plot::Series;
use iced_plot::ShapeId;
use itertools::Itertools;
use iced_plot::PlotWidgetBuilder;
use rodio::buffer::SamplesBuffer;
use rodio::microphone::{MicrophoneBuilder};
use rodio::source::EmptyCallback;
use rodio::{Decoder, Player, Source};

mod sstv;

mod pixel;
use pixel::PixelRGBA;

use crate::sstv::SSTVProgress;
use crate::sstv::decoder::DecoderState;
mod demod;

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

#[derive(Clone)]
enum Message {
    FilePathChanged(String),
    PlayToggled(bool),
    DecoderSourceChanged(DecoderSource),
    ModeChanged(sstv::SSTVMode),
    StartDecode,
    DecodeProgress(sstv::SSTVProgress),
    DecodeComplete(Result<(), ()>),
    PlaceHolder
}

struct SlowScan {
    play_while_decoding: bool,

    selected_source: Option<DecoderSource>,
    should_display_file_widgets: bool,

    selected_mode: Option<sstv::SSTVMode>,

    filepath: String,
    pixels: Vec<PixelRGBA>,
    curr_scanline: usize,
    curr_decoder_state: DecoderState,

    total_samples_processed: usize,

    spectrogram_sid: ShapeId,
    spectrogram: PlotWidget,

    spectrum_analyzer: PlotWidget
}

impl SlowScan {
    pub fn new() -> Self {

        let spectrogram_data = [[0.0, 0.0]].repeat(50_000);

        let spectrogram_series = Series::line_only(
            spectrogram_data,
            LineStyle::solid().with_pixel_width(1.5)
        ).with_color(Color::from_rgb8(255, 120, 0));
        let spectrogram_series_id = spectrogram_series.id;

        let mut spectrogram = PlotWidgetBuilder::new()
            .with_x_label("Time (MS)")
            .with_y_label("Hz")
            .with_y_lim(1100.0, 2500.0)
            .add_series(spectrogram_series)
            .with_autoscale_on_updates(true)
            .disable_controls_help()
            .disable_legend()
            .build()
            .unwrap();

        let mut spectrum_analyzer = PlotWidgetBuilder::new()
            .with_x_label("Hz")
            .with_y_label("dB")
            .disable_controls_help()
            .disable_legend()
            .build()
            .unwrap();

        spectrum_analyzer.get_controls_mut()
            .unbind_drag(iced::mouse::Button::Left);

        spectrogram.get_controls_mut()
            .unbind_drag(iced::mouse::Button::Left);

        Self {
            play_while_decoding: false,

            selected_source: None,
            should_display_file_widgets: false,

            selected_mode: None,

            filepath: String::from(""),
            pixels: vec![PixelRGBA::default(); WIDTH * HEIGHT],
            curr_scanline: 0,
            curr_decoder_state: DecoderState::AwaitingHSync,

            total_samples_processed: 0,

            spectrogram_sid: spectrogram_series_id,
            spectrogram,
            spectrum_analyzer
        }
    }

    pub fn title(&self) -> String {
        String::from("slowscan")
    }

    pub fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    pub fn view(&self) -> Element<'_, Message> {
        let handle = iced::advanced::image::Handle::from_rgba(
            WIDTH as u32,
            HEIGHT as u32,
            self.pixels_to_bytes());

        column![
            row![ // Top Row

                // Config Panel
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
                    ]

                ]).width(FillPortion(2)),

                rule::vertical(1),

                // Status Panel
                container(column![
                    text("Status").width(Fill).center(),

                    text("Progress:"),
                    text(format!("{}/{} scanlines", self.curr_scanline, HEIGHT)).width(Fill).center(),

                    text("Decoder State:"),
                    text(self.curr_decoder_state.to_string()).width(Fill).center(),

                    /*
                    // Don't display if quick decoding from file
                    text("Time Elapsed:"),
                    text("0m:0s").width(Fill).center(),

                    text("Processing Rate:"),
                    text("4 KB/s").width(Fill).center(),
                    */

                    space().height(Fill),

                    container(column![
                        button("decode").on_press(Message::StartDecode),
                        space().height(5)
                    ]
                    ).width(FillPortion(1)).align_x(Center)
                ]),

                rule::vertical(1),

                image(handle).expand(true)
            ].height(FillPortion(3)),

            rule::horizontal(1),

            row![ // Bottom Row
                // Spectrogram & spectrum
                container(self.spectrogram.view().map(|msg| { Message::PlaceHolder })).width(FillPortion(2)),
                rule::vertical(1),
                container(self.spectrum_analyzer.view().map(|msg| { Message::PlaceHolder })).width(FillPortion(1)),
            ]
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
                self.curr_scanline = 0;

                //let file = File::open(&self.filepath);
                let file = File::open("/home/wizard/Downloads/sstv_signal.wav");
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
            Message::DecodeProgress(progress) => {
                self.curr_decoder_state = progress.decoder_state;

                if let Some(scanline) = progress.scanline {
                    self.update_scanline(scanline);
                }

                self.spectrogram.update_series(&self.spectrogram_sid, |series| {
                    let frequencies = progress.frequencies.iter().map(|freq| {
                        self.total_samples_processed += 1;
                        [self.total_samples_processed as f64, *freq as f64]
                    }).collect_vec();

                    // NOTE: Currently drawing every single point.
                    // Maybe the entire signal could be displayed at once using downsampling/mipmapping.
                    let total_len = series.positions.len() + frequencies.len();
                    if total_len > series.positions.capacity() {
                        // Shift down to discard the oldest data points then fill the void with the new frequencies.
                        // Seems to be more efficient than removing one by one.
                        series.positions.copy_within(frequencies.len()-1.., 0);

                        let pos_to_insert = series.positions.capacity() - frequencies.len();
                        series.positions[pos_to_insert..].copy_from_slice(&frequencies);
                    }
                }).unwrap();


                Task::none()
            }
            Message::DecodeComplete(_) => {
                println!("Decoding complete.");
                Task::none()
            }
            _ => Task::none()
        }
    }

    fn decode_file(file: File) -> impl Sipper<Result<(),()>, SSTVProgress> {
        let decoder = Decoder::try_from(file).unwrap();
        let sample_rate = decoder.sample_rate();

        let waveform = decoder.collect_vec();

        sipper(async move |mut sender| {
            let (tx, rx) = flume::bounded(0);

            thread::spawn(move || {
                sstv::decode::<FFT_SIZE, _, _>(
                    waveform,
                    sample_rate.into(),
                    |progress| {
                        let _ = tx.clone().send(progress);
                    }
                );
            });

            while let Ok(scanline) = rx.recv_async().await {
                sender.send(scanline).await;
            }

            Ok(())
        })
    }

    fn decode_and_play_file(file: File) -> impl Sipper<Result<(),()>, SSTVProgress> {
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
                sstv::decode::<FFT_SIZE, _, _>(
                    waveform,
                    sample_rate.into(),
                    |progress| {
                        //let progress = Arc::new(progress);


                        let player = player_clone.lock().unwrap();
                        let to_play = SamplesBuffer::new(
                            channels,
                            sample_rate,
                            progress.samples.clone());

                        player.append(to_play);

                        let tx = tx.clone();

                        let callback = EmptyCallback::new(Box::new(move || {
                            let _ = tx.send(progress.clone());
                        }));

                        player.append(callback);

                    }
                );
            });

            while let Ok(progress) = rx.recv_async().await {
                sender.send(progress).await;
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

    fn update_scanline(&mut self, new_scanline: Vec<PixelRGBA>) {
        println!("Updating scanline {}", self.curr_scanline);
        if self.curr_scanline >= HEIGHT {
            return;
        }

        let scanline_start = WIDTH * self.curr_scanline;
        let scanline_end = scanline_start + WIDTH;

        let scanline = &mut self.pixels[scanline_start..scanline_end];
        scanline.copy_from_slice(&new_scanline);

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

struct SettingsPanel {
    
}

impl SettingsPanel {

}

struct InformationPanel {}

impl InformationPanel {

}

struct ImagePanel {}

impl ImagePanel {

}

struct PlotPanel {}


fn init_app() -> Application<impl Program> {
    iced::application(SlowScan::new, SlowScan::update, SlowScan::view)
        .title(SlowScan::title)
        .theme(SlowScan::theme)
        .window_size((1280, 720))
}

fn main() -> iced::Result {
    init_app().run()
}
