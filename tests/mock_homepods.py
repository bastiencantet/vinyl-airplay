"""Independent integration peer (Python), NOT a HomePod compatibility claim.

Run: python tests/mock_homepods.py target/debug/vinyl-airplay [gain_db|auto]
Checks real SRP proofs, key derivation, encrypted records/events, NTP requests,
two-receiver RTP sync, PCM integrity, retransmission, and clean TEARDOWN.
"""
import asyncio
import hashlib
import json
import math
import os
from pathlib import Path
import plistlib
import struct
import sys

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from srptools import SRPContext, SRPServerSession, SRPClientSession, constants


def derive(secret, salt, info):
    return HKDF(algorithm=hashes.SHA512(), length=32, salt=salt.encode(), info=info.encode()).derive(secret)


def tlv(fields):
    result = bytearray()
    for tag, value in fields.items():
        for offset in range(0, len(value), 255):
            piece = value[offset:offset + 255]
            result.extend(bytes([tag, len(piece)]) + piece)
    return bytes(result)


def untlv(data):
    fields = {}
    while data:
        tag, length = data[:2]
        assert len(data) >= length + 2
        fields[tag] = fields.get(tag, b"") + data[2:2 + length]
        data = data[2 + length:]
    return fields


def context():
    return SRPContext("Pair-Setup", "3939", prime=constants.PRIME_3072,
                      generator=constants.PRIME_3072_GEN, hash_func=hashlib.sha512)


class Wire:
    def __init__(self, reader, writer):
        self.reader, self.writer = reader, writer
        self.tx = self.rx = None
        self.sent = self.received = 0
        self.buffer = bytearray()

    def enable(self, tx, rx):
        self.tx, self.rx = ChaCha20Poly1305(tx), ChaCha20Poly1305(rx)

    async def byte(self):
        if self.rx is None:
            return await self.reader.readexactly(1)
        if not self.buffer:
            length = await self.reader.readexactly(2)
            size = int.from_bytes(length, "little")
            assert 0 < size <= 1024
            ciphertext = await self.reader.readexactly(size + 16)
            self.buffer.extend(self.rx.decrypt(bytes(4) + self.received.to_bytes(8, "little"), ciphertext, length))
            self.received += 1
        result = bytes(self.buffer[:1])
        del self.buffer[:1]
        return result

    async def read(self):
        header = bytearray()
        while not header.endswith(b"\r\n\r\n"):
            assert len(header) < 65536
            header.extend(await self.byte())
        start, *lines = header.decode().strip().split("\r\n")
        headers = dict((k.lower(), v.strip()) for k, v in (line.split(":", 1) for line in lines))
        length = int(headers.get("content-length", 0))
        body = b"".join([await self.byte() for _ in range(length)])
        return start, headers, body

    async def write(self, data):
        if self.tx:
            records = bytearray()
            for offset in range(0, len(data), 1024):
                part = data[offset:offset + 1024]
                length = len(part).to_bytes(2, "little")
                records.extend(length + self.tx.encrypt(bytes(4) + self.sent.to_bytes(8, "little"), part, length))
                self.sent += 1
            data = records
        # Deliberately fragment TCP writes, including encrypted framing boundaries.
        for offset in range(0, len(data), 137):
            self.writer.write(data[offset:offset + 137])
            await self.writer.drain()

    async def reply(self, headers, body=b"", extra=None):
        outgoing = {"CSeq": headers["cseq"], "Content-Length": str(len(body)), **(extra or {})}
        data = "RTSP/1.0 200 OK\r\n" + "".join(f"{k}: {v}\r\n" for k, v in outgoing.items()) + "\r\n"
        await self.write(data.encode() + body)


class Datagrams(asyncio.DatagramProtocol):
    def __init__(self):
        self.queue = asyncio.Queue()

    def connection_made(self, transport):
        self.transport = transport

    def datagram_received(self, packet, address):
        self.queue.put_nowait((packet, address))


