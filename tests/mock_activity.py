"""Linux-only integration: fake arecord + independent encrypted AirPlay peers.
No real ALSA device or HomePod is used. Two activity/silence cycles, no idle TCP.
Run with the Python environment used by mock_homepods.py.
"""
import asyncio
import os
from pathlib import Path
import signal
import sys
import tempfile

from mock_homepods import Receiver

PRODUCER = '''#!/usr/bin/env python3
import os, struct, time
start = time.monotonic()
period = 352 / 44100
index = 0
try:
    while True:
        elapsed = time.monotonic() - start
        music = 1 <= elapsed < 4 or elapsed >= 12
        click = 0.35 <= elapsed < 0.35 + period
        sample = 1000 if music or click else 1
        os.write(1, struct.pack(">hh", sample, sample) * 352)
        index += 1
        time.sleep(max(0, start + index * period - time.monotonic()))
except BrokenPipeError:
    pass
'''


async def main(binary):
    peers = []
    handlers = []

    async def accept(reader, writer):
        peer = Receiver(1)
        if not peers:
            peer.test_remote = True
            peer.packet_count = 150
        await peer.start()
        peers.append(peer)
        task = asyncio.create_task(peer.session(reader, writer))
        handlers.append(task)
        await task

    server = await asyncio.start_server(accept, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="vinyl-activity-test-") as directory:
        fake = Path(directory) / "arecord"
        fake.write_text(PRODUCER)
        fake.chmod(0o755)
        process = await asyncio.create_subprocess_exec(
            str(Path(binary).resolve()), "stream", "--target", f"127.0.0.1:{port}",
            "--alsa-device", "fake", "--auto-connect", "--silence-seconds", "5",
            env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"]},
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        # Drain verbose jitter diagnostics while the child is running.
        output = asyncio.create_task(process.communicate())
        try:
            await asyncio.sleep(0.8)
            assert not peers, "idle input / isolated click opened a receiver connection"
            async def first_cycle():
                while not peers:
                    await asyncio.sleep(0.05)
                await peers[0].done.wait()
            await asyncio.wait_for(first_cycle(), 11)
            assert len(peers) == 1, "unexpected reconnect during silence"
            await asyncio.sleep(0.5)
            assert len(peers) == 1, "receiver was occupied again during silence"
            async def second_cycle():
                while len(peers) < 2 or len(peers[1].pcm) < 5:
                    await asyncio.sleep(0.05)
            await asyncio.wait_for(second_cycle(), 8)
            process.send_signal(signal.SIGINT)
            _, stderr = await asyncio.wait_for(output, 8)
            assert process.returncode == 0, stderr.decode()
            await asyncio.gather(*(peer.finish() for peer in peers))
            await asyncio.gather(*handlers)
            assert len(peers) == 2
            assert peers[0].remote_checked, "live pause/resume failed"
            assert all(peer.mrp_commands[:5] == ["DeviceInfo", "updateMRNowPlayingInfo", "updateMRSupportedCommands", "updateMRPlaybackState", "updateMRNowPlayingClient"] for peer in peers)
            assert all(peer.metadata == {"minm": "Vinyle", "asar": "Entrée audio", "asal": "Entrée ligne"} for peer in peers)
            assert all(peer.pcm[0] == bytes.fromhex("03e803e8") * 352 for peer in peers)
            assert "Silence timeout" in stderr.decode()
            assert stderr.decode().count("Monitoring input: no HomePod session") == 2
            print("PASS: idle noise/click ignored; music triggers encrypted playback; silence TEARDOWN; second activity reconnects; Ctrl-C cleans up.")
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()
            if not output.done():
                output.cancel()
            server.close()
            await server.wait_closed()
            for peer in peers:
                peer.server.close()
                peer.event_server.close()
                peer.data_transport.close()
                peer.control_transport.close()


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1] if len(sys.argv) > 1 else "target/release/vinyl-airplay"))
