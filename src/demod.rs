use rustfft::FftPlanner;

use num::Complex;
use std::f32::consts::PI;
use std::ops::Div;

use std::iter::zip;
use itertools::Itertools;

pub fn demodulate_frequencies<const FFT_SIZE: usize>(waveform: &[f32; FFT_SIZE], sample_rate: u32) -> Vec<f32> {
    let mut windowed_waveform = *waveform;
    apply_window_fn(&mut windowed_waveform);

    let mut waveform_complex = windowed_waveform.iter()
        .map_into::<Complex<f32>>()
        .collect_vec();

    compute_analytic_signal::<FFT_SIZE>(
        waveform_complex.as_mut_slice()
            .try_into()
            .unwrap()
    );

    let mut frequencies = Vec::new();

    for pair in waveform_complex.windows(2) {
        frequencies.push(inst_frequency(pair[0], pair[1], sample_rate));
    }

    frequencies
}

fn apply_window_fn<const FFT_SIZE: usize>(waveform: &mut [f32; FFT_SIZE]) {
    let window_type = windowfunctions::WindowFunction::BlackmanHarris;
    let symmetry = windowfunctions::Symmetry::Symmetric;
    let window_fn = windowfunctions::window::<f32>(FFT_SIZE,
        window_type,
        symmetry);

    for (sample, window_term) in zip(waveform, window_fn) {
        *sample *= window_term;
    }
}

fn compute_analytic_signal<const FFT_SIZE: usize>(waveform_complex: &mut [Complex<f32>; FFT_SIZE]) {
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

    let phase_diff_i16 = phase_b.wrapping_sub(phase_a);

    let phase_diff_f32= phase_diff_i16 as f32 / i16::MAX as f32;

    (phase_diff_f32 / 2.0) * sample_rate as f32
}
