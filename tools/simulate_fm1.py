# simulate_fm1.py: a pretend FM-1 with the Big Screen firmware, for testing FM-1 Mirror without one.
# A virtual MIDI device "FM1 Simulator" that, while hellos keep coming, sends a screen drawn with the
# FM-1's own 5x7 font, row by row, in the firmware's format (see the firmware's screen.c).
import sys, time, threading, mido

FONT = {}  # 5 columns per glyph, bit 0 at the top: the firmware's table
digits = [(0x3e,0x51,0x49,0x45,0x3e),(0,0x42,0x7f,0x40,0),(0x42,0x61,0x51,0x49,0x46),(0x21,0x41,0x45,0x4b,0x31),
          (0x18,0x14,0x12,0x7f,0x10),(0x27,0x45,0x45,0x45,0x39),(0x3c,0x4a,0x49,0x49,0x30),(0x01,0x71,0x09,0x05,0x03),
          (0x36,0x49,0x49,0x49,0x36),(0x06,0x49,0x49,0x29,0x1e)]
letters = [(0x7e,0x11,0x11,0x11,0x7e),(0x7f,0x49,0x49,0x49,0x36),(0x3e,0x41,0x41,0x41,0x22),(0x7f,0x41,0x41,0x22,0x1c),
           (0x7f,0x49,0x49,0x49,0x41),(0x7f,0x09,0x09,0x09,0x01),(0x3e,0x41,0x49,0x49,0x7a),(0x7f,0x08,0x08,0x08,0x7f),
           (0,0x41,0x7f,0x41,0),(0x20,0x40,0x41,0x3f,0x01),(0x7f,0x08,0x14,0x22,0x41),(0x7f,0x40,0x40,0x40,0x40),
           (0x7f,0x02,0x0c,0x02,0x7f),(0x7f,0x04,0x08,0x10,0x7f),(0x3e,0x41,0x41,0x41,0x3e),(0x7f,0x09,0x09,0x09,0x06),
           (0x3e,0x41,0x51,0x21,0x5e),(0x7f,0x09,0x19,0x29,0x46),(0x46,0x49,0x49,0x49,0x31),(0x01,0x01,0x7f,0x01,0x01),
           (0x3f,0x40,0x40,0x40,0x3f),(0x1f,0x20,0x40,0x20,0x1f),(0x3f,0x40,0x38,0x40,0x3f),(0x63,0x14,0x08,0x14,0x63),
           (0x07,0x08,0x70,0x08,0x07),(0x61,0x51,0x49,0x45,0x43)]
for i, g in enumerate(digits): FONT[str(i)] = g
for i, g in enumerate(letters): FONT[chr(65 + i)] = g
FONT['/'] = (0x60,0x18,6,1,0); FONT['-'] = (8,8,8,8,8); FONT[' '] = (0,0,0,0,0)

W = 240; screen = [0] * (W * W)
def fill(x, y, w, h, c):
    for r in range(y, y + h):
        for k in range(x, x + w): screen[r * W + k] = c
def text(x, y, s, scale, c):
    for ch in s.upper():
        g = FONT.get(ch, FONT[' '])
        for r in range(7 * scale):
            for k in range(6 * scale):
                col = k // scale
                screen[(y + r) * W + x + k] = c if col < 5 and g[col] >> (r // scale) & 1 else 0
        x += 6 * scale
TEXT, ACCENT, PANEL, DIM, AMBER = 0xef5d, 0x07ff, 0x18e3, 0x738e, 0xfd20
fill(0, 29, 240, 1, PANEL)
text(6, 8, "001 PIANO 1", 2, TEXT)
text(8, 50, "HOME", 3, ACCENT)
for t in range(8):
    fill(8 + 28 * t, 90, 24, 70, PANEL); fill(8 + 28 * t, 160 - 8 * (t + 1), 24, 8 * (t + 1), [0x3c9f,0x07bf,0x4eae,0x06d4,0xac9f,0xed18,0xff06,0xfba5][t])
for col, (lab, val) in enumerate([("ALG", "05"), ("FDBACK", "6"), ("TRANS", "+0"), ("VOL", "100")]):
    x = 6 + 59 * col; fill(x, 195, 54, 2, AMBER); text(x, 200, lab, 1, TEXT); text(x, 212, val, 2, AMBER)

def rows(y0, y1):
    for y in range(y0, y1):
        px = screen[y * W:(y + 1) * W]; i = 0
        while i < W:
            m = [0xf0, 0x7d, 70, 83, 1, y & 127, y >> 7, i & 127, i >> 7]
            while i < W and len(m) + 5 <= 70:
                run = 1
                while i + run < W and run < 127 and px[i + run] == px[i]: run += 1
                c = px[i]; m += [run, c & 127, (c >> 7) & 127, c >> 14]; i += run
            m.append(0xf7); yield m

heard = [0.0]; sending = [False]
out = mido.open_output("FM1 Simulator", virtual=True)
def got(msg):
    if msg.type == "sysex" and list(msg.data) == [0x7d, 70, 83, 0]:
        if time.time() - heard[0] > 3: sending[0] = True  # new listener: the whole screen
        heard[0] = time.time()
inp = mido.open_input("FM1 Simulator", virtual=True, callback=got)
print("FM1 Simulator running", flush=True)
tick = 0; end = time.time() + float(sys.argv[1] if len(sys.argv) > 1 else 60)
while time.time() < end:
    if sending[0]:
        sending[0] = False; n = 0
        for m in rows(0, W): out.send(mido.Message("sysex", data=m[1:-1])); n += 1
        print("sent whole screen:", n, "messages", flush=True)
    if time.time() - heard[0] < 3:
        # like turning a knob: the volume value changes, so only its rows go out again
        tick += 1; v = "%3d" % (100 - tick % 50)
        fill(183, 212, 54, 14, 0); text(183, 212, v.strip(), 2, AMBER)
        for m in rows(212, 226): out.send(mido.Message("sysex", data=m[1:-1]))
    time.sleep(0.25)
