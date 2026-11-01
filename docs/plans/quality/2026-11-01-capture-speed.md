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

## Umsetzung E1 (01.11.)

- Erster Entwurf (Token je Byte, 5 volle Filter-Puffer, 32-Bit-Bitpuffer,
  eigene Adler-Schleife) war korrekt und 9 % kleiner als `image` (2,12 statt
  2,34 MB am echten 3840 × 1080 Bild), aber einthreadig 61 ms gegen 26 ms
  (Filter 16,7, Tokenisieren 26, Schreiben 14 ms); mit 12 Threads 14 ms,
  über 6 Threads skaliert es kaum (6 physische Kerne).
- **Abweichung in der Umsetzung:** Kern wie fdeflate gebaut (gelesen:
  `fdeflate-0.3.7/src/compress.rs`): Filterkosten ohne Zwischenpuffer in
  vektorisierbaren Schleifen, Abbruch bei Kosten 0; Deflate scannt in
  Spannen (Literale, Null-Läufe ab 4) mit `u64`-Sprüngen, Histogramm über
  die Spannen, 64-Bit-Bitpuffer mit 4 Literalen je Schreibzugriff; Läufe
  nur noch aus Nullen (nach dem Filtern sind gleiche Flächen Nullen).
  Huffman bleibt dynamisch je Streifen. Adler-32 über `simd-adler32` (schon
  im Baum über `fdeflate`), weil eigene SIMD `unsafe` bräuchte; das
  Kombinieren bleibt eigener Code.
- Gemessen am echten Bild (Filter einthreadig): nur Up 1,6 ms / 2,66 MB,
  nur Sub 1,6 ms / 2,45 MB, **nur Paeth 4,8 ms / 2,21 MB**, adaptiv 51 ms /
  2,16 MB (`image` 2,34 MB). **Entscheidung: nur Paeth**, weil kleiner als
  heute bei einem Zehntel der Filterzeit; der Kostenvergleich entfällt.
  Null-Läufe werden wie in fdeflate je 8-Byte-Block erkannt (Ende eines
  Blocks, volle Nullblöcke, Anfang des nächsten), Bytezähler einmal je
  Streifen in vier Tabellen (vorher je Spanne neu angelegt: 29,6 ms).
- **Ergebnis E1** (Release, normale Priorität, 3840 × 1080 Desktop):
  1 Thread 25,6 ms, 6 Threads 7,5 ms, 12 Threads 6,0–6,8 ms, 2,15 MB;
  `image` 21–24 ms, 2,34 MB. Etwa 3,7× schneller, 8 % kleiner.
  Tests: Rundreise über `image`-Decoder und `miniz_oxide` (zlib, Adler,
  CRC je Chunk) für 1 × 1 bis 1920 × 33, Flächen, Rauschen, RGB/RGBA/BGRX,
  Kacheln mit Stride und Lücke; Paeth gegen die Spec; Spannen und
  Lauf-Zerlegung. `cargo test` 186 + 428 grün.

## Umsetzung E2 (01.11.)

- Wie geplant, vier Stellen: `capture` (nur bei Endung `.png`, sonst
  `image` wie bisher, damit `-o x.jpg` JPEG bleibt), `capture_png`,
  `main.rs` Standardweg zum Shelf, `cards.rs` (Daemon, Raw-Weg).
- Abweichung: `encode` gibt `Result` statt `assert!`, weil der Daemon
  Breite und Höhe vom Client bekommt und eine Panik ihn beenden würde (der
  alte `PngEncoder` lieferte dort einen Fehler). Test
  `sizes_and_tiles_that_do_not_fit_are_errors_not_panics`.
- hyperfine `full --no-copy -o`, 3 Warmups, 20 Läufe, vorher (`fbc5ea4`)
  gegen nachher: **Median 60,5 → 43,4 ms** (Mittel 61,7 → 46,8), Datei 2,7
  → 2,4 MB. Das Ausgabe-PNG dekodiert mit `image` pixelgleich (Speed-Test
  mit `BOLTSNAP_PNG_SAMPLE` auf die echte Datei). `cargo test` 187 + 428.

