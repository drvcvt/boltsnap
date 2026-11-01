# Full-screen capture benchmark — 2026-11-01

This is a local end-to-end timing of full-desktop capture to a PNG file. It is
not a selector, editor, or feature benchmark. It replaces
[the 2026-07-21 run](full-capture-2026-07-21.md) and adds Boltsnap 1.0.0 as a
row, so the gain since then is measured on the same machine and desktop.

## Test machine

- Gentoo Linux, kernel `6.18.57-gentoo-dist-bin`
- Hyprland 0.56.2, Wayland
- Intel Core i5-12400F, NVIDIA GeForce RTX 4060 (driver 615.78.08)
- Two 1920 x 1080 outputs at 200 Hz and 240 Hz, arranged as one
  3840 x 1080 desktop
- Boltsnap at commit `b5d989c` (version 1.0.1 plus unreleased work), with its
  shelf daemon running, which takes the screenshot itself; its replay buffer
  was recording as usual
- Boltsnap 1.0.0 built from commit `d20fab8`
- hyperfine 2.0.0, 3 warmups and 20 measured runs, through `sh`
- Output directory on ext4 (not tmpfs)
- A video was playing during the run; nothing was compiling, load average
  about 1.7

All five commands produced a 3840 x 1080 PNG of 2.0 to 2.5 MB after the run.

## Results

| Tool | Mean ± standard deviation | Median | Min | Max | Time vs. Boltsnap |
|------|--------------------------:|-------:|----:|----:|------------------:|
| Boltsnap (`b5d989c`) | 28.8 ± 5.6 ms | 29.2 ms | 20.5 ms | 40.0 ms | 1.00x |
| Boltsnap 1.0.0 | 66.8 ± 6.0 ms | 65.2 ms | 58.9 ms | 79.3 ms | 2.32x |
| Wayshot 1.6.0 | 598.2 ± 18.2 ms | 593.6 ms | 575.8 ms | 665.1 ms | 20.73x |
| grim 1.5.0 | 726.4 ± 17.7 ms | 720.3 ms | 706.1 ms | 761.7 ms | 25.18x |
| Flameshot 13.3.0 | 828.1 ± 13.3 ms | 823.8 ms | 810.1 ms | 850.3 ms | 28.70x |

## Command

```sh
hyperfine --shell=sh --warmup 3 --runs 20 \
  --command-name 'Boltsnap' \
  'boltsnap full --no-copy -o out/boltsnap.png' \
  --command-name 'Boltsnap 1.0.0' \
  'boltsnap-1.0.0 full --no-copy -o out/old.png' \
  --command-name 'Flameshot 13.3.0' \
  'flameshot full --raw > out/flameshot.png' \
  --command-name 'grim 1.5.0' \
  'grim out/grim.png' \
  --command-name 'Wayshot 1.6.0' \
  'wayshot --silent out/wayshot.png'
```

Flameshot can only capture on Hyprland through its grim adapter, set in
`~/.config/flameshot/flameshot.ini`:

```ini
[General]
useGrimAdapter=true
disabledGrimWarning=true
showDesktopNotification=false
```

Wayshot 1.6.0 shows a desktop notification after each screenshot unless it
runs with `--silent`.

## Raw wall-clock times

Values are seconds, in run order.

```json
{
  "boltsnap_b5d989c": [
    0.0321, 0.0356, 0.0331, 0.0310, 0.0340,
    0.0365, 0.0320, 0.0323, 0.0249, 0.0302,
    0.0229, 0.0205, 0.0236, 0.0232, 0.0230,
    0.0224, 0.0400, 0.0269, 0.0243, 0.0282
  ],
  "boltsnap_1.0.0": [
    0.0661, 0.0718, 0.0611, 0.0679, 0.0634,
    0.0589, 0.0602, 0.0643, 0.0793, 0.0656,
    0.0629, 0.0634, 0.0737, 0.0751, 0.0783,
    0.0675, 0.0613, 0.0648, 0.0696, 0.0610
  ],
  "flameshot_13.3.0": [
    0.8203, 0.8101, 0.8204, 0.8150, 0.8442,
    0.8198, 0.8486, 0.8345, 0.8239, 0.8236,
    0.8238, 0.8110, 0.8431, 0.8168, 0.8238,
    0.8448, 0.8452, 0.8503, 0.8278, 0.8143
  ],
  "grim_1.5.0": [
    0.7178, 0.7551, 0.7206, 0.7384, 0.7173,
    0.7429, 0.7061, 0.7454, 0.7277, 0.7201,
    0.7509, 0.7090, 0.7070, 0.7238, 0.7177,
    0.7617, 0.7085, 0.7396, 0.7109, 0.7069
  ],
  "wayshot_1.6.0": [
    0.5894, 0.5867, 0.5915, 0.5758, 0.5871,
    0.5926, 0.6044, 0.5842, 0.5988, 0.6152,
    0.5927, 0.5853, 0.5995, 0.5989, 0.6060,
    0.5946, 0.5916, 0.5979, 0.6651, 0.6060
  ]
}
```

## Limits

- PNG compression cost depends on what is visible on screen, and a playing
  video makes it higher. The other tools took two to three times as long as
  in July on a different system; only the rows of one run compare.
- Not measured: a single output, other refresh rates (the outputs were left
  at 200 Hz and 240 Hz), and Boltsnap without its daemon. The compositor
  copies each output with its next frame, so at 60 Hz every tool may wait
  up to one frame (16.7 ms) longer; this is an estimate, not a measurement.
- Without the daemon, `boltsnap full -o` captures in its own process. An
  earlier run on this machine measured that path at a median of 43 ms
  (the commit before captures moved into the daemon).
- A run during a parallel release build (load average up to 26, swapping)
  was discarded: Boltsnap ranged from 31 to 112 ms there, and one capture
  failed.
