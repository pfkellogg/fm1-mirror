# flash_fm1.py: installs an FM1 Open firmware package (.fwsc) on an FM-1 that already runs
# Open firmware, over USB MIDI. A port of the "tagged" update in M-VAVE's own updater
# (fm1_open sdk/WebUpdate.js), so the FM-1 sees exactly what the workbench page sends.
#
#   flash_fm1.py PACKAGE.fwsc BUILD-IDENTITY.json [--check]
#
# The build identity file comes from the same build as the package (its build_id is what the FM-1
# reports afterwards); --check only inspects the package and the FM-1.
# The FM-1 asks for each block itself (address + length) and the reply carries it, so there
# is no timing to get wrong. Quit other apps that use the FM-1's MIDI first.
import hashlib, json, re, sys, time, queue
import mido

VENDOR_LOADER = "ef1f2db89b011fa53e71df363f0122de0ea896618c53fb53e8da676322004541"
OPEN_IDENTITY = re.compile(r"^(FM1_OFFICIAL_OPEN|FM1_COMMUNITY_\d+|FM-1_8\d{6})$")
LOADER_PORTS = re.compile(r"fm.?1|felucca|sloop|clipper|ota|sinco|usb.?midi|composite", re.I)
ENTER_LOADER = [0xf0, 0x22, 0x24, 0x35, 0x7f, 0xf7]


def fail(msg):
    raise SystemExit("STOPPED: " + msg)


def check(ok, msg):
    if not ok:
        fail(msg)


def le(p, at, n):
    return sum(p[at + i] << (8 * i) for i in range(n))


def crc16(b):
    c = 0
    for x in b:
        c ^= x << 8
        for _ in range(8):
            c = ((c << 1) ^ 0x1021 if c & 0x8000 else c << 1) & 0xFFFF
    return c


def decode_header(b, key=0xFFFF):
    p = bytearray(b)
    for i in range(len(p)):
        p[i] ^= key & 255
        key = ((key << 1) ^ (0x1021 if key & 0x8000 else 0)) & 0xFFFF
    return p


def ascii_z(p):
    return bytes(p).split(b"\0")[0].decode("ascii", "replace")


def inspect_package(data):
    """The flash image and loader inside a .fwsc, checked like the updater does."""
    check(1024 <= len(data) <= 2097152, "package length")
    image = bytearray(len(data) - 20)
    product, ended = "", False
    for i in range(20):
        image[i * 47:i * 47 + 47] = data[i * 48:i * 48 + 47]
        marker = data[i * 48 + 47]
        if marker == 125:
            ended = True
        else:
            c = (marker - i - 1) & 255
            check(not ended and 32 <= c <= 126, "package identity")
            product += chr(c)
    image[940:] = data[960:]
    head = decode_header(image[:64])
    count = le(head, 8, 2)
    check(le(head, 0, 2) == crc16(head[2:]) and le(head, 4, 4) == len(image), "package header")
    check(ascii_z(head[16:32]) == "AC791N" and 0 < count <= 12, "chip or directory")
    table = image[64:64 + count * 80]
    check(crc16(table) == le(head, 2, 2), "directory checksum")
    flash, loader = [], []
    for i in range(count):
        e = decode_header(table[i * 80:i * 80 + 80])
        typ, offset, size, stored, encrypted = le(e, 0, 2), le(e, 8, 4), le(e, 12, 4), le(e, 16, 4), le(e, 24, 4)
        check(offset >= 64 + count * 80 and offset <= len(image) and size <= len(image) - offset, "entry bounds")
        body = image[offset:offset + size]
        check(encrypted or crc16(body) == le(e, 4, 2), "entry checksum " + ascii_z(e[64:]))
        if typ == 0:
            flash.append(body)
        if typ == 100:
            loader.append(body)
    check(len(flash) == 1 and 16384 <= len(flash[0]) <= 1044480 and len(flash[0]) % 4096 == 0, "flash image")
    check(len(loader) == 1 and 0 < len(loader[0]) < 32768, "loader")
    return product, bytes(image), bytes(loader[0])


