# Full-screen capture benchmark — 2026-07-21

This is a local end-to-end timing of full-desktop capture to a PNG file. It is
not a selector, editor, or feature benchmark.

## Test machine

- Arch Linux, kernel `7.1.3-zen1-2-zen`
- Hyprland 0.55.4, Wayland
- Intel Core i5-12400F, NVIDIA GeForce RTX 4060
- Two 1920 x 1080 outputs arranged as one 3840 x 1080 desktop
- Boltsnap commit `d20fab8ba0aa39bf2f62bc4da93cdf74d631654d`
- hyperfine 1.20.0, 3 warmups and 20 measured runs
- Output directory: `/tmp` (tmpfs)

All four commands produced a non-empty 3840 x 1080 PNG after the run.

## Results

| Tool | Mean ± standard deviation | Median | Min | Max |
|------|--------------------------:|-------:|----:|----:|
| Boltsnap 1.0.0 | 67.1 ± 4.0 ms | 67.4 ms | 57.3 ms | 75.0 ms |
| Wayshot 1.5.0 | 235.8 ± 6.6 ms | 235.4 ms | 225.4 ms | 248.7 ms |
| grim 1.5.0 | 274.6 ± 9.2 ms | 271.9 ms | 265.0 ms | 303.5 ms |
| Flameshot 14.0.0 | 652.1 ± 11.4 ms | 648.4 ms | 637.0 ms | 681.1 ms |

## Command

```sh
hyperfine --warmup 3 --runs 20 \
  --command-name 'Boltsnap 1.0.0' \
  'target/release/boltsnap full --no-copy -o /tmp/boltsnap-benchmark-20260721/boltsnap.png' \
  --command-name 'Flameshot 14.0.0' \
  'flameshot full --raw > /tmp/boltsnap-benchmark-20260721/flameshot.png' \
  --command-name 'grim 1.5.0' \
  'grim /tmp/boltsnap-benchmark-20260721/grim.png' \
  --command-name 'Wayshot 1.5.0' \
  'wayshot --silent /tmp/boltsnap-benchmark-20260721/wayshot.png'
```

## Raw wall-clock times

Values are seconds, in run order.

```json
{
  "boltsnap_1.0.0": [
    0.0573345598, 0.0666472248, 0.0704815478, 0.0632268388,
    0.0671336928, 0.0650528388, 0.0676594768, 0.0749938368,
    0.0706084848, 0.0661588078, 0.0614081178, 0.0729604498,
    0.0696035408, 0.0687553578, 0.0658616998, 0.0691340898,
    0.0676334478, 0.0652802688, 0.0640604158, 0.0687229278
  ],
  "flameshot_14.0.0": [
    0.6450621678, 0.6501787268, 0.6810828108, 0.6469756628,
    0.6707573288, 0.6370038128, 0.6531640988, 0.6440905488,
    0.6506512338, 0.6637735298, 0.6479821768, 0.6593649878,
    0.6436758238, 0.6481457148, 0.6408688268, 0.6711431948,
    0.6462840408, 0.6440611428, 0.6486417648, 0.6487958408
  ],
  "grim_1.5.0": [
    0.2766555778, 0.2655479858, 0.2778396238, 0.2693206008,
    0.2649574358, 0.2755072468, 0.2730151878, 0.2686352048,
    0.2702557678, 0.2716142528, 0.2870761428, 0.2768685638,
    0.2665758608, 0.2832852908, 0.2721580978, 0.3035270498,
    0.2696053828, 0.2715091038, 0.2822518568, 0.2661550898
  ],
  "wayshot_1.5.0": [
    0.2301227438, 0.2487479728, 0.2273851008, 0.2277791428,
    0.2440962408, 0.2422097958, 0.2451010138, 0.2354697828,
    0.2356088518, 0.2287801018, 0.2416311958, 0.2332306148,
    0.2314900688, 0.2353842698, 0.2309946468, 0.2385925158,
    0.2411910088, 0.2322798308, 0.2398792978, 0.2253856078
  ]
}
```

PNG compression cost depends on what is visible on screen, so these numbers
describe this machine and desktop state. Re-run the command before using them
to make a broader claim.
