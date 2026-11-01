# Vollbild-Screenshot: wo die Zeit hingeht (2026-11-01)

Status: **Plan, wird umgesetzt (Nutzer 01.11.: „so schnell es geht, wir machen so viel wie moeglich selber“).** Frage des Nutzers: „wie
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

## Plan

Entscheidung: eigener paralleler PNG-Encoder statt `mtpng`, weil der Nutzer
so viel wie möglich selbst will und der Kern klein ist (Filter, ein
Deflate nur mit Literalen und Läufen, Huffman je Streifen); `crc32fast`
(schon im Baum über `png`) für die Prüfsummen, weil std keine CRC hat und
eigene Tabellen langsamer wären als dessen PCLMUL-Weg.

**P1, Encoder** `boltsnap::image_model::png` (Datei `src/image/png.rs`;
`src/image.rs` wird dafür `src/image/mod.rs`, `src/` wächst nicht, hat
schon 17 Einträge).
- `encode_rgb(pixels, width, height) -> Vec<u8>` und `encode_rgba_as_rgb`
  (Alpha fällt beim Filtern weg, kein Zwischenpuffer). Ausgabe immer
  8-bit RGB, wie heute.
- Zeilen in Streifen, einer je Kern (`available_parallelism`), mindestens
  32 Zeilen je Streifen; `std::thread::scope`.
- Je Streifen: Filter je Zeile adaptiv (kleinste Summe der Beträge, wie
  `image` Fast), Deflate als ein dynamischer Block: Literale plus Läufe
  gleicher Bytes (Abstand 1, Länge 3 bis 258), Huffman aus dem Histogramm
  des Streifens, auf 15 Bit begrenzt (Häufigkeiten halbieren, bis es passt).
  Nicht der letzte Streifen endet mit einem leeren Stored-Block
  (Sync-Flush), also byte-genau; der letzte setzt BFINAL.
- Jeder Streifen ist ein eigener IDAT-Chunk mit eigener CRC (parallel); der
  erste trägt den zlib-Kopf, ein letzter kleiner IDAT die Adler-32, aus den
  Streifen-Teilsummen kombiniert.
- Tests: Rundreise über den `image`-Decoder für Rauschen, Flächen,
  Verläufe, 1 × 1, 1 × N, N × 1, weniger Zeilen als Kerne, RGB und RGBA;
  zlib-Strom samt Adler mit `miniz_oxide::inflate::decompress_to_vec_zlib`
  (dev-dependency) geprüft; CRCs je Chunk.

**P2, Aufrufer** (alle Linux): `capture/mod.rs:22` (`capture`, Datei),
`:31-38` (`capture_png`, stdout), `main.rs:621-625` (Standardweg zum
Shelf), `shelf/cards/cards.rs:364` (Daemon, Raw-Weg). Windows und die
Test-Screenshots bleiben beim `image`-Encoder.

**P3, RGB-Umwandlung sparen**: wo kein Rand abgeschnitten wird (`Full`,
`Area`), kodiert `capture` direkt aus RGBA.

**P4, dmabuf-Probe** (nur messen, kein Produktcode): ob eine Aufnahme über
dmabuf statt shm auf NVIDIA schneller ist. Erst nach P1 bis P3.

Was brechen kann: ein Decoder, der den Strom nicht mag (Rundreise-Tests mit
zwei Decodern), Dateigröße (Annahme: ±10 % zu heute, wird gemessen), CPU-
Spitze über alle Kerne für ~5 ms bei jedem Screenshot (Spiel im
Vordergrund: Threads laufen mit normaler Priorität, wie heute der eine).

Abnahme: Tests grün; hyperfine `-o` alt gegen neu, Median und Dateigröße in
dieser Datei; ein Screenshot der echten Ausgabe angesehen.
