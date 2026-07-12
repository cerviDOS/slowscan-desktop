import sys
import pandas as pd
from PIL import Image
import subprocess
import select
import cv2
import numpy as np

image = Image.new(mode='RGB', size=(320, 256), color=(0,0,0))
pixels: Image.PixelAccess = image.load()

teensy_output = subprocess.Popen(['tail', '-F', sys.argv[1]], \
    stdout=subprocess.PIPE)
io = select.poll()
io.register(teensy_output.stdout)

width = 320
height = 256

x = 0
y = 0

img = np.zeros([height, width, 3], dtype=np.uint8)
while True:
    print("Waiting for next scanline...")
    while io.poll(1):
        split = teensy_output.stdout.readline() \
            .decode() \
            .replace('\n', '') \
            .split(',', maxsplit=3)

        ignore_line = False

        for val in split:
            if not val.isdigit():
                ignore_line = True
                break

        if ignore_line or len(split) != 3:
            continue

        
        print(split)

        red, green, blue = tuple(map(int, split))

        img[y, x, 0] = blue
        img[y, x, 1] = green
        img[y, x, 2] = red

        x += 1
        if x == 320:
            x = 0
            if y == 255:
                y = 0
            else:
                y += 1

    cv2.imshow('SSTV', img)
    cv2.waitKey(1)
