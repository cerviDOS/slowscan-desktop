
import time
from numpy import average, unwrap, atan2, pi, zeros
from scipy import signal, io
from scipy.signal import freqz, remez, fftconvolve, hilbert, firls, windows
from matplotlib.pyplot import plot, sca, show, axhline, vlines
from PIL import Image

image = Image.new(mode='RGB', size=(320, 256), color=(0,0,0))
pixels: Image.PixelAccess = image.load()

def hz_2_intensity(val):
    result = int(((val - 1500) / (2300 - 1500)) * 255)
    
    if result > 255:
        return 255
    elif result < 0:
        return 0
    else:
        return result

def IQ_2_phase(I_samples, Q_samples):
    phases = []
    for (i,q) in zip(I_samples, Q_samples):
        phases.append(atan2(q, i))

    return unwrap(phases)

def IQ_2_frequency(I, Q, fs):
    freq = []

    phases = IQ_2_phase(I, Q)

    for index in range(1, len(I)-1):
        phase2 = phases[index];
        phase1 = phases[index-1];
        freq.append(((phase2 - phase1) / (2 * pi)) * fs)
    return freq

def get_frequencies(data, fs):
    N = len(data)
    data = data * windows.kaiser(len(data), 8)
    res = hilbert(data)
    return IQ_2_frequency(res.real, res.imag, fs)

def detect_hsync(data, fs):
    freqs = get_frequencies(data, fs)

    tolerance = 50
    hsync = 1200
    hsync_low = hsync - tolerance
    hsync_high = hsync + tolerance

    hsync_x = []
    hsync_val = []

    for index, val in enumerate(freqs):
        if val >= hsync_low and val <= hsync_high:
            hsync_x.append(index)
            hsync_val.append(val)

    plot(freqs, 'b')
    plot(hsync_x, hsync_val, 'r.')


def is_within_error(value, query, tolerance):
    return (value > query - tolerance) and (value < query + tolerance)

def load_rgb(r,g,b,y):
    for i in range(320):
        pixels[i, y] = (r[i], g[i], b[i])


def display_scanline(data, fs, cursor_pos, scanline_num):
    #           0.572ms porch
    #           146.432ms green
    #           0.572ms seperator
    #           146.432ms blue
    #           0.572ms seperator
    #           146.432ms red
    #           0.572ms seperator

    secs_per_pixel = 0.0004576
    samples_per_pixel = secs_per_pixel * fs
    
    green_vals = [0] * 320
    blue_vals = [0] * 320
    red_vals = [0] * 320

    eof = False

    # Green loop
    x_pos = 0
    while (x_pos < 320):
        pixel_end_index = cursor_pos + samples_per_pixel

        pixel_data = []
        while (cursor_pos < pixel_end_index):
            if cursor_pos >= len(data):
                eof = True
                break

            pixel_data.append(data[cursor_pos])
            cursor_pos += 1

        if eof:
            break

        green_val = hz_2_intensity(average(pixel_data))
        green_vals[x_pos] = green_val

        #print(f"({x_pos, scanline_num}, G={green_val}")
        #pixels[x_pos, scanline_num] = (0, green_val, 0)

        x_pos += 1
    

    # Blue loop
    x_pos = 0
    while (x_pos < 320):
        pixel_end_index = cursor_pos + samples_per_pixel

        pixel_data = []
        while (cursor_pos < pixel_end_index):
            if cursor_pos >= len(data):
                eof = True
                break

            pixel_data.append(data[cursor_pos])
            cursor_pos += 1

        if eof:
            break

        blue_val = hz_2_intensity(average(pixel_data))
        blue_vals[x_pos] = blue_val

        #print(f"({x_pos, scanline_num}, G={green_val}")
        #pixels[x_pos, scanline_num] = (0, green_val, 0)

        x_pos += 1

    # Red loop
    x_pos = 0
    while (x_pos < 320):
        pixel_end_index = cursor_pos + samples_per_pixel

        pixel_data = []
        while (cursor_pos < pixel_end_index):
            if cursor_pos >= len(data):
                eof = True
                break

            pixel_data.append(data[cursor_pos])
            cursor_pos += 1

        if eof:
            break

        red_val = hz_2_intensity(average(pixel_data))
        red_vals[x_pos] = red_val

        #print(f"({x_pos, scanline_num}, G={green_val}")
        #pixels[x_pos, scanline_num] = (0, green_val, 0)

        x_pos += 1

    load_rgb(red_vals, green_vals, blue_vals, scanline_num)
    return cursor_pos