class Receiver:
    def __init__(self, index):
        self.index = index
        self.packet_count = 5
        self.salt = bytes([index + 10]) * 16
        ctx = context()
        verifier = ctx.get_common_password_verifier(ctx.get_common_password_hash(self.salt))
        self.srp = SRPServerSession(ctx, password_verifier=verifier)
        self.pcm = []
        self.headers = []
        self.nonces = []
        self.tasks = []
        self.done = asyncio.Event()
        self.errors = []
        self.recorded = False
        self.metadata = None
        self.mrp_commands = []
        self.playback_states = []
        self.test_remote = False
        self.remote_checked = False
        self.volume_test = False
        self.volume_updates = []
        self.native_volume_sent = asyncio.Event()
        self.recognition_test = False
        self.recognized = asyncio.Event()
        self.artwork_seen = False
        self.setup_identity = {}

    def launch(self, coro):
        task = asyncio.create_task(coro)
        self.tasks.append(task)

    async def start(self):
        loop = asyncio.get_running_loop()
        self.data_transport, self.data = await loop.create_datagram_endpoint(Datagrams, local_addr=("127.0.0.1", 0))
        self.control_transport, self.control = await loop.create_datagram_endpoint(Datagrams, local_addr=("127.0.0.1", 0))
        self.event_server = await asyncio.start_server(lambda r, w: self.launch(self.events(r, w)), "127.0.0.1", 0)
        self.server = await asyncio.start_server(lambda r, w: self.launch(self.session(r, w)), "127.0.0.1", 0)
        return self.server.sockets[0].getsockname()[1]

    async def events(self, reader, writer):
        try:
            wire = Wire(reader, writer)
            wire.enable(derive(self.secret, "Events-Salt", "Events-Write-Encryption-Key"),
                        derive(self.secret, "Events-Salt", "Events-Read-Encryption-Key"))
            await wire.write(b"POST /event HTTP/1.1\r\nCSeq: 123\r\nContent-Length: 0\r\n\r\n")
            start, headers, _ = await wire.read()
            assert start == "HTTP/1.1 200 OK" and headers["cseq"] == "123"
            self.event_ack = True
            if self.volume_test:
                async def ready_volume():
                    while len(self.pcm) < 5:
                        await asyncio.sleep(0.005)
                await asyncio.wait_for(ready_volume(), 4)
                for index, level in enumerate([0.3, 1.5]):
                    event = plistlib.dumps({"type":"sendMediaRemoteCommand","value":"dvlc","params":{"volume":level}},fmt=plistlib.FMT_BINARY)
                    await wire.write(f"POST /event HTTP/1.1\r\nCSeq: {220+index}\r\nContent-Length: {len(event)}\r\n\r\n".encode()+event)
                    start, headers, _ = await wire.read()
                    assert start=="HTTP/1.1 200 OK" and headers["cseq"]==str(220+index)
                self.native_volume_sent.set()
            if self.test_remote:
                async def ready():
                    while len(self.pcm) < 5 or len(self.mrp_commands) < 5:
                        await asyncio.sleep(0.005)
                await asyncio.wait_for(ready(), 3)
                for index, (value, state, sample) in enumerate([
                    ("paus", 2, 0), ("paus", 2, 0), ("play", 1, 1000),
                    ("play", 1, 1000), ("plps", 2, 0), ("plps", 1, 1000),
                ]):
                    event = plistlib.dumps({"type": "sendMediaRemoteCommand", "value": value}, fmt=plistlib.FMT_BINARY)
                    await wire.write(f"POST /event HTTP/1.1\r\nCSeq: {124 + index}\r\nContent-Length: {len(event)}\r\n\r\n".encode() + event)
                    start, headers, _ = await wire.read()
                    assert start == "HTTP/1.1 200 OK" and headers["cseq"] == str(124 + index)
                    offset = len(self.pcm)
                    async def settled():
                        while len(self.pcm) < offset + 8 or self.playback_states[-1] != state:
                            await asyncio.sleep(0.005)
                    await asyncio.wait_for(settled(), 2)
                    assert all(packet == struct.pack(">hh", sample, sample) * 352 for packet in self.pcm[-3:]), value
                self.remote_checked = True
            await self.done.wait()
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            writer.close()

    async def session(self, reader, writer):
        wire = Wire(reader, writer)
        try:
            while True:
                start, headers, body = await wire.read()
                method, path, _ = start.split()
                if path == "/pair-setup":
                    fields = untlv(body)
                    if fields[6] == b"\x01":
                        assert fields[0x13] == b"\x10"
                        await wire.reply(headers, tlv({6: b"\x02", 2: self.salt, 3: bytes.fromhex(self.srp.public)}))
                    else:
                        self.srp.process(fields[3], self.salt)
                        assert self.srp.verify_proof(fields[4]), "client SRP proof rejected"
                        self.secret = bytes.fromhex(self.srp.key)
                        await wire.reply(headers, tlv({6: b"\x04", 4: bytes.fromhex(self.srp.key_proof_hash)}))
                        wire.enable(derive(self.secret, "Control-Salt", "Control-Read-Encryption-Key"),
                                    derive(self.secret, "Control-Salt", "Control-Write-Encryption-Key"))
                elif method == "GET" and path == "/info":
                    await wire.reply(headers, plistlib.dumps({"features": 1 << 50, "initialVolume":float(-18 + self.index*6)}, fmt=plistlib.FMT_BINARY))
                elif path == "/command":
                    assert headers["content-type"] == "application/x-apple-binary-plist"
                    command = plistlib.loads(body)
                    kind = command.get("type", "DeviceInfo")
                    params = command["params"]
                    if kind == "DeviceInfo":
                        data = params["data"]
                        length, offset = read_varint(data, 0)
                        assert length == len(data) - offset
                        message = protobuf(data[offset:])
                        assert message[1] == 15
                        info = protobuf(message[20])
                        assert info[19].decode() == headers["dacp-id"]
                        assert info[41].decode() == self.setup_identity["sessionUUID"]
                        assert info[42].decode() == self.setup_identity["groupUUID"]
                    elif kind == "updateMRNowPlayingInfo":
                        assert params["type"] == "npi-text" and params["mergePolicy"] in ["replace", "merge"]
                        info = params["params"]
                        if params["mergePolicy"] == "merge":
                            assert info["kMRMediaRemoteNowPlayingInfoPlaybackRate"] in [0.0, 1.0]
                            await wire.reply(headers)
                            continue
                        if self.recognition_test and info["kMRMediaRemoteNowPlayingInfoTitle"] != "Vinyle":
                            assert info["kMRMediaRemoteNowPlayingInfoTitle"] == "I THINK"
                            assert info["kMRMediaRemoteNowPlayingInfoArtist"] == "Tyler, The Creator"
                            assert info["kMRMediaRemoteNowPlayingInfoAlbum"] == "IGOR"
                            assert info["kMRMediaRemoteNowPlayingInfoArtworkData"] == bytes.fromhex("ffd8fffeffd9")
                            assert self.artwork_seen
                            assert "kMRMediaRemoteNowPlayingInfoDuration" not in info
                            self.recognized.set()
                            await wire.reply(headers)
                            continue
                        assert info["kMRMediaRemoteNowPlayingInfoTitle"] == "Vinyle"
                        assert info["kMRMediaRemoteNowPlayingInfoArtist"] == "Entrée audio"
                        assert info["kMRMediaRemoteNowPlayingInfoPlaybackRate"] == 1.0
                        assert "kMRMediaRemoteNowPlayingInfoDuration" not in info
                        assert isinstance(info["kMRMediaRemoteNowPlayingInfoTimestamp"], __import__("datetime").datetime)
                    elif kind == "updateMRSupportedCommands":
                        commands = [plistlib.loads(entry) for entry in params["mrSupportedCommandsFromSender"]]
                        assert commands == [{"kCommandInfoCommandKey": index, "kCommandInfoEnabledKey": True} for index in [0, 1, 2]]
                    elif kind == "updateMRPlaybackState":
                        assert params["mrPlaybackState"] in [1, 2]
                        self.playback_states.append(params["mrPlaybackState"])
                    elif kind == "updateMRNowPlayingClient":
                        assert protobuf(params["mrNowPlayingClient"])[7] == b"Vinyl AirPlay Rust"
                    else:
                        raise AssertionError(kind)
                    self.mrp_commands.append(kind)
                    await wire.reply(headers)
                elif method == "SETUP":
                    setup = plistlib.loads(body)
                    if "streams" not in setup:
                        self.setup_identity = setup
                        assert setup["timingProtocol"] == "NTP"
                        assert setup["senderSupportsRelay"] is True
                        timing_transport, timing = await asyncio.get_running_loop().create_datagram_endpoint(Datagrams, local_addr=("127.0.0.1", 0))
                        try:
                            request = b"\x80\xd2\x00\x07" + bytes(20) + (123).to_bytes(8, "big")
                            timing_transport.sendto(request, ("127.0.0.1", setup["timingPort"]))
                            response, _ = await asyncio.wait_for(timing.queue.get(), 2)
                            assert response[8:16] == request[24:32] and response[1] == 0xd3
                        finally:
                            timing_transport.close()
                        await wire.reply(headers, plistlib.dumps({"eventPort": self.event_server.sockets[0].getsockname()[1]}, fmt=plistlib.FMT_BINARY))
                    else:
                        assert self.recorded, "HomePod sequence requires RECORD before stream SETUP"
                        stream = setup["streams"][0]
                        assert stream["type"] == 96 and stream["audioFormat"] == 0x800 and stream["ct"] == 1
                        self.audio_key = stream["shk"]
                        self.sender_control = ("127.0.0.1", stream["controlPort"])
                        reply = {"streams": [{"dataPort": self.data_transport.get_extra_info("sockname")[1], "controlPort": self.control_transport.get_extra_info("sockname")[1]}]}
                        await wire.reply(headers, plistlib.dumps(reply, fmt=plistlib.FMT_BINARY))
                        self.launch(self.audio())
                        self.launch(self.controls())
                elif method == "RECORD":
                    self.recorded = True
                    await wire.reply(headers, extra={"Audio-Latency": str(11025 if self.index == 0 else 13230)})
                elif method == "SET_PARAMETER":
                    assert self.recorded
                    if headers["content-type"] == "text/parameters":
                        assert self.volume_test or self.index==1
                        assert body.startswith(b"volume: ") and body.endswith(b"\r\n")
                        level=float(body[8:].strip());assert math.isfinite(level) and -144 <= level <= 0
                        self.volume_updates.append(level)
                        await wire.reply(headers)
                        continue
                    if headers["content-type"] == "image/jpeg":
                        assert self.recognition_test
                        assert body == bytes.fromhex("ffd8fffeffd9")
                        self.artwork_seen = True
                        await wire.reply(headers)
                        continue
                    assert headers["content-type"] == "application/x-dmap-tagged"
                    assert headers["rtp-info"].startswith("seq=") and ";rtptime=" in headers["rtp-info"]
                    assert body[:4] == b"mlit"
                    assert int.from_bytes(body[4:8], "big") == len(body) - 8
                    fields = {}
                    offset = 8
                    while offset < len(body):
                        name = body[offset:offset + 4].decode("ascii")
                        length = int.from_bytes(body[offset + 4:offset + 8], "big")
                        fields[name] = body[offset + 8:offset + 8 + length].decode("utf-8")
                        offset += 8 + length
                    assert offset == len(body)
                    if self.recognition_test and fields.get("minm") != "Vinyle":
                        assert fields == {"minm": "I THINK", "asar": "Tyler, The Creator", "asal": "IGOR"}
                    else:
                        assert fields == {"minm": "Vinyle", "asar": "Entrée audio", "asal": "Entrée ligne"}
                    self.metadata = fields
                    await wire.reply(headers)
                elif method == "TEARDOWN":
                    await wire.reply(headers)
                    self.done.set()
                    break
                elif path == "/feedback":
                    await wire.reply(headers)
                else:
                    raise AssertionError(start)
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            writer.close()

    async def audio(self):
        cipher = ChaCha20Poly1305(self.audio_key)
        for index in range(self.packet_count):
            packet, _ = await asyncio.wait_for(self.data.queue.get(), 4)
            nonce = packet[-8:]
            assert int.from_bytes(nonce, "little") == index
            self.pcm.append(cipher.decrypt(bytes(4) + nonce, packet[12:-8], packet[4:12]))
            self.headers.append(packet[:12])
            self.nonces.append(nonce)
            if index == 0:
                self.first_packet = packet
                # Request exactly the packet received. It must be resent unchanged.
                request = b"\x80\xd5\x00\x01" + packet[2:4] + b"\x00\x01"
                self.control_transport.sendto(request, self.sender_control)

    async def controls(self):
        self.got_sync = self.got_resend = False
        while not (self.got_sync and self.got_resend):
            packet, _ = await asyncio.wait_for(self.control.queue.get(), 4)
            if packet[1] == 0xd4:
                playback, _, _, head = struct.unpack(">IIII", packet[4:20])
                assert (head - playback) & 0xffffffff == 13230, "common receiver latency not respected"
                self.got_sync = True
            elif packet[1] == 0xd6:
                assert packet[4:] == self.first_packet
                self.got_resend = True

    async def finish(self):
        await asyncio.wait_for(self.done.wait(), 4)
        await asyncio.gather(*self.tasks)
        self.server.close()
        self.event_server.close()
        self.data_transport.close()
        self.control_transport.close()
        assert not self.errors, self.errors
        assert self.got_sync and self.got_resend and self.event_ack


