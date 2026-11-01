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

## Zeitachse eines Schusses (WAYLAND_DEBUG, nur der CLI-Prozess)

Ein `boltsnap full -o`, Zeiten aus dem Protokoll-Log (Debug-Ausgabe bremst,
die Verhältnisse stimmen):

| Phase | Dauer |
| --- | ---: |
| erste Verbindung: Registry, Bind, Outputs (ein `sync` allein 3,5 ms) | ~5,4 ms |
| zwei weitere Verbindungen, je eine pro Monitor (`compose.rs:114-148`): Registry, Outputs, ein `sync` von 9,6 ms | ~12,3 ms |
| `capture` → `ready`, Monitor 1 / Monitor 2 | 9,5 / 15,6 ms, nacheinander |
| Zusammensetzen in die Leinwand, Layout-Prüfung | ~4,3 ms |

Befunde:
- Gut die Hälfte der „Aufnahme“ ist Setup und Warten auf Roundtrips, kein
  Kopieren. Die Extra-Verbindungen gibt es, weil libway pro Verbindung nur
  eine laufende Aufnahme kennt (`capture.rs:150`, `pending:
  Option<Pending>`); Hyprland kopiert die Monitore trotzdem nacheinander
  (zweites `ready` 6,3 ms nach dem ersten), die Parallelität bringt also
  nichts.
- Hyprland bietet shm XRGB/ARGB (im Speicher BGRX) und dmabuf nur mit
  NVIDIA-Block-Linear-Modifiern bzw. `INVALID`, kein `LINEAR`: ein
  dmabuf-Weg müsste auf der GPU entkacheln und zurücklesen.
- libway wandelt BGRX nach RGBA (`buffer.rs:177`), Boltsnap danach nach RGB
  (`capture/mod.rs:66`): zwei volle Durchläufe über 16 MB.

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

## Recherche (01.11.)

Belege aus der Web-Recherche (Upstream-Quellen, ohne Zeilennummern):
- Hyprland, `src/managers/screenshare/ScreenshareFrame.cpp`: `copyShm()`
  rendert in einen Offscreen-Framebuffer und liest mit `readPixels` zurück;
  `src/render/gl/GLFramebuffer.cpp` `readPixels` ist ein synchrones
  `glReadPixels` ohne PBO, **zeilenweise, wenn der Stride nicht passt**.
  `copyDmabuf()` rendert direkt in den Client-dmabuf, Abschluss per Callback,
  kein Zurücklesen. Kopiert wird je Monitor auf dessen nächstem Commit
  (`ScreenshareManager::onOutputCommit`), ein einfaches `copy` erzwingt
  dafür einen neuen Frame (`damageMonitor`).
- Schnelle PNG-Encoder: fpnge (AVX2, 180 bis 800 MP/s je Kern), fpng
  (RLE-Treffer, vorberechnete Huffman-Tabellen, Filter Up), mtpng (parallel
  wie pigz, Streifen mit Sync-Flush, kombinierte Adler-32; Größe „winzig“
  schlechter als libpng). Keine fpnge-Portierung für Rust gefunden.
  Streifen mit Sync-Flush plus `adler32_combine` sind ein gültiges,
  bekanntes Verfahren.

Folgen für den Plan: keine Leinwand mit Desktop-Stride (zeilenweises
`glReadPixels` in Hyprland), Puffer je Monitor dicht gepackt; dmabuf nur als
Probe.

## Plan

Ziel: `boltsnap full -o` von 57,6 ms (Median) so weit runter wie möglich,
gleiche Bilder, Dateigröße höchstens +10 %. Schätzung, nicht gemessen:
E1 −18 ms, L1 −10 ms, L2 −6 ms, also Richtung 25 ms.

Regeln für jeden Schritt: ein Commit je Schritt mit Test, Dateien unter
500 Zeilen nach Konzept, kein `unsafe` außer dem, was libway für shm schon
hat, Diff vor jedem Commit gelesen, `cargo fmt --check` und `cargo test`
(Root und libway), danach hyperfine alt gegen neu mit Median und Größe hier.