def pack7(b):
    out, bits, value = [], 0, 0
    for x in b:
        value |= x << bits
        bits += 8
        while bits >= 7:
            out.append(value & 127)
            value >>= 7
            bits -= 7
    if bits:
        out.append(value & 127)
    return out


def frame(command, payload=()):
    payload = list(payload)
    check(len(payload) <= 520, "payload too long")
    s = sum(payload) & 255
    return [0xF0] + pack7([0, 89, command, len(payload) & 255, len(payload) >> 8, 0] + payload + [(~s) & 255]) + [0xF7]


def parse(m):
    if len(m) < 10 or len(m) > 608 or m[0] != 0xF0 or m[-1] != 0xF7:
        return None
    raw, bits, value = [], 0, 0
    for x in m[1:-1]:
        if x > 127:
            return None
        value |= x << bits
        bits += 7
        if bits >= 8:
            raw.append(value & 255)
            value >>= 8
            bits -= 8
    if bits >= 7 or value or len(raw) < 7 or raw[0] != 0 or raw[1] != 89 or le(raw, 3, 3) != len(raw) - 7:
        return None
    if sum(raw[6:]) & 255 != 255:
        return None
    return raw[2], bytes(raw[6:-1])


class Link:
    def __init__(self, name):
        self.name = name
        self.queue = queue.Queue()
        self.inp = mido.open_input(name, callback=self._got)
        self.out = mido.open_output(name)

    def _got(self, msg):
        if msg.type == "sysex":
            p = parse([0xF0] + list(msg.data) + [0xF7])
            if p:
                self.queue.put(p)

    def send(self, m):
        self.out.send(mido.Message("sysex", data=m[1:-1]))

    def read(self, secs):
        try:
            return self.queue.get(timeout=secs)
        except queue.Empty:
            return None

    def query(self, command, payload=(), secs=1.5):
        while not self.queue.empty():
            self.queue.get_nowait()
        self.send(frame(command, payload))
        end = time.time() + secs
        while time.time() < end:
            r = self.read(min(0.25, end - time.time()))
            if r and r[0] == command:
                return r[1]
        return None

    def close(self):
        self.inp.close()
        self.out.close()


def identity(link):
    for _ in range(2):
        r = link.query(17, secs=0.75)
        if r is not None:
            check(len(r) == 27, "identity reply")
            return ascii_z(r)
    return None


def find(names_ok, want=None, secs=0.0):
    """Opens the one port that answers with an identity (or `want`); retries for `secs`."""
    end = time.time() + secs
    while True:
        found = []
        for name in mido.get_input_names():
            if not names_ok.search(name) or name not in mido.get_output_names():
                continue
            try:
                link = Link(name)
            except Exception:
                continue
            who = identity(link)
            if who and (want is None or who == want):
                found.append((link, who))
            else:
                link.close()
        check(len(found) <= 1, "more than one FM-1 answered: unplug the others")
        if found or time.time() >= end:
            return found[0] if found else (None, None)
        time.sleep(0.6)


def capability(data):
    check(data is not None and len(data) == 47 and data[:4] == b"FMC1" and data[4] == 1 and data[5] <= 7
          and le(data, 8, 2) == 512 and le(data, 10, 4) == 1048576, "update protocol mismatch")
    check(data[7] == 1, "unknown loader")
    return {"family": data[5], "state": data[6], "build": data[14:46].hex()}