def write_vector():
    """Deterministic cross-language fixture; only public test values, never credentials."""
    salt = bytes.fromhex("00112233445566778899aabbccddeeff")
    ctx = context()
    verifier = ctx.get_common_password_verifier(ctx.get_common_password_hash(salt))
    server = SRPServerSession(ctx, password_verifier=verifier, private=bytes([0x23]) * 32)
    client = SRPClientSession(context(), private=bytes([0x42]) * 32)
    client.process(bytes.fromhex(server.public), salt)
    server.process(bytes.fromhex(client.public), salt)
    assert server.verify_proof(client.key_proof)
    vector = dict(private=(bytes([0x42]) * 32).hex(), salt=salt.hex(), server=server.public,
                  public=client.public, proof=client.key_proof, server_proof=server.key_proof_hash,
                  key=client.key)
    path = Path(__file__).parent / "fixtures" / "hap_srp.json"
    path.parent.mkdir(exist_ok=True)
    path.write_text(json.dumps(vector, indent=2) + "\n")


async def main(binary, gain_db=0):
    automatic = gain_db == "auto"
    receivers = [Receiver(0), Receiver(1)]
    for receiver in receivers:
        receiver.packet_count = 700 if automatic else 5
    ports = [await receiver.start() for receiver in receivers]
    pcm = bytes(range(256)) * 27 + bytes(range(128))  # 7040 bytes = 5 packets
    assert len(pcm) == 5 * 352 * 4
    if automatic:
        # More than five seconds of real signal plus the one-time startup fade.
        pcm = struct.pack(">hh", 1000, 500) * (352 * 700)
    scale = 1 if automatic else 10 ** (gain_db / 20)
    def amplify(value):
        scaled = value * scale
        rounded = math.floor(scaled + 0.5) if scaled >= 0 else math.ceil(scaled - 0.5)
        return max(-32768, min(32767, rounded))
    expected = b"".join(struct.pack(">h", amplify(value)) for (value,) in struct.iter_unpack(">h", pcm))
    gain_args = ["--auto-gain"] if automatic else ["--gain-db", str(gain_db)]
    process = await asyncio.create_subprocess_exec(binary, "stream", "--target", f"127.0.0.1:{ports[0]}",
        "--target", f"127.0.0.1:{ports[1]}", *gain_args, stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
    try:
        stdout, stderr = await asyncio.wait_for(process.communicate(pcm), 15)
        assert process.returncode == 0, stderr.decode()
        await asyncio.gather(*(receiver.finish() for receiver in receivers))
        if automatic:
            output = b"".join(receivers[0].pcm)
            assert output == b"".join(receivers[1].pcm), "automatic gain must match on both receivers"
            assert len(output) == len(pcm) and output != pcm
            assert output[:1408] == pcm[:1408], "measurement must start at unity gain"
            assert len(set(receivers[0].pcm[-20:])) == 1, "gain must stay fixed after calibration/fade"
            left, right = struct.unpack(">hh", output[-4:])
            assert left > 1000 and abs(left - 2 * right) <= 1
            assert "Input calibration complete: fixed gain" in stderr.decode()
            assert "calibrated fixed" in stderr.decode() and "SATURATION" not in stderr.decode()
        else:
            for receiver in receivers:
                assert b"".join(receiver.pcm) == expected
        if gain_db == 20:
            assert "SATURATION" in stderr.decode(), stderr.decode()
        assert receivers[0].headers == receivers[1].headers
        assert receivers[0].audio_key != receivers[1].audio_key
        print(stderr.decode(), end="")
        print(f"PASS: 2 independent receivers, verified SRP, encrypted control/events/audio, NTP, common latency, retransmissions, PCM gain mode {gain_db}, clean teardown.")
    except BaseException:
        if process.returncode is None:
            process.kill()
            await process.wait()
        for receiver in receivers:
            print("peer errors:", receiver.errors, file=sys.stderr)
        raise




def read_varint(data, offset):
    value = 0
    shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if not byte & 128:
            return value, offset
        shift += 7
        assert shift < 70


def protobuf(data):
    fields = {}
    offset = 0
    while offset < len(data):
        key, offset = read_varint(data, offset)
        if key & 7 == 0:
            value, offset = read_varint(data, offset)
        else:
            assert key & 7 == 2
            length, offset = read_varint(data, offset)
            value = data[offset:offset + length]
            assert len(value) == length
            offset += length
        fields[key >> 3] = value
    return fields


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--write-vector":
        write_vector()
    else:
        asyncio.run(main(sys.argv[1] if len(sys.argv) > 1 else "target/debug/vinyl-airplay",
                         ("auto" if sys.argv[2] == "auto" else int(sys.argv[2])) if len(sys.argv) > 2 else 0))
