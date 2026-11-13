"""Independent encrypted HomePod + fake Shazam: transient HTTP failure, title/album/JPEG,
PCM identity and live pause coexistence. No Internet or real audio device.
"""
import asyncio
import os
from pathlib import Path
import signal
import sys
import tempfile
from mock_homepods import Receiver, ChaCha20Poly1305

PRODUCER = '''#!/usr/bin/env python3
import os, struct, time
start = time.monotonic()
index = 0
try:
 while True:
  os.write(1, struct.pack(">hh", 1000, 1000) * 352)
  index += 1
  time.sleep(max(0, start + index * 352 / 44100 - time.monotonic()))
except BrokenPipeError: pass
'''
FINGERPRINT = '''#!/usr/bin/env python3
import sys, json
pcm = sys.stdin.buffer.read()
assert len(pcm) == 12 * 44100 * 4
assert pcm == bytes.fromhex("03e803e8") * (12 * 44100)
print(json.dumps({"uri":"data:audio/vnd.shazam.sig;base64,TEST", "samplems":12000}))
'''
CURL = '''#!/usr/bin/env python3
import sys, json, pathlib
if "amp.shazam.com/" in sys.argv[-1]:
 body = json.load(sys.stdin)
 assert body["signature"]["uri"] == "data:audio/vnd.shazam.sig;base64,TEST"
 assert body["signature"]["samplems"] == 12000
 assert set(body) == {"signature", "timestamp", "timezone"}
 state = pathlib.Path(__file__).with_name("requests")
 count = int(state.read_text()) if state.exists() else 0
 state.write_text(str(count + 1))
 if count == 0:
  sys.stdout.write("{}\\n429")
 else:
  sys.stdout.write(json.dumps({"track":{"key":"468056690","title":"I THINK","subtitle":"Tyler, The Creator","sections":[{"metadata":[{"title":"Album","text":"IGOR"}]}],"images":{"coverart":"https://is1-ssl.mzstatic.com/test.jpg"}}}) + "\\n200")
else:
 assert sys.argv[-1] == "https://is1-ssl.mzstatic.com/test.jpg"
 sys.stdout.buffer.write(bytes.fromhex("ffd8fffeffd9") + b"\\n200")
'''

async def main(binary):
    receiver = Receiver(1)
    receiver.recognition_test = True
    receiver.packet_count = 100
    port = await receiver.start()
    with tempfile.TemporaryDirectory(prefix="vinyl-recognition-") as directory:
        for name, code in [("arecord", PRODUCER), ("fingerprint", FINGERPRINT), ("curl", CURL)]:
            path = Path(directory) / name
            path.write_text(code)
            path.chmod(0o755)
        process = await asyncio.create_subprocess_exec(
            str(Path(binary).resolve()), "stream", "--alsa-device", "fake", "--target", f"127.0.0.1:{port}", "--recognize",
            env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"], "VINYL_SHAZAM_FINGERPRINT": str(Path(directory) / "fingerprint")},
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        async def monitor():
            lines = []
            async for line in process.stderr:
                lines.append(line)
                if b"recogn" in line.lower(): print(line.decode().strip(), flush=True)
            await process.wait()
            return b"", b"".join(lines)
        output = asyncio.create_task(monitor())
        try:
            recognized = asyncio.create_task(receiver.recognized.wait())
            done, _ = await asyncio.wait([recognized, output], timeout=80, return_when=asyncio.FIRST_COMPLETED)
            if output in done:
                raise AssertionError((await output)[1].decode())
            if recognized not in done:
                process.kill()
                _, stderr = await output
                raise AssertionError("recognition update timed out: " + stderr.decode())
            process.send_signal(signal.SIGINT)
            _, stderr = await asyncio.wait_for(output, 8)
            assert process.returncode == 0, stderr.decode()
            await receiver.finish()
            assert all(packet == bytes.fromhex("03e803e8") * 352 for packet in receiver.pcm)
            cipher = ChaCha20Poly1305(receiver.audio_key)
            count = len(receiver.pcm)
            while not receiver.data.queue.empty():
                packet, _ = receiver.data.queue.get_nowait()
                assert int.from_bytes(packet[-8:], "little") == count
                assert cipher.decrypt(bytes(4) + packet[-8:], packet[12:-8], packet[4:12]) == bytes.fromhex("03e803e8") * 352
                count += 1
            assert count > 40 * 44100 / 352, count
            assert "HTTP 429" in stderr.decode()
            assert "recognized metadata published" in stderr.decode()
            assert int((Path(directory) / "requests").read_text()) == 2
            print("PASS: HTTP 429 does not stop PCM; bounded retry; recognized title/album/JPEG reach encrypted HomePod; PCM unchanged; clean teardown.")
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()
            if not output.done(): output.cancel()
            receiver.server.close()
            receiver.event_server.close()
            receiver.data_transport.close()
            receiver.control_transport.close()

if __name__ == "__main__":
    asyncio.run(main(sys.argv[1]))