## Ziel unter 15 ms (Nutzer, 01.11.: „wir muessen unter 15ms kommen wenn moeglich oder noch lower“)

Stand nach E2: 43,4 ms Median. Untergrenze je Teil (gemessen, wo nicht als
Schätzung markiert):
- Prozess und Verbindungen ~17 ms → ~2 ms mit L1 und Aufnahme im Daemon
  (warme Verbindung, gebundene Outputs, vorab angelegte Puffer; die CLI
  fragt nur per IPC an). Prozessstart allein 1,5 ms (gemessen).
- Compositor-Kopie mit shm ~16 ms (beide `ready` nach 9,5/15,6 ms) und
  **nicht drückbar**: synchrones `glReadPixels` ohne PBO, Monitore
  nacheinander. Mit dmabuf rendert Hyprland asynchron je Monitor auf dessen
  Commit (Schätzung 5–8 ms für beide), dazu eigenes Zurücklesen von der GPU
  (Schätzung 2–3 ms; NVIDIA bietet kein LINEAR).
- Zusammensetzen und Farbwandlung ~6 ms → 0 mit L2.
- PNG ~7 ms → Schätzung 4–5 ms mit weiterer Arbeit am Kern.

Folgerung: mit shm bleibt es bei ~22–24 ms. Unter 15 ms nur mit dmabuf und
eigenem Zurücklesen; für Shelf und Clipboard zusätzlich PNG aus dem
kritischen Pfad (Karte aus Rohpixeln, PNG direkt danach). P4 wird damit die
entscheidende Probe und geht vor L2.

Reihenfolge neu: L1 → P4 (Probe dmabuf) → je nach Ergebnis dmabuf-Weg oder
L2 → D1 Aufnahme im Daemon (eigener Plan, IPC) → PNG aus dem kritischen Pfad
(eigener Plan) → Feinschliff am PNG-Kern.

## D1: Vollbild-Screenshot im Daemon (Plan, 01.11.)

Messung Shelf-Weg heute (`boltsnap full --no-copy`, `BOLTSNAP_TIMINGS`,
Mittel aus 5): Capture 32,4 ms, RGB 2,1 ms, Karten-Vorschau 3,1 ms, PNG
12,4 ms (frischer Prozess: kalte Threads und Seiten), `last.png` schreiben
~7 ms, PNG an den Daemon und Bestätigung nach dem Zeichnen 16,7 ms; die
CLI ist nach ~74 ms fertig. Daemon-seitig: `shelf_png_prepare` 0,7–4,7 ms.

Verhalten:
- `boltsnap full` (Shelf-Weg, mit und ohne Kopie) und `full -o DATEI`
  fragen den laufenden Daemon: neuer Befehl `capture` mit `output: Option
  <PathBuf>` (absolut, von der CLI aufgelöst) und `copy: bool`.
- Der Daemon nimmt auf einem eigenen Capture-Thread auf, mit einer
  dauerhaft offenen libway-Verbindung (beim ersten Mal aufgebaut; bei einem
  Fehler einmal neu verbunden, dann Fehler an die CLI).
- Shelf-Weg: Vorschau aus den Rohpixeln, die Karte erscheint sofort im
  Zustand „wird geschrieben“ (wie Replay-Clips, `cards/pending.rs`); PNG,
  Temp-Datei, `last.png` und die Kopie ins Clipboard laufen danach im
  Hintergrund; Ziehen, Kopieren und Speichern der Karte warten bis dahin
  (gibt es schon für Replay-Clips). Antwort an die CLI, sobald die Karte
  steht.
- `-o`: der Daemon schreibt die Datei und antwortet erst dann.
- Kein Daemon, alter Daemon („unknown cmd“) oder Capture-Fehler vor der
  Aufnahme: die CLI nimmt wie heute selbst auf. Ein Fehler nach der Aufnahme
  geht als Fehler zurück (keine doppelte Aufnahme).