**E1, eigener paralleler PNG-Encoder** `boltsnap::image_model::png`
(`src/image/png.rs`, Tests in `src/image/png/tests.rs`; `src/image.rs` wird
`src/image/mod.rs`, `src/` wächst nicht). Entscheidung: selbst statt
`mtpng`, weil der Nutzer es selbst will, `mtpng` libz braucht und der Kern
klein ist.
- Eingabe: Zeilenquelle aus einer oder mehreren Kacheln (Pixel, Stride,
  Format RGB, RGBA oder BGRX, Position), Lücken schwarz; Ausgabe 8-bit RGB.
  Umwandlung passiert beim Filtern, kein Zwischenpuffer.
- Streifen je Kern, mindestens 32 Zeilen; je Streifen adaptiver Filter (5
  Filter, kleinste Betragssumme) und ein dynamischer Deflate-Block mit
  Literalen und Läufen eines Bytes (Abstand 1), Huffman aus dem Histogramm,
  15 Bit Grenze. Nicht letzte Streifen enden mit leerem Stored-Block.
  Ein IDAT je Streifen mit eigener CRC (`crc32fast`), zlib-Kopf im ersten,
  Adler-32 kombiniert in einem letzten IDAT.
- Vergleich im Test (ignoriert, lokal): Filter adaptiv gegen nur Up, Größe
  und Zeit am echten Bild; die schnellere Variante bleibt, wenn sie höchstens
  +10 % Größe kostet.
- Tests: Rundreise über den `image`-Decoder (Rauschen, Flächen, Verläufe,
  1 × 1, 1 × N, N × 1, weniger Zeilen als Kerne, alle Formate, zwei Kacheln
  mit Lücke); zlib-Strom samt Adler mit `miniz_oxide` (dev-dependency)
  geprüft; CRC jedes Chunks.

**E2, Aufrufer** (alle Linux): `capture/mod.rs` `capture` und
`capture_png`, `main.rs` Standardweg zum Shelf, `shelf/cards/cards.rs`
(Daemon, Raw-Weg). Windows und Test-Screenshots bleiben beim `image`-Encoder.

**L1, libway: alle Monitore auf einer Verbindung.** `State.pending` wird
eine Map je Token, `compose` fordert alle Frames auf der eigenen Verbindung
an und wartet auf alle; die Verbindungen pro Monitor
(`compose.rs:114-148`) fallen weg. Tests im libway-Crate mit seinem
Test-Server: zwei Outputs, beide Frames auf einer Verbindung, Fehler eines
Frames bricht beide ab.

**L2, libway: Rohbild ohne Zusammensetzen.** Neue API
`capture_desktop_tiles` gibt die Monitore als dicht gepackte Kacheln im
Server-Format (BGRX/BGRA) mit Position zurück, wenn alle die gleiche
Skalierung und keine Drehung haben; sonst `None`, und Boltsnap nimmt den
alten Weg. Spart die Leinwand-Allokation, die Kopie und beide
Formatwandlungen. Nur für `full` und den Standardweg zum Shelf; der
Selector bleibt beim `RgbaImage`.

**P4, dmabuf-Probe** (isoliert, kein Produktcode, fail-closed): Zeit von
`capture` bis `ready` mit dmabuf gegen shm, plus eigenes Zurücklesen per
Vulkan oder EGL. Nur wenn die Summe deutlich unter shm liegt, eigener Plan.

**Später, eigener Plan:** Aufnahme im Daemon mit warmer Verbindung (spart
die ~5 ms der ersten Verbindung, braucht eine IPC-Änderung).

Was brechen kann: Decoder, die den Strom nicht mögen (zwei Decoder im
Test); Monitore mit Skalierung, Drehung oder Lücken (L2 fällt auf den alten
Weg zurück, Test); ein Fehler eines Frames bei mehreren auf einer Verbindung
(L1-Test); CPU-Spitze über alle Kerne für wenige ms je Screenshot.

Nicht: kein JPEG/QOI statt PNG, keine niedrigere PNG-Qualität, kein
Dauer-Capture im Hintergrund (kostet GPU, blockiert Direct Scanout).

Abnahme je Schritt: Tests grün; hyperfine `-o` gegen den Stand davor;
am Ende ein Screenshot der echten Ausgabe angesehen und gegen das alte PNG
pixelgleich geprüft.
