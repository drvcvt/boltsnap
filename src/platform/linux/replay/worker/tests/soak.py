import argparse
import json
from pathlib import Path
import resource
import statistics
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description="Accelerated NUT transport soak; not a real-time capture benchmark")
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--loops", type=int, default=100)
    parser.add_argument("--syncpoints", choices=["none", "default"], default="none")
    args = parser.parse_args()
    if not 1 <= args.loops <= 10000:
        parser.error("loops must be 1..10000")
    directory = Path(tempfile.mkdtemp(prefix="boltsnap-replay-soak."))
    print(f"artifacts: {directory}", flush=True)
    children = []
    samples = []
    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    with (directory / "producer.log").open("wb") as producer_log, (directory / "worker.log").open("wb") as worker_log:
        try:
            producer = subprocess.Popen(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin",
                                         "-stream_loop", str(args.loops - 1), "-i", str(args.source.resolve()),
                                         "-c", "copy", "-f", "nut", "-write_index", "0", "-syncpoints", args.syncpoints,
                                         "-strict", "experimental", "-flush_packets", "1", "pipe:1"],
                                        stdout=subprocess.PIPE, stderr=producer_log)
            children.append(producer)
            worker = subprocess.Popen([str(args.worker.resolve()), "probe", "--input", "-",
                                       "--output", str(directory / "clip.mkv"), "--seconds", "3",
                                       "--memory-mib", "8", "--closed-gop"],
                                      stdin=producer.stdout, stdout=subprocess.PIPE, stderr=worker_log)
            children.append(worker)
            producer.stdout.close()
            while worker.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed > 180:
                    raise TimeoutError("soak exceeded 180 seconds")
                try:
                    status = Path(f"/proc/{worker.pid}/status").read_text()
                except FileNotFoundError:
                    break
                rss = next((int(line.split()[1]) for line in status.splitlines() if line.startswith("VmRSS:")), 0)
                if rss:
                    samples.append({"seconds": round(elapsed, 3), "rss_kib": rss})
                time.sleep(0.1)
            result, _ = worker.communicate(timeout=5)
            if worker.returncode != 0 or producer.wait(timeout=5) != 0:
                raise RuntimeError(f"transport failed; see {directory}")
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
            for child in children:
                child.wait(timeout=5)
    usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    report = {
        "accelerated": True, "loops": args.loops, "syncpoints": args.syncpoints,
        "wall_seconds": time.monotonic() - started,
        "child_user_seconds": usage_after.ru_utime - usage_before.ru_utime,
        "child_system_seconds": usage_after.ru_stime - usage_before.ru_stime,
        "worker": json.loads(result), "rss_samples": samples,
    }
    warmed = [sample["rss_kib"] for sample in samples
              if sample["seconds"] >= max(0.5, report["wall_seconds"] * 0.2)]
    report["rss_plateau_checked"] = len(warmed) >= 10
    if report["rss_plateau_checked"]:
        report["rss_growth_kib"] = statistics.median(warmed[-5:]) - statistics.median(warmed[:5])
    (directory / "report.json").write_text(json.dumps(report, indent=2))
    if report["worker"]["peak_accounted_bytes"] > report["worker"]["budget_bytes"]:
        raise AssertionError("ring exceeded its accounted byte budget")
    if report.get("rss_growth_kib", 0) > 4096:
        raise AssertionError(f"RSS grew after warmup; see {directory / 'report.json'}")
    decode = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-xerror",
                             "-i", str(directory / "clip.mkv"), "-f", "null", "-"], capture_output=True, timeout=30)
    if decode.returncode:
        raise AssertionError(decode.stderr.decode(errors="replace"))
    print(json.dumps({key: value for key, value in report.items() if key != "rss_samples"}))
    if samples:
        print(f"RSS KiB: first={samples[0]['rss_kib']} last={samples[-1]['rss_kib']} peak={max(s['rss_kib'] for s in samples)}")


if __name__ == "__main__":
    main()
