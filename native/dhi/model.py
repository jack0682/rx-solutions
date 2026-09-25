"""Device-side PTY register model. It is not physical XL430 qualification.

Only the foreign DHI plugin authors command packets on the slave. This model
reads those packets on the master and emits simulated device replies.
"""

import errno
import os
import select
import threading
import time
from collections import deque

MODEL_NUMBER = 1060
FIRMWARE = 44


def crc(data):
    value = 0
    for byte in data:
        value ^= byte << 8
        for _ in range(8):
            value = ((value << 1) ^ 0x8005 if value & 0x8000 else value << 1) & 65535
    return value


def packet(ident, body, stuff=True):
    if stuff:
        body = body.replace(b"\xff\xff\xfd", b"\xff\xff\xfd\xfd")
    data = (
        b"\xff\xff\xfd\x00"
        + bytes([ident])
        + (len(body) + 2).to_bytes(2, "little")
        + body
    )
    return data + crc(data).to_bytes(2, "little")


class Model:
    def __init__(self, master, emit):
        self.master = master
        self.emit = emit
        self.reg = bytearray(2048)
        self.reg[:2] = MODEL_NUMBER.to_bytes(2, "little")
        self.reg[6] = FIRMWARE
        self.reg[7] = 1
        self.reg[11] = 3
        self.reg[68] = 2
        self.reg[132:136] = (2048).to_bytes(4, "little")
        self.running = True
        self.error = None
        self.requests = 0
        self.writes = 0
        self.identity = None
        self.rows = deque(maxlen=512)
        self.lock = threading.RLock()
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def read(self, start, count):
        result = []
        for address in range(start, start + count):
            if 634 <= address < 690:
                slot = 578 + 2 * (address - 634)
                address = int.from_bytes(self.reg[slot : slot + 2], "little")
            elif 224 <= address < 280:
                slot = 168 + 2 * (address - 224)
                address = int.from_bytes(self.reg[slot : slot + 2], "little")
            result.append(self.reg[address])
        return bytes(result)

    def write(self, start, data):
        before = self.reg[64]
        for offset, value in enumerate(data):
            address = start + offset
            if 224 <= address < 280:
                slot = 168 + 2 * (address - 224)
                address = int.from_bytes(self.reg[slot : slot + 2], "little")
            self.reg[address] = value
        self.writes += 1
        if before != self.reg[64]:
            self.emit(
                {
                    "event": "model_torque_observed",
                    "value": self.reg[64],
                    "observed_at_ns": time.monotonic_ns(),
                    "stop_effect": "UNCONFIRMED",
                }
            )

    def respond(self, raw):
        if crc(raw[:-2]) != int.from_bytes(raw[-2:], "little"):
            raise ValueError("DHI_MODEL_CRC_INVALID")
        ident = raw[4]
        body = raw[7:-2].replace(b"\xff\xff\xfd\xfd", b"\xff\xff\xfd")
        instruction, params = body[0], body[1:]
        self.requests += 1
        self.rows.append(
            {
                "instruction": instruction,
                "id": ident,
                "packet": raw.hex(),
                "at_ns": time.monotonic_ns(),
            }
        )
        reply = None
        if ident not in (1, 254):
            return
        if instruction == 1:
            self.identity = {
                "model": int.from_bytes(self.reg[:2], "little"),
                "firmware": self.reg[6],
            }
            reply = packet(1, b"\x55\x00" + self.reg[:2] + bytes([self.reg[6]]))
        elif instruction == 2:
            reply = packet(
                1,
                b"\x55\x00"
                + self.read(
                    int.from_bytes(params[:2], "little"),
                    int.from_bytes(params[2:4], "little"),
                ),
            )
        elif instruction == 3:
            self.write(int.from_bytes(params[:2], "little"), params[2:])
            if ident != 254:
                reply = packet(1, b"\x55\x00")
        elif instruction in (0x82, 0x8A):
            address, count = int.from_bytes(params[:2], "little"), int.from_bytes(
                params[2:4], "little"
            )
            if 1 in params[4:]:
                data = self.read(address, count)
                reply = (
                    packet(254, b"\x55\x00\x01" + data, stuff=False)
                    if instruction == 0x8A
                    else packet(1, b"\x55\x00" + data)
                )
        elif instruction == 0x83:
            address, count = int.from_bytes(params[:2], "little"), int.from_bytes(
                params[2:4], "little"
            )
            rest = params[4:]
            for offset in range(0, len(rest), count + 1):
                if rest[offset] == 1:
                    self.write(address, rest[offset + 1 : offset + 1 + count])
        elif instruction == 8:
            reply = packet(1, b"\x55\x00")
        else:
            raise ValueError("DHI_MODEL_INSTRUCTION_UNSUPPORTED")
        if reply:
            os.write(self.master, reply)

    def loop(self):
        pending = b""
        try:
            while self.running:
                if not select.select([self.master], [], [], 0.02)[0]:
                    continue
                try:
                    data = os.read(self.master, 4096)
                except OSError as error:
                    if error.errno == errno.EIO:
                        time.sleep(0.01)
                        continue
                    raise
                pending += data
                while len(pending) >= 7:
                    if not pending.startswith(b"\xff\xff\xfd\x00"):
                        raise ValueError("DHI_MODEL_FRAME_INVALID")
                    size = 7 + int.from_bytes(pending[5:7], "little")
                    if size > 8192:
                        raise ValueError("DHI_MODEL_FRAME_TOO_LARGE")
                    if len(pending) < size:
                        break
                    raw, pending = pending[:size], pending[size:]
                    with self.lock:
                        self.respond(raw)
        except Exception as error:
            self.error = str(error)
            self.emit({"event": "model_error", "reason": self.error})

    def snapshot(self):
        with self.lock:
            return {
                "observed_at_ns": time.monotonic_ns(),
                "model_torque": self.reg[64],
                "requests": self.requests,
                "writes": self.writes,
                "identity": self.identity,
                "error": self.error,
                "stop_effect": "UNCONFIRMED",
                "physical_qualification": "NOT_PERFORMED",
            }

    def close(self):
        self.running = False
        self.thread.join(2)
        self.emit(
            {
                "event": "model_retired",
                "residual": self.snapshot(),
                "retirement_is_not_confirmed_stop": True,
            }
        )
        os.close(self.master)
