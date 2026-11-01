# Vollbild-Screenshot: wo die Zeit hingeht (2026-11-01)

Status: **Messung und Optionen, nichts gebaut.** Frage des Nutzers: „wie
koennen wir das noch schneller machen? gpu oder so benutzen?“

## Alt gegen neu, gleiche Maschine

hyperfine 2.0.0, 3 Warmups, 20 Läufe, `boltsnap full --no-copy -o
<scratchpad>/x.png` (3840 × 1080, zwei Monitore, Hyprland 0.56.2, Gentoo
6.18.57, Replay-Puffer lief mit). Beide gegen denselben Daemon.

| Build | Median | Mittel ± σ | Min–Max | CPU (Median) |
| --- | ---: | ---: | ---: | ---: |
| 1.0.0 `d20fab8` (Juli) | 71,3 ms | 76,7 ± 15,4 ms | 62,7–127,9 ms | 51,5 ms |
| 1.0.1 `701629e` (jetzt) | 57,6 ms | 59,3 ± 8,1 ms | 48,5–78,2 ms | 42,8 ms |

Median 1,24× schneller (−19 %). Mit `-o` geht kein Bild ans Shelf
(`main.rs`, `PostCapture::File`).

## Aufschlüsselung

Wegwerf-Test im Bin-Crate (nicht eingecheckt), 5 Läufe, Release:

| Stufe | Zeit |
| --- | ---: |
| `libway::Connection::connect` | 0,3–0,6 ms |
| `capture_desktop` (beide Monitore) | 21,8–31,4 ms |
| nur ein Monitor (1920 × 1080) | 9,5–14,6 ms |
| `rgba_to_rgb` | 1,9–2,6 ms |
| PNG über `write_to` (heutiger Weg, `capture/mod.rs:22`) | 22,5–27,4 ms |
| PNG `Fast` + Adaptive | 19,4–26,1 ms, 2,4 MB |
| PNG `Fast` + Sub | 14,7–21,1 ms, 2,7 MB |
| PNG `Default` | 455–703 ms, 1,9 MB |
| PNG `Best` | 1,6–2,5 s, 1,8 MB |

Rest bis ~57 ms: Prozessstart, Datei schreiben, Ausgabe (~7 ms).

Befunde:
- Die zwei Monitore laufen in libway schon parallel (`compose.rs:114-148`,
  je eigene Verbindung), kosten aber fast das Doppelte von einem: der
  Compositor kopiert sie offenbar nacheinander (GPU → Speicher in Hyprland).
  Annahme, nicht im Hyprland-Code belegt.
- PNG ist ein Kern Deflate. Eine GPU hilft da nicht: Deflate ist seriell,
  es gibt keinen brauchbaren GPU-PNG-Encoder (nvJPEG kann nur JPEG).

## Optionen

1. **PNG parallel kodieren** (größter, sicherer Hebel): Bild in Streifen
   teilen, jeden Streifen auf einem eigenen Kern filtern und deflaten, die
   Teile als ein gültiger zlib-Strom aneinanderhängen (wie `pigz`, `mtpng`).
   Erwartung bei 6 bis 12 Kernen: ~23 ms → ~4–6 ms, Datei gleich groß wie
   heute. Ansatz: Crate `mtpng` oder eigener kleiner Encoder über `fdeflate`
   und `std::thread::scope`. Muss gemessen werden.
2. **RGB-Umwandlung sparen** (~2 ms): RGBA direkt kodieren oder beim
   Zusammensetzen schon RGB schreiben.
3. **GPU-Weg für die Aufnahme** (unsicher): dmabuf statt shm, der Compositor
   kopiert dann nur GPU → GPU, das Zurücklesen machen wir selbst (EGL/GBM,
   NVIDIA). Ob das schneller ist, ist offen; nur mit Probe.
4. Nicht: PNG-Stufe senken (`Fast` ist schon die schnelle; `Sub` spart 5 ms
   für 12 % größere Dateien).
