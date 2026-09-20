import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time

from probe import ffmpeg, run, frame_hashes


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--encoder", default="libx264")
    args = parser.parse_args()
    run([str(args.worker.resolve()), "check-encoder", args.encoder])
    directory = Path(tempfile.mkdtemp(prefix="boltsnap-replay-live."))
    print(f"artifacts: {directory}", flush=True)
    name = f"test-{os.getpid()}-{time.time_ns()}"
    address = f"\0boltsnap-replay-{os.geteuid()}-{name}"
    producer = subprocess.Popen(["ffmpeg", "-v", "error", "-nostdin", "-re", "-f", "lavfi", "-i",
        "testsrc2=size=640x360:rate=60", "-re", "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
        "-c:v", "libx264", "-threads:v", "2", "-preset", "ultrafast", "-crf", "23", "-g", "60", "-bf", "0",
        "-sc_threshold", "0", "-flags", "+cgop", "-c:a", "aac", "-f", "nut", "-write_index", "0",
        "-syncpoints", "none", "-strict", "experimental", "-flush_packets", "1", "pipe:1"], stdout=subprocess.PIPE,
        stderr=(directory / "producer.log").open("wb"))
    worker = subprocess.Popen([str(args.worker.resolve()), "live", name, str(directory), "3", "64", args.encoder],
        stdin=producer.stdout, stderr=(directory / "worker.log").open("wb"))
    producer.stdout.close()

    def call(command, **fields):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(30)
            connection.connect(address)
            data = json.dumps({"version": 1, "command": command, **fields}).encode()
            connection.sendall(struct.pack(">I", len(data)) + data)
            def exact(n):
                result = b""
                while len(result) < n:
                    part = connection.recv(n - len(result))
                    assert part, "unexpected EOF"
                    result += part
                return result
            response = json.loads(exact(struct.unpack(">I", exact(4))[0]))
            if response.get("preview_bytes"):
                preview = exact(response["preview_bytes"])
                assert preview[:8] == b"\x89PNG\r\n\x1a\n"
                assert struct.unpack(">II", preview[16:24]) == (640, 360)
                (directory / "preview.png").write_bytes(preview)
            return response

    try:
        deadline = time.monotonic() + 15
        while True:
            assert worker.poll() is None, (directory / "worker.log").read_text()
            try:
                status = call("status")
                if status.get("ready") and status["duration_us"] >= 2_500_000:
                    break
            except (ConnectionRefusedError, FileNotFoundError):
                pass
            assert time.monotonic() < deadline, status
            time.sleep(0.1)
        frozen = call("freeze")
        assert frozen["ok"], frozen
        assert not call("freeze")["ok"]
        time.sleep(1)
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            export = pool.submit(call, "save", snapshot=frozen["snapshot"],
                                 crop={"x": 64, "y": 48, "width": 320, "height": 180})
            time.sleep(0.3)
            status = call("status")
            assert status["ready"] and status["busy"], status
            assert not call("save")["ok"]
            cropped = export.result(timeout=30)
        assert cropped["ok"], cropped
        assert cropped["duration_us"] == frozen["duration_us"], (frozen, cropped)
        assert not call("save", snapshot=frozen["snapshot"])["ok"]
        latest = call("freeze")
        assert latest["ok"], latest
        full = call("save", snapshot=latest["snapshot"])
        assert full["ok"] and 0 < full["duration_us"] <= 3_000_000, full
        assert frame_hashes(directory / "preview.png", "format=rgb24") == frame_hashes(full["path"], "format=rgb24")[-1:]
        for result, dimensions in [(cropped, (320, 180)), (full, (640, 360))]:
            path = result["path"]
            ffmpeg("-xerror", "-i", path, "-f", "null", "-")
            streams = json.loads(run(["ffprobe", "-v", "error", "-count_frames", "-show_streams", "-of", "json", path]).stdout)["streams"]
            video = next(s for s in streams if s["codec_type"] == "video")
            assert (video["width"], video["height"]) == dimensions, video
            assert video["avg_frame_rate"] == "60/1", video
            assert int(video["nb_read_frames"]) == round(result["duration_us"] * 60 / 1_000_000), video
            assert any(s["codec_type"] == "audio" for s in streams)
        (directory / "report.json").write_text(json.dumps({"frozen": frozen, "cropped": cropped, "full": full}, indent=2))
        assert call("stop")["ok"]
        worker.wait(timeout=5)
        print("passed: live buffer, frozen crop, concurrent status, export limit, stale snapshot, full clip, audio, decode")
    finally:
        for child in [worker, producer]:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)


if __name__ == "__main__":
    main()