def detect_horizontal_sync(data, fs, cursor_pos):
    hsync_start = cursor_pos

    while is_within_error(data[cursor_pos], 1200, 50):
        cursor_pos += 1

    # TODO: check if I need to account for extra addition to cursor_pos in 
    # loop before false is found

    secs_per_sample = 1 / fs
    hsync_duration_ms = (cursor_pos - hsync_start) * secs_per_sample * 1000
    if (hsync_duration_ms > 1):
        # returning cursor_pos-1 to account for extra increment
        # before loop ends
        return (True, hsync_duration_ms, hsync_start, cursor_pos-1)
    else:
        return (False, 0, hsync_start, cursor_pos)

def decode_sstv(data, fs):
    # iterate through frequency/time data
    # if hsync is detected --- 1200Hz for 4.862ms, 1500Hz for 0.572ms
    #   for next x samples (~441ms worth), begin reading color data (need timings)
    # else, continue reading
    # only display a line if it was preceded by a hsync

    # until 256 lines are read or in waterfall:
    #   read data until hsync is found (1200Hz for 4.862ms)
    #       when hsync is detected, begin reading color data
    #           0.572ms porch
    #           146.432ms green
    #           0.572ms seperator
    #           146.432ms blue
    #           0.572ms seperator
    #           146.432ms red
    #           0.572ms seperator
    #
    #
    #           try: assume that 0.572ms porch occurs immediately after last hsync reading and 
    #                   delay accordingly.
    #           try: ignore porch and begin reading colors immediately
    frequencies = get_frequencies(data, fs)

    index = 0
    size = len(frequencies)

    scanline_index = 0;
    while(index < size):
        value = frequencies[index]

        found, duration_ms, start, end = detect_horizontal_sync(frequencies, fs, index)
        if found:
            index = end

            if is_within_error(duration_ms, 4.862, 0.5):
                print(f"DECODE: detected hysnc, {duration_ms}, {start}, {end}")
                vlines(start, value - 100, value + 100, color='g')
                index = display_scanline(frequencies, fs, index, scanline_index)
                scanline_index += 1



        else:
            index += 1


    return None

def time_hsync(data, fs):
    freqs = get_frequencies(data, fs)

    freq_tolerance = 50
    hsync = 1200
    hsync_low = hsync - freq_tolerance
    hsync_high = hsync + freq_tolerance

    secs_per_sample = 1 / fs

    hsync_latch = False
    hsync_start = 0
    hsync_end = 0
    for index, val in enumerate(freqs):
        if (val >= hsync_low and val <= hsync_high) and not hsync_latch:
            hsync_start = index
            hsync_latch = True
        elif hsync_latch and not (val >= hsync_low and val <= hsync_high):
            last_end = hsync_end

            hsync_latch = False
            hsync_end = index

            hsync_duration_ms = (hsync_end - hsync_start) * secs_per_sample * 1000
            ms_since_last_sync = (hsync_start - last_end) * secs_per_sample * 1000

            # toss detections lasting less than a millisecond
            if (hsync_duration_ms > 1):
                print(f"hsync @ ({hsync_start}-{hsync_end}), duration: {hsync_duration_ms}")
                print(f"time since last detection: {ms_since_last_sync}\n")
            else:
                # restore previous 
                hsync_end = last_end

import sys
sample_rate, data = io.wavfile.read(sys.argv[1])
#detect_hsync(data, sample_rate)
#time_hsync(data, sample_rate)

start = time.time_ns() / 10**9
decode_sstv(data, sample_rate)
end = time.time_ns() / 10**9

print(f"Total Time: {end - start}")


image.show()
