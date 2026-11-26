# fm1sim.py: a pretend FM-1 for testing several at once: virtual MIDI ports "FM-1 Sim N" that answer the app's
# Big Screen hellos with a 240x240 screen ("FM-1 SIM N", a moving bar), the v1 row protocol (screen.rs).
import sys, time, threading, rtmidi
from PIL import Image, ImageDraw, ImageFont
N = sys.argv[1] if len(sys.argv) > 1 else "2"
NAME = f"FM-1 Sim {N}"
F = ImageFont.truetype("/System/Library/Fonts/Avenir Next.ttc", 34, index=0)
F2 = ImageFont.truetype("/System/Library/Fonts/Avenir Next.ttc", 22, index=0)
KNOBS = [0] * 8          # the app's Big Knob turns it got (04 <knob> <delta>), shown on its screen
GOT = [0.0]
def frame(t):
    im = Image.new("RGB", (240, 240), (10, 30, 70)); d = ImageDraw.Draw(im)
    d.text((14, 20), f"FM-1 SIM {N}", font=F, fill=(255, 210, 31))
    d.text((14, 66), "pretend FM-1", font=F2, fill=(255, 255, 255))
    names = ["SELECT", "ALGO", "KNOB 1", "KNOB 2", "KNOB 3", "KNOB 4", "PRESETS", "MASTER"]
    for i in (2, 3, 4, 5):
        y = 100 + (i - 2) * 26
        d.text((14, y), f"{names[i]}", font=F2, fill=(200, 200, 210))
        v = KNOBS[i]
        col = (60, 220, 90) if time.time() - GOT[0] < 1.5 else (255, 255, 255)
        d.text((150, y), f"{v:+d}", font=F2, fill=col)
    x = int(t * 60) % 200
    d.rectangle([x, 212, x + 30, 232], fill=(60, 220, 90))
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
        if m[4] >= 2: print("got", " ".join(f"{b:02x}" for b in m[:12]), flush=True)   # a knob or a button (the lock)
        if m[4] == 4 and len(m) >= 7:
            k, n = m[5], m[6]
            if k < 8:
                KNOBS[k] += n - 128 if n >= 64 else n
                GOT[0] = time.time(); asked.set()
    elif m and m[0] == 0xf0:
        print("got", " ".join(f"{b:02x}" for b in m[:12]), flush=True)   # a knob, a button (to check the lock)
mi.set_callback(cb)
print("ready", NAME, flush=True)
t0 = time.time(); prev = None
while True:
    if time.time() - last_hello[0] < 3:          # like the FM-1: sends while it's asked, only the rows that changed
        im = frame(time.time() - t0)
        if prev is None or asked.is_set():
            asked.clear(); send = rows(im)
        else:
            a, b = prev.tobytes(), im.tobytes()
            changed = [y for y in range(240) if a[y * 720:(y + 1) * 720] != b[y * 720:(y + 1) * 720]]
            send = rows(im, only=set(changed))
        for m in send: mo.send_message(m); time.sleep(0.002)
        prev = im
    else:
        prev = None
    time.sleep(0.1)
