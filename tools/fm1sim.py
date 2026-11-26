# fm1sim.py: a pretend FM-1 for testing several at once: virtual MIDI ports "FM-1 Sim N" that answer the app's
# Big Screen hellos with a 240x240 screen ("FM-1 SIM N", a moving bar), the v1 row protocol (screen.rs).
import sys, time, threading, rtmidi
from PIL import Image, ImageDraw, ImageFont
N = sys.argv[1] if len(sys.argv) > 1 else "2"
NAME = f"FM-1 Sim {N}"
F = ImageFont.truetype("/System/Library/Fonts/Avenir Next.ttc", 34, index=0)
def frame(t):
    im = Image.new("RGB", (240, 240), (10, 30, 70)); d = ImageDraw.Draw(im)
    d.text((14, 20), f"FM-1 SIM {N}", font=F, fill=(255, 210, 31))
    d.text((14, 80), "test unit", font=F, fill=(255, 255, 255))
    x = int(t * 60) % 200
    d.rectangle([x, 170, x + 40, 210], fill=(60, 220, 90))
    return im
def rows(im, only=None):
    px = im.load(); out = []
    for y in range(240):
        if only is not None and y not in only: continue
        runs, x = [], 0
        while x < 240:
            r, g, b = px[x, y]; c = (r >> 3) << 11 | (g >> 2) << 5 | (b >> 3); n = 1
            while x + n < 240 and n < 127 and px[x + n, y] == px[x, y]: n += 1
            runs += [n, c & 127, (c >> 7) & 127, (c >> 14) & 3]; x += n
        out.append([0xf0, 0x7d, ord('F'), ord('S'), 1, y & 127, y >> 7, 0, 0] + runs + [0xf7])
    return out
mo = rtmidi.MidiOut(); mo.open_virtual_port(NAME)
mi = rtmidi.MidiIn(); mi.ignore_types(sysex=False); mi.open_virtual_port(NAME)
asked = threading.Event(); last_hello = [0.0]
def cb(msg, _):
    m = msg[0]
    if len(m) >= 5 and m[:4] == [0xf0, 0x7d, ord('F'), ord('S')]:
        last_hello[0] = time.time()
        if m[4] == 1: asked.set()
mi.set_callback(cb)
print("ready", NAME, flush=True)
t0 = time.time(); prev = None
while True:
    if time.time() - last_hello[0] < 3:          # like the FM-1: sends while it's asked
        im = frame(time.time() - t0)
        if prev is None or asked.is_set():
            asked.clear(); send = rows(im)
        else:
            send = rows(im, only=range(165, 215))
        for m in send: mo.send_message(m); time.sleep(0.0005)
        prev = im
    else:
        prev = None
    time.sleep(0.1)