def serve(link, image, finish, idle, phase):
    covered, total, last, started = bytearray((len(image) + 7) // 8), 0, time.time(), time.time()
    shown = -1
    while True:
        check(time.time() - started < max(idle * 3, 120), "update took too long")
        check(time.time() - last < idle, "the FM-1 stopped asking for data")
        p = link.read(0.25)
        if not p or p[0] != 48 or len(p[1]) != 8:
            continue
        d = p[1]
        address, n = le(d, 1, 4), le(d, 5, 3)
        check(0 < n <= 512, "bad request length")
        if address in (0xE0000000, 0xF0000000):
            check(address == finish and n == 8, "update phase mismatch")
            done = frame(48, [d[0], *d[1:8], *b"success", 0])
            link.send(done)
            end = time.time() + 0.6
            while time.time() < end:
                again = link.read(min(0.1, end - time.time()))
                if again and again[0] == 48 and len(again[1]) == 8 and le(again[1], 1, 4) == finish:
                    link.send(frame(48, [again[1][0], *again[1][1:8], *b"success", 0]))
            print(f"\r{phase}: done      ", flush=True)
            return
        check(address <= len(image) and n <= len(image) - address, "request outside the package")
        last = time.time()
        link.send(frame(48, [d[0], *d[1:8], *image[address:address + n]]))
        for i in range(address, address + n):
            if not covered[i >> 3] & (1 << (i & 7)):
                covered[i >> 3] |= 1 << (i & 7)
                total += 1
        pct = min(99, total * 100 // len(image))
        if pct != shown:
            shown = pct
            print(f"\r{phase}: {pct}%", end="", flush=True)


def main():
    package = sys.argv[1]
    build_id = json.load(open(sys.argv[2]))["build_id"].lower()
    only_check = "--check" in sys.argv
    data = open(package, "rb").read()
    product, image, loader = inspect_package(data)
    check(OPEN_IDENTITY.match(product), "not an Open firmware package")
    check(re.fullmatch(r"[0-9a-f]{64}", build_id), "build id")
    loader_identity = "ota-FM-1_015" if hashlib.sha256(loader).hexdigest() == VENDOR_LOADER else "ota-FM1C_" + build_id[:16]
    print(f"package {product}, image {len(image)} bytes, loader {loader_identity}")

    link, who = find(re.compile(r"fm.?1|^usb.?midi(?:\s+\d+)?$", re.I))
    check(link, "no FM-1 answered (is it plugged in and on, with other MIDI apps closed?)")
    print(f"FM-1 on '{link.name}' says it is {who}")
    check(OPEN_IDENTITY.match(who), "this FM-1 isn't running Open firmware: install M-VAVE's Official Open first")
    cap = capability(link.query(49))
    print(f"family {cap['family']}, state {cap['state']}, current build {cap['build'][:16]}")
    check(cap["state"] == 0, "the FM-1 is busy with another update")
    if cap["build"] == build_id:
        print("already running this build")
        return
    if only_check:
        print("check only: nothing written")
        return

    tag = list(bytes.fromhex(build_id))
    n = len(image)
    ack = link.query(50, [1, cap["family"], n & 255, (n >> 8) & 255, (n >> 16) & 255, (n >> 24) & 255, *tag])
    check(ack == b"\0", "the FM-1 refused the package")
    link.send(ENTER_LOADER)
    serve(link, image, 0xE0000000, 10, "Preparing the updater")
    link.close()
    print("waiting for the FM-1's updater to reconnect…")
    link, who = find(LOADER_PORTS, loader_identity, 40)
    check(link, "the updater didn't reconnect; run this again with the FM-1 still plugged in")
    link.send(ENTER_LOADER)
    serve(link, image, 0xF0000000, 180, "Writing firmware")
    link.close()
    print("waiting for the FM-1 to restart…")
    link, who = find(re.compile(r"fm.?1|^usb.?midi(?:\s+\d+)?$", re.I), product, 40)
    check(link, "the FM-1 didn't come back with the new firmware")
    after = capability(link.query(49))
    check(after["build"] == build_id and after["state"] == 0, "restarted, but the build doesn't match")
    link.close()
    print("Done: the FM-1 is running build", build_id[:16])


if __name__ == "__main__":
    main()
