import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def run(args, *, success=True):
    result = subprocess.run(args, capture_output=True, timeout=90)
    if (result.returncode == 0) != success:
        raise AssertionError(f"command failed expectation: {args}\n{result.stderr.decode(errors='replace')}")
    return result


def ffmpeg(*args):
    return run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-n", *map(str, args)])


def frame_hashes(path, filters=None):
    args = ["-i", path, "-map", "0:v"]
    if filters:
        args += ["-vf", filters]
    result = ffmpeg(*args, "-vsync", "0", "-f", "framemd5", "-")
    return [line.rsplit(b",", 1)[1].strip() for line in result.stdout.splitlines() if not line.startswith(b"#")]


def audio_packets(path):
    result = run(["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_packets",
                  "-show_data_hash", "sha256", "-show_entries", "packet=data_hash,pts_time,duration_time",
                  "-of", "json", str(path)])
    return json.loads(result.stdout)["packets"]


def main():
    parser = argparse.ArgumentParser(description="Synthetic replay transport and remux checks")
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--vulkan", action="store_true")
    args = parser.parse_args()
    worker = str(args.worker.resolve())
    directory = Path(tempfile.mkdtemp(prefix="boltsnap-replay-test."))
    print(f"artifacts: {directory}", flush=True)
    source = directory / "source.nut"
    ffmpeg("-f", "lavfi", "-i", "testsrc2=size=640x360:rate=60",
           "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000",
           "-t", "12", "-c:v", "libx264", "-threads:v", "2", "-preset", "ultrafast",
           "-crf", "23", "-g", "60", "-keyint_min", "60", "-sc_threshold", "0",
           "-bf", "0", "-flags", "+cgop", "-c:a", "aac", "-b:a", "128k",
           "-f", "nut", "-write_index", "0", source)

    def probe(input_path, output_name, seconds=3, memory=8, success=True):
        result = run([worker, "probe", "--input", str(input_path), "--output", str(directory / output_name),
                      "--seconds", str(seconds), "--memory-mib", str(memory), "--closed-gop"], success=success)
        return json.loads(result.stdout) if success else result

    report = probe(source, "clip.mkv")
    clip = directory / "clip.mkv"
    assert report["duration_us"] == 3_000_000, report
    assert report["evicted_gops"] == 9, report
    assert report["peak_accounted_bytes"] <= report["budget_bytes"], report
    ffmpeg("-xerror", "-i", clip, "-f", "null", "-")
    assert frame_hashes(clip) == frame_hashes(source, "select=gte(n\\,540)")

    original_audio = audio_packets(source)
    saved_audio = audio_packets(clip)
    saved_hashes = [packet["data_hash"] for packet in saved_audio]
    selected_audio = [packet for packet in original_audio
                      if float(packet["pts_time"]) * 1e6 < report["end_us"]
                      and (float(packet["pts_time"]) + float(packet["duration_time"])) * 1e6 > report["start_us"]]
    assert [packet["data_hash"] for packet in selected_audio] == saved_hashes
    assert abs(float(selected_audio[0]["pts_time"]) * 1e6 - report["start_us"]) < 22_000

    between_keys = directory / "between-keys.nut"
    ffmpeg("-i", source, "-map", "0:v", "-c", "copy", "-frames:v", "690", between_keys)
    shortened = probe(between_keys, "between-keys.mkv")
    assert shortened["duration_us"] == 2_500_000, shortened
    ffmpeg("-xerror", "-i", directory / "between-keys.mkv", "-f", "null", "-")
    assert frame_hashes(directory / "between-keys.mkv") == frame_hashes(source, "select=gte(n\\,540)*lt(n\\,690)")

    before = hashlib.sha256(clip.read_bytes()).digest()
    probe(source, "clip.mkv", success=False)
    assert hashlib.sha256(clip.read_bytes()).digest() == before

    pressure = probe(source, "pressure.mkv", seconds=10, memory=1)
    assert 0 < pressure["duration_us"] < 10_000_000, pressure
    assert pressure["peak_accounted_bytes"] <= pressure["budget_bytes"], pressure
    ffmpeg("-xerror", "-i", directory / "pressure.mkv", "-f", "null", "-")

    reordered = directory / "reordered.nut"
    ffmpeg("-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30", "-t", "2",
           "-c:v", "libx264", "-threads:v", "2", "-g", "30", "-bf", "2", reordered)
    rejected = probe(reordered, "reordered.mkv", success=False)
    assert b"frame reordering" in rejected.stderr or b"no DTS" in rejected.stderr
    assert not (directory / "reordered.mkv").exists()

    producer = subprocess.Popen(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin",
                                 "-i", str(source), "-c", "copy", "-f", "nut", "-write_index", "0",
                                 "-syncpoints", "none", "-strict", "experimental",
                                 "-flush_packets", "1", "pipe:1"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        piped = subprocess.run([worker, "probe", "--input", "-", "--output", str(directory / "pipe.mkv"),
                                "--seconds", "3", "--memory-mib", "8", "--closed-gop"],
                               stdin=producer.stdout, capture_output=True, timeout=30)
        producer.stdout.close()
        producer_code = producer.wait(timeout=5)
        producer_error = producer.stderr.read()
        assert producer_code == 0, producer_error
    finally:
        if producer.poll() is None:
            producer.kill()
        producer.wait(timeout=5)
        producer.stdout.close()
        producer.stderr.close()
    assert piped.returncode == 0, piped.stderr
    assert frame_hashes(directory / "pipe.mkv") == frame_hashes(clip)

    if args.vulkan:
        gpu = ["-init_hw_device", "vulkan=vk:0", "-filter_hw_device", "vk"]
        decode = [*gpu, "-hwaccel", "vulkan", "-hwaccel_output_format", "vulkan"]
        raw_args = ["-map", "0:v", "-frames:v", "1"]
        cpu_hash = ffmpeg("-i", clip, *raw_args, "-vf", "crop=320:180:64:48,format=yuv420p",
                          "-f", "hash", "-hash", "sha256", "-").stdout
        gpu_hash = ffmpeg(*decode, "-i", clip, *raw_args, "-vf",
                          "crop=320:180:64:48,scale_vulkan=w=320:h=180:scaler=nearest,hwdownload,format=nv12,format=yuv420p",
                          "-f", "hash", "-hash", "sha256", "-").stdout
        assert cpu_hash == gpu_hash, (cpu_hash, gpu_hash)
        cropped = directory / "crop.mkv"
        ffmpeg(*decode, "-i", clip, "-vf", "crop=320:180:64:48,scale_vulkan=w=320:h=180:scaler=nearest",
               "-c:v", "h264_vulkan", "-qp", "22", "-bf", "0", "-c:a", "copy", cropped)
        ffmpeg("-xerror", "-i", cropped, "-f", "null", "-")
        streams = json.loads(run(["ffprobe", "-v", "error", "-show_streams", "-of", "json", str(cropped)]).stdout)["streams"]
        video = next(stream for stream in streams if stream["codec_type"] == "video")
        assert (video["width"], video["height"]) == (320, 180)
        assert [p["data_hash"] for p in audio_packets(cropped)] == saved_hashes

    (directory / "report.json").write_text(json.dumps({"normal": report, "between_keys": shortened, "pressure": pressure,
                                                    "vulkan_tested": args.vulkan}, indent=2))
    print("passed: decode, frame identity, audio identity, time window, memory pressure, no overwrite, rejected B-frames, stream input"
          + (", Vulkan crop identity and encode" if args.vulkan else ""))


if __name__ == "__main__":
    main()
