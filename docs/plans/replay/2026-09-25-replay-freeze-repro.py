"""Synthetic worker repro for G5; exits 1 while a held selection is invisible.

Usage: python3 THIS_FILE --worker /absolute/path/to/boltsnap-replay-worker
No desktop capture, installed daemon, or existing user files are touched.
"""

import argparse
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, required=True)
    args = parser.parse_args()
    directory = Path(tempfile.mkdtemp(prefix="boltsnap-freeze-review-"))
    name = f"review-freeze-{os.getpid()}-{time.time_ns()}"
    address = f"\0boltsnap-replay-{os.geteuid()}-{name}"

    def call(command, **fields):
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(15)
            stream.connect(address)
            request = json.dumps({"version": 1, "command": command, **fields}).encode()
            stream.sendall(struct.pack(">I", len(request)) + request)

            def exact(size):
                data = b""
                while len(data) < size:
                    part = stream.recv(size - len(data))
                    if not part:
                        raise RuntimeError("unexpected worker EOF")
                    data += part
                return data

            response = json.loads(exact(struct.unpack(">I", exact(4))[0]))
            if response.get("preview_bytes"):
                exact(response["preview_bytes"])
            return response

    with (directory / "producer.log").open("wb") as log:
        producer = subprocess.Popen([
            "ffmpeg", "-v", "error", "-nostdin", "-re", "-f", "lavfi",
            "-i", "testsrc2=size=320x180:rate=30", "-t", "5", "-c:v", "libx264",
            "-threads:v", "2", "-preset", "ultrafast", "-g", "30", "-bf", "0",
            "-sc_threshold", "0", "-flags", "+cgop", "-f", "nut",
            "-write_index", "0", "-syncpoints", "none", "-strict", "experimental",
            "-flush_packets", "1", "pipe:1",
        ], stdout=subprocess.PIPE, stderr=log)
    worker = None
    try:
        with (directory / "worker.log").open("wb") as log:
            worker = subprocess.Popen([
                str(args.worker.resolve()), "live", name, str(directory), "3", "64", "libx264",
            ], stdin=producer.stdout, stdout=subprocess.DEVNULL, stderr=log)
        producer.stdout.close()
        deadline = time.monotonic() + 15
        while True:
            try:
                status = call("status")
                if status.get("ready") and status["duration_us"] >= 1_000_000:
                    break
            except (ConnectionRefusedError, FileNotFoundError):
                pass
            assert time.monotonic() < deadline, "worker did not become ready"
            time.sleep(0.05)
        frozen = call("freeze")
        assert frozen["ok"], frozen
        producer.wait(timeout=10)
        while True:
            status = call("status")
            if not status["ready"]:
                break
            assert time.monotonic() < deadline, "worker did not observe EOF"
            time.sleep(0.05)
        saved = call("save", snapshot=frozen["snapshot"])
        assert saved["ok"], saved
        report = {"frozen": frozen, "status_while_frozen_after_eof": status, "saved": saved}
        (directory / "report.json").write_text(json.dumps(report, indent=2))
        print(f"artifacts: {directory}", flush=True)
        print(json.dumps(report), flush=True)
        assert call("stop")["ok"]
        worker.wait(timeout=5)
        assert status["busy"] or status.get("selection_active", False), (
            "G5: a valid frozen selection is invisible to the watchdog's ready/busy check"
        )
    finally:
        for child in (worker, producer):
            if child is not None:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)


if __name__ == "__main__":
    main()