- Nicht: `area`/`window` (Selector läuft in der CLI; eigener Plan D2 mit
  dem eingefrorenen Desktop vom Daemon), Windows, X11, Portal (bleiben beim
  heutigen Weg).

Umsetzung:
1. `protocol.rs`: `Request::Capture { output, copy }`, Kodierung als `cmd:
   "capture"`, Rundreise-Test.
2. `shelf/cards/capture.rs` (neu, `cards/` hat 6 Dateien): Capture-Thread
   mit Kanal, warme Verbindung, Rückgabe der Rohpixel; Daemon-Event
   „Screenshot fertig“ in den Event-Loop (`DaemonEvent`).
3. Karte aus Rohpixeln: Vorschau wie `make_rgb_card_thumbnail`, Karte
   `pending`; PNG-Thread schreibt Temp-Datei und `last.png`, dann Karte
   fertig und ggf. Clipboard.
4. CLI (`main.rs` `capture_flow`, Linux, Wayland, Modus `full`): erst
   `capture` an den Daemon, Rückfall wie oben.
5. Tests: Protokoll; Rückfall bei „unknown cmd“ und bei fehlendem Daemon;
   Karte bleibt `pending`, bis das PNG geschrieben ist; Fehler nach der
   Aufnahme löst keine zweite Aufnahme aus.

Messung danach: hyperfine `full --no-copy` und `full -o` gegen vorher; Zeit
bis Karte sichtbar über `process_entry` → `shelf_commit`.

Risiken: Daemon-Thread mit offener Wayland-Verbindung (Ereignisse müssen
gelesen werden, sonst läuft der Puffer voll: der Thread pumpt die
Verbindung bei jedem Auftrag und verbindet bei Fehlern neu); Speicher
(Rohbild 16 MB nur während eines Auftrags); `shelf/` wächst nicht.

## Umsetzung D1a (01.11.)

- Gebaut wie geplant, in zwei Schritten statt einem: **D1a** nimmt im Daemon
  auf und schreibt PNG, `last.png` und Clipboard noch vor der Karte
  (`prepare_pixels` nimmt jetzt `&[u8]` und wird wiederverwendet); **D1b**
  (Karte vor dem PNG) folgt.
- `Request::Capture { output, copy }` (`cmd: "capture"`),
  `CAPTURE_UNAVAILABLE`; Daemon: `shelf/cards/screenshot.rs` mit warmer
  libway-Verbindung (einmal neu verbunden bei Fehler), Arm im Client-Worker
  (`clients.rs`) mit `same_user` und Bild-Permit; CLI:
  `capture::full_in_daemon` und `ipc::capture_in_daemon` (für eine Datei
  wird kein Daemon gestartet, nur ein laufender gefragt; nicht-PNG-Endungen
  bleiben lokal).
- Windows: ein Arm `Request::Capture` → „capture unavailable: not on
  Windows“, nötig für den erschöpfenden Match. **Kein `cargo check --target
  x86_64-pc-windows-msvc`**: das Target ist hier nicht installiert.
- `main.rs` 979 Zeilen (vorher 965): unter der Grenze, braucht aber bald
  einen Split nach Konzept (Capture-Ablauf aus `main.rs` heraus).
- Tests: Protokoll-Rundreise, `daemon_capture` (EOF und
  `CAPTURE_UNAVAILABLE` → lokal; Fehler nach der Aufnahme → kein zweiter
  Schuss). `cargo test` 188 + 429.
- Live (Nutzer-Go für Aufnahmen, YouTube lief nebenher): hyperfine Median
  `-o` 53,3 → 49,6 ms, Shelf-Weg 72,8 → 66,1 ms. Im Daemon: Aufnahme
  ~23 ms (libway öffnet weiter zwei Verbindungen je Schuss, erst L1 hilft),
  PNG 6,5–21 ms (Mittel 10,9, Last), Datei schreiben ~5,6 ms.
- Messpunkte `daemon_capture`, `daemon_capture_output`, `png_encode` (in
  `capture::encode_png`) bleiben; ohne `BOLTSNAP_TIMINGS` kosten sie nichts.
