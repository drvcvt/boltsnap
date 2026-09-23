#!/usr/bin/env python3
"""Opt-in headless GPU test. Synthetic buffers only, never desktop capture."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--worker', required=True, type=Path)
p.add_argument('--node', required=True, type=Path)
a = p.parse_args()
root = Path(__file__).resolve().parents[1]
out = Path(tempfile.mkdtemp(prefix='boltsnap-native-test.'))
env = dict(os.environ, VK_INSTANCE_LAYERS='VK_LAYER_KHRONOS_validation')

def run(args):
    result = subprocess.run([str(s) for s in args], env=env, capture_output=True, timeout=90)
    with (out / 'commands.log').open('ab') as log:
        log.write(result.stderr)
    assert result.returncode == 0, result.stderr.decode(errors='replace')
    assert b'Validation Error' not in result.stderr, result.stderr.decode(errors='replace')
    return result.stdout

flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'egl', 'glesv2', 'gbm', 'vulkan', 'libavutil', 'libavfilter', 'libavformat', 'libavcodec'], text=True).split()
run(['cc', '-std=c11', '-D_GNU_SOURCE', '-O2', '-Wall', '-Wextra', '-Werror', root / 'native/gpu.c', root / 'native/test_gpu.c', '-o', out / 'pixels', *flags, '-lm'])
run([out / 'pixels', a.node, 'libx264', out / 'dma.nut'])
result = json.loads(run([a.worker, 'cursor-fixture', a.node, 'libx264', out / 'motion.nut']))
assert result['frames'] == 120
# Existing output must remain byte-identical after a repeated start attempt.
before = (out / 'motion.nut').read_bytes()
retry = subprocess.run([str(a.worker), 'cursor-fixture', str(a.node), 'libx264', str(out / 'motion.nut')], capture_output=True, timeout=10)
assert retry.returncode != 0 and (out / 'motion.nut').read_bytes() == before
for fps, codec in [('240', 'libx264'), ('60', 'h264_vulkan')]:
    destination = out / f'rejected-{fps}-{codec}.mp4'
    rejected = subprocess.run([str(a.worker), 'cursor-record', 'TEST-0', fps, codec, '-', str(destination)], env={'PATH': os.environ['PATH']}, capture_output=True, timeout=10)
    assert rejected.returncode != 0 and not destination.exists()
    assert b'60 FPS only' in rejected.stderr or b'requires explicit libx264' in rejected.stderr
raw = run(['ffmpeg', '-v', 'error', '-i', out / 'motion.nut', '-f', 'rawvideo', '-pix_fmt', 'rgb24', 'pipe:1'])
size = 320 * 180 * 3
assert len(raw) == size * 120
for index in range(120):
    frame = raw[index * size:(index + 1) * size]
    assert all(abs(x-y) <= 4 for x, y in zip(frame[:3], [16, 32, 48]))
    if 10 <= index <= 110:
        xs = [x for x in range(320) if frame[(88*320+x)*3] > 70]
        assert xs and abs(sum(xs)/len(xs) - 2*index) <= 1.2, (index, xs)
        center = (88*320+2*index)*3
        assert all(abs(x-y) <= 7 for x, y in zip(frame[center:center+3], [136, 16, 24]))
        # No second cursor or unrelated changed background in this scanline.
        assert max(xs) - min(xs) < 19
print(f'GPU DMA-BUF lifetime, alpha/edges/visibility and 120 decoded interpolated frames passed: {out}')
