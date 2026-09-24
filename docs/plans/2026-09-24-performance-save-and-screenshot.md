# Performance: Speichern von Aufnahmen und Screenshot-Pfad, 2026-09-24

Status: freigegeben 2026-09-24, Umsetzung läuft (siehe Status-Tabelle).
Nur Linux; das Windows-Backend bleibt eingefroren.

## 1. Zusammenfassung

Der User wollte wissen, wo Boltsnap noch schneller werden kann. Eine erste
leichte Messung (ffprobe 31-53 ms pro Aufruf, `boltsnap full` Capture 34-47 ms)
und die offenen Punkte aus dem Review vom 23.09.
(`docs/plans/2026-09-23-performance-functionality.md`, letzter Abschnitt)
ergaben vier Themen:

1. ffprobe beim Speichern (A1)
2. Stopzeit von gpu-screen-recorder, ca. 0,21 s (A2, A3)
3. Karten-Thumbnail vom CLI statt Decode im Daemon, ca. 31 ms (B1)
4. Screenshot-Capture-Pfad (B2, B3, B4)

Recherche: nur Lesen und winzige Messungen (ffprobe auf vorhandene Dateien,
7 Läufe `boltsnap full --no-copy -o /tmp/...`, davon 2 mit `WAYLAND_DEBUG=1`).
Keine Aufnahme, kein gsr-Start, kein Daemon-Neustart.

### Wohin die Zeit heute geht

Speichern einer Aufnahme (ein Segment, Smooth-Cursor), live 0,33-0,40 s:

| Schritt | Zeit | Quelle |
|---|---|---|
| gsr-Stop nach SIGINT | ca. 210 ms | synthetischer NVENC-Test vom 24.09. |
| Warte-Poll `try_wait` alle 10 ms | 0-10 ms | `src/record/session.rs:715-729` |
| Tracker-Join nach gsr-Exit | 0-50 ms pro Tracker | `src/platform/linux/cursor_track.rs:36-42`, `:245-247` |
| ffprobe Segment (Dauer) | ca. 43 ms | `src/record/cursor_sidecar.rs:44-47` |
| ffmpeg-Concat-Remux mkv -> mp4 | ca. 50 ms | `src/record/finalize.rs:221-236` |
| ffprobe fertiger Clip (Breite) | ca. 43 ms | `src/record/finalize.rs:169` |
| `cursor.json` schreiben | wenige ms | |

Screenshot `full` (2x 1920x1080), vom Hotkey bis zur Karte ca. 95-100 ms:

| Schritt | Zeit | Quelle |
|---|---|---|
| Prozessstart bis `process_entry` | 0,6-0,7 ms warm | Messung |
| Capture (`capture_wayland_total`) | 40-45 ms, davon ca. 26-28 ms Warten auf Hyprland | Messung, Protokoll-Log |
| RGB-Drop (`rgb_and_trim`) | 5,7-6,1 ms | Messung |
| PNG-Encode | ca. 13 ms | Review 23.09. |
| Daemon: PNG voll dekodieren nur fürs Thumbnail | ca. 31 ms (Annahme: Review-Wert, nicht nachgemessen) | `src/platform/linux/shelf/mod.rs:682-704` |

### Nicht geplant

- **mkv-Segmente direkt als mp4:** Das spart den Remux von ca. 50 ms, aber
  Matroska ist bewusst gewählt. MP4 + AAC verlor beim Start ein Audiopaket, und
  ein gekillter Recorder hinterlässt nur mit Matroska ein brauchbares Segment
  (`docs/plans/2026-09-23-performance-functionality.md:125`, `:275`).
- **Compositor-Wartezeit beim Capture:** Hyprland liefert die zwei Outputs
  nacheinander, etwa 13 ms auseinander (`presentation_time` im Protokoll-Log).
  Clientseitig ist da nichts zu holen.
- **Poll-Intervall 10 ms verkleinern:** höchstens 5 ms im Schnitt.
- **Weniger Barriers in `libway::initialize`:** unter 1 ms, und
  `vendor/libway/src/capture.rs:240-242` begründet sie.
- **Buffer-Wiederverwendung im CLI:** jeder Screenshot ist ein neuer Prozess.

## 2. Wellen

| Welle | IDs | Warum diese Reihenfolge |
|---|---|---|
| 1 | A1, A2, B2 | klein, nur Boltsnap, keine Formatänderung |
| 2 | B3 | libway-intern, eigenes Repo, danach Sync |
| 3 | B1 | Protokolländerung, braucht Freigabe (Entscheidung 1) |
| 4 | A3, B4 | nur nach einer Messung und neuer Entscheidung |

Abhängigkeiten: B4 ersetzt B2 und Teile von B3, wird also erst nach Welle 1+2
neu bewertet. A3 hängt von einer gsr-Messung ab (Entscheidung 4). Sonst sind
die Items unabhängig.

Erwarteter Gewinn nach Welle 1-3: Speichern ca. 70 ms schneller (ca. 20 %),
Screenshot-Karte ca. 35 ms schneller (ca. 35 %).

## 3. Items

### A1: Ein ffprobe weniger pro Save

**Phase 1.**
Ist: Mit Smooth-Cursor probt `load` jedes Segment für die Dauer
(`src/record/cursor_sidecar.rs:44-47`, nur wenn Trackdateien existieren,
`:40-43`), danach probt `write_cursor_track` den fertigen Clip noch einmal nur
für die Breite (`src/record/finalize.rs:169`). Ohne Smooth-Cursor läuft kein
ffprobe.

| Fall | ffprobe heute | ffprobe danach |
|---|---|---|
| ein Segment, Output oder Bereich | 2 | 1 |
| Pause/Resume, N Segmente | N+1 | N |
| Combined als ein Stream | N+1 | N |
| Separate, beide Outputs | 2N+2 | 2N |
| Combined zusammengesetzt (gemischte Skalierung) | 2N+1 | 2N+1 |

Die Dauer bleibt nötig: Sie setzt den Offset für Folgesegmente
(`src/record/cursor_sidecar.rs:64`) und begrenzt die Samples am Segmentende
(`src/record/cursor.rs:81`, `:107-109`). Ohne sie stünden die ca. 0,25 s Samples
aus dem Stop hinter dem Clip-Ende. Die Breite aber liefert `probe()` beim
Segment schon mit (`src/record/cursor_sidecar.rs:103-148`). Ohne Compose ist der
Clip ein Stream-Copy der Segmente, also gleich groß. Beim zusammengesetzten
Combined hat der Clip die Leinwandgröße, dort bleibt der Clip-Probe.

Wer es merkt: ca. 43 ms schnellere Karte nach dem Speichern, bei Separate ca.
86 ms.

**Phase 2.**
1. `src/record/cursor_sidecar.rs`: `Logical` bekommt `size: Option<(u32, u32)>`.
   `load` behält die `Probe`-Ergebnisse statt nur `.duration` (`:46`).
   `Some` nur, wenn alle Segmente gleich groß sind (Moduswechsel zwischen Pause
   und Resume).
2. `src/record/finalize.rs` `finalize_recording` (`:66-80`, `:101-105`): Größe
   mitführen, nach Compose `None`.
3. `write_cursor_track` (`:160-178`) bekommt `size`; bei `Some` wird
   `factor = w / logical_width` direkt gerechnet, sonst wie bisher
   `probe(clip)`.
4. Notiz in `docs/plans/2026-09-24-live-cursor-plugin.md` („one ffprobe per
   segment plus one for the clip size“) nachziehen.

Was brechen kann: eine falsche Breite nur, wenn Segment und Clip verschieden
groß wären, und das passiert nur bei Compose, wo weiter geprobt wird. Der
gekillte Recorder (kein Duration-Header) behält seinen Fallback
(`src/record/cursor_sidecar.rs:143-147`); die Breite steht auch dann im
Stream-Header.

Test (neu, `src/record/finalize.rs`, heute schreibt kein Test `cursor.json` über
`finalize_recording`, alle nutzen `RecordCursor::System`, `:854-868`):
- Fake-`ffprobe` neben dem Fake-`ffmpeg` (`probe` nutzt
  `ffmpeg.with_file_name("ffprobe")`, `src/record/cursor_sidecar.rs:110`). Er
  hängt pro Aufruf eine Zeile an eine Zähldatei und gibt
  `width=1920\nheight=1080\nduration=5` aus.
- Segment `a.mkv` mit `a.mkv.ts` und `a.mkv.cursor`, `cursor: Quick`.
- Prüft: `cursor.json` am Clip, `width == 1920`, genau 1 Aufruf. Vor der
  Änderung rot mit 2 Aufrufen.
- Zweiter Test: zusammengesetztes Combined probt den Clip weiterhin (schon
  vorher grün, schützt die Ausnahme).

Verifikation: `cargo fmt --check && nice -n 19 cargo test --locked -j 4`.
Manuell mit Freigabe: kurze Aufnahme mit quick, `ffprobe` auf den Clip und
`jq .width` auf `cursor.json` müssen übereinstimmen.

### A2: Cursor-Tracker schon beim SIGINT stoppen

**Phase 1.**
Ist: `stop_children` schickt SIGINT an alle Recorder
(`src/platform/linux/shelf/mod.rs:2747-2750`, `src/record/session.rs:625-637`).
Die Tracker in `ActiveRecorder.cursor` (`src/record/session.rs:48-55`) werden
aber erst gedroppt, wenn `wait_with_timeouts` nach dem gsr-Exit zurückkehrt
(`:649-712`). `Tracker::drop` setzt das Flag und joint
(`src/platform/linux/cursor_track.rs:36-42`), der Thread hängt bis zu 50 ms in
`poll_event` (`:245-247`), wenn der Cursor still steht, und das ist beim Stoppen
der Normalfall. Die Drops laufen nacheinander.

Kosten: 0-50 ms bei einem Tracker, 0-100 ms bei Combined als ein Stream und bei
Separate. Ziel: Die Tracker laufen parallel zum gsr-Stop aus, der Join danach
ist sofort fertig.

Wer es merkt: im Schnitt ca. 25 ms, bei Combined ca. 50 ms schnellere Karte.

**Phase 2.**
1. `src/platform/linux/cursor_track.rs`: `Tracker::request_stop(&self)`, setzt
   nur das Flag. `Drop` joint weiter.
2. `src/record/session.rs`: in `StopChildrenJob::interrupt` (`:625-637`) nach
   jedem SIGINT `request_stop()` für alle Tracker; dasselbe in `stop_and_reap`
   (`:563-570`).
3. `#[allow(dead_code)]` an `ActiveRecorder.cursor` (`:52-53`) entfernen.

Was brechen kann: Das Plugin sieht früher EOF und behält den letzten Zustand
(`src/platform/linux/gsr_cursor/src/lib.rs:106`); nach SIGINT zeichnet gsr
höchstens noch den laufenden Frame. `cursor.json` kann ganz am Ende ein Sample
weniger haben, harmlos. Pause und Resume bekommen frische Tracker
(`src/record/session.rs:426-459`).

Test: in `cursor_track.rs` ein Tracker mit Dummy-Thread (`while !stop`), nach
`request_stop()` ist der Thread fertig, während der Tracker noch lebt. Vorher
gibt es die API nicht, der Test kompiliert also nicht, statt rot zu sein.
Optional in `session.rs`: `interrupt()` setzt alle Flags vor `wait()`.

Verifikation: wie A1. Manuell mit Freigabe: Save-Zeit einer kurzen Aufnahme im
Daemon-Log gegen 0,33-0,40 s.

### A3: Auf die „gespeichert“-Zeile von gsr warten statt auf den Exit

**Phase 1.**
gsr schreibt nach SIGINT: Frame fertig (höchstens 8,3 ms bei 120 FPS,
`/tmp/gsr-src/src/recorder/recorder.c:858-866`, `:778-796`), NVENC-Flush,
Audio-Join (bis ca. 45 ms, `audio_capture.c:187-203`, `:357`),
Matroska-Trailer (`recorder.c:881-921`), dann `puts(filepath)` + `fflush`
(`recorder.c:920`, `src/cli/main.c:379-380`). Erst danach kommen NVENC/CUDA- und
EGL-Teardown (`recorder.c:923-958`, `main.c:629-638`) und `_exit`. Feste Sleeps
gibt es nicht; `-ipc` nutzt Boltsnap nicht.

Annahme: 60-100 ms bis zur fertigen Datei, der Rest der 0,21 s ist Teardown.
Wartet Boltsnap auf die Zeile statt auf den Exit, spart das geschätzt
50-150 ms. Das ist ungemessen.

**Phase 2** (nur nach Messung, Entscheidung 4).
- `src/record/session.rs`: stdout von gsr als Pipe statt `Stdio::null()`
  (`:322-324`). `wait_for_exit` liest nicht blockierend: Zeile mit dem Pfad
  bedeutet fertig, `gsr error: Failed to save recording` bedeutet Fehler. Das
  Reapen mit den SIGTERM/SIGKILL-Fristen läuft im Hintergrund weiter.
- Risiken: Die Datei ist bis `avio_close` offen (Annahme: `av_write_trailer`
  flusht). Nach schnellem Resume laufen kurz zwei gsr gleichzeitig. Zombie,
  falls das Hintergrund-Reapen fehlt.
- Test: Fake-Recorder gibt den Pfad aus und schläft 1 s; `wait()` muss deutlich
  unter 1 s `Ready` liefern. Vorher rot.
- Messung vorher (Freigabe nötig): ein gsr-Lauf von 2 s bei 640x360, Zeitpunkt
  der stdout-Zeile gegen den Exit.

Messung 2026-09-24 (freigegeben): gsr 6.1.2 mit Boltsnaps Argumenten
(`src/platform/linux/gsr.rs:164-222`: mkv, 120 FPS, NVENC h264, qp,
`flush_packets=1;cluster_time_limit=100`), 640x360, Systemton AAC, 2 s, dann
SIGINT, je 3 Läufe:

| Lauf | stdout-Zeile | Exit |
|---|---|---|
| 1 | 13 ms | 182 ms |
| 2 | 9 ms | 162 ms |
| 3 | 19 ms | 210 ms |

In drei weiteren Läufen war die Datei beim Erscheinen der Zeile byte-identisch
mit dem Stand nach dem Exit (Größe, SHA-256, Paketzahl, ffprobe-Dauer). Die
Datei ist also fertig, wenn die Zeile kommt; ca. 150-190 ms pro Save sind
reiner Teardown.

### B1: Karten-Thumbnail vom CLI mitschicken

**Phase 1.**
Ist: Das CLI hat das Bild als `RgbImage` (`src/main.rs:529`), kodiert das PNG
(`:552-556`) und wartet in `send_shelf_add` auf das ACK (`:650-657`). Der Daemon
dekodiert im Reader-Thread das ganze PNG (`src/platform/linux/shelf/mod.rs:690`,
`:793-810`), schreibt es (`:691`) und baut das 190x132-Thumbnail (`:692`). Erst
dann entsteht die Karte (`:1697-1719`). Das Modell braucht nur Pfad und
Thumbnail (`src/shelf/model.rs:92`). Der volle Decode (bei 3840x1080 eine
12,4-MB-Allokation) liegt komplett auf dem kritischen Pfad.

`make_rgb_card_thumbnail` ist portabel (`src/shelf/thumbnail.rs:27-33`) und
liefert auf denselben Pixeln dasselbe Ergebnis (`:35-62`), weil PNG verlustfrei
ist. Im CLI kostet es ca. 1-3 ms (Annahme).

Verworfen:
- Karte mit Platzhalter und späterem `replace_thumb` (`mod.rs:2769-2776`): Der
  Tausch fiele mitten in die 150-ms-Einblendung (`mod.rs:439`) und springt.
- Rohpixel-Transport als Default: gemessen langsamer (31,9 gegen 37,0 ms,
  `docs/screenshot-performance.md`, Abschnitt Rohbild-IPC), und Encode und Write
  wandern vor das ACK.

Ziel: Karte ca. 28 ms früher, netto (Annahme).

**Phase 2.**
Wire-Format: neues Kommando `add_thumb`, Header wie `add` plus `png_len`,
`thumb_w`, `thumb_h`; Payload = PNG, direkt dahinter RGBA-Thumbnail (100.320 B).
In den Header passt es nicht (`MAX_HEADER_BYTES` 64 KiB, `src/protocol.rs:6`),
und an `add` anhängen ginge nicht, weil ein alter Daemon die Bytes mit in die
Datei schriebe (`mod.rs:691`).

1. `src/protocol.rs:27-32`: `Request::Add` bekommt `thumb: Option<CardThumb>`
   (eigener Typ `{ w, h, rgba }`, kein `image`-Typ im Protokoll). `encode`
   (`:330-340`) schreibt bei `Some` `add_thumb`; `read` (`:435-447`) prüft
   `png_len <= payload` und `w*h*4 == Rest`, sonst `InvalidData`.
2. `src/platform/linux/ipc.rs:80-91`: `add_thumb` ohne Kopie schreiben, dafür
   `write_frame` um eine Zwei-Slice-Variante erweitern (`src/protocol.rs:295`).
3. `src/main.rs:529-571`: Thumbnail vor `png_encode` bauen, `thumb: Some(..)`;
   der gemeinsame Pfad in `:638` bekommt `None`.
4. `send_shelf_add` (`src/main.rs:650`): kommt auf `add_thumb` ein
   `UnexpectedEof` vor jeder Antwort, einmal als altes `add` wiederholen. Ein
   alter Daemon verwirft das unbekannte Kommando nur mit einer Logzeile
   (`mod.rs:677`). Vorbild: `is_legacy_recording_status_eof` (`src/main.rs:432`).
5. `mod.rs:623-633`, `prepare_add` (`:682`): Mit Thumbnail nur den PNG-Header
   prüfen (IHDR über `into_decoder`, 64-MP-Grenze), Thumb-Maße müssen exakt der
   Kartengröße entsprechen, und `same_user` (`image_transfer.rs:96`) muss
   gelten, weil der Socket ohne `XDG_RUNTIME_DIR` im Temp-Verzeichnis liegt
   (`ipc.rs:11-18`). Sonst der bisherige volle Decode.
6. Windows: `src/platform/windows/shelf.rs:479` matcht `Request::Add { source,
   png, .. }` und kompiliert unverändert; der Windows-CLI sendet nie
   `add_thumb` (`src/main.rs:521` ist Linux-only). Kein neuer Windows-Code.
7. `docs/screenshot-performance.md` ergänzen.

Was brechen kann:
- Neuer CLI, alter Daemon: EOF-Retry, im Versions-Skew-Fenster ein doppelter
  Transfer.
- Alter CLI, neuer Daemon: `add` unverändert.
- Mit Thumbnail prüft der Daemon den PNG-Body nicht mehr ganz; ein kaputtes
  IDAT vom eigenen CLI landet dann auf dem Shelf (Entscheidung 2).
- Die gespeicherte Datei bleibt byte-identisch mit dem gesendeten PNG.

Tests:
- `request_add_roundtrip` (`ipc.rs:296`) mit und ohne Thumbnail;
  `request_add_without_output_remains_compatible` (`ipc.rs:319`) liefert
  `thumb == None`.
- Neu: falsche Längen oder Maße bei `add_thumb` ergeben `InvalidData`.
- Neu: `make_rgb_card_thumbnail(rgb)` ist pixelgleich mit
  `make_image_card_thumbnail(decode(encode(rgb)))`.
- Neu: Fake-Listener schließt nach einem Frame, danach folgt ein altes `add`
  mit denselben PNG-Bytes (Vorlage `legacy_eof_falls_back_but_lost_ack_does_not`,
  `image_transfer.rs:451`).
- Bestehend, müssen grün bleiben: `invalid_png_preparation_returns_an_ingest_error`
  (`mod.rs:3865`), `grayscale_png_cannot_expand_beyond_rgba_pixel_budget`
  (`mod.rs:3819`), `add_is_prepared_before_it_reaches_the_ui_event_loop`
  (`mod.rs:3959`).

Verifikation: `cargo fmt --check && nice -n 19 cargo test --locked -j 4`; kein
Windows-Target lokal, das ist im Bericht zu nennen. Manuell mit Freigabe:
Daemon mit `BOLTSNAP_TIMINGS=1` starten (`setsid env BOLTSNAP_TIMINGS=1
~/.cargo/bin/boltsnap daemon`, kein systemd-Unit), `boltsnap full` und `area`,
`shelf_png_prepare` und `shelf_model_ack` vorher und nachher vergleichen, Karte
optisch prüfen, danach ohne Env neu starten.

### B2: Schnellerer RGB-Drop in Boltsnap

**Phase 1.**
Ist: `rgba_to_rgb` schiebt jedes Pixel einzeln mit `extend_from_slice` an
(`src/platform/linux/capture.rs:64-71`), 5,7-6,1 ms bei 3840x1080. Ziel: vorab
dimensionierter Puffer, `chunks_exact_mut(3).zip(chunks_exact(4))`. Greift bei
`full`, `area`, `window`. Gewinn ca. 3 ms (Annahme).

**Phase 2.** Nur diese Funktion umschreiben. Test: Pixelgleichheit gegen
`DynamicImage::into_rgb8` für ein kleines Bild mit gemischten Alpha-Werten; die
`strip_uniform_border`-Tests laufen mit. Manuell: 3-5 Läufe
`BOLTSNAP_TIMINGS=1 boltsnap full --no-copy -o /tmp/x.png`, `rgb_and_trim`
vorher und nachher, Datei mit `/usr/bin/rm` löschen.

### B3: Weniger Speicherdurchgänge in libway

**Phase 1.**
Nach dem Compositor bleiben in libway zwei vermeidbare Nullfüllungen:
`convert_rgba` legt den Zielpuffer mit `resize(len, 0)` an und überschreibt ihn
dann pixelweise mit einem `[u8; 4]`-Swizzle (`vendor/libway/src/buffer.rs:198-223`,
ca. 4,8 ms für den zweiten Output im Log), und der Canvas wird mit 16,6 MB
genullt, bevor die Outputs hineinkopiert werden
(`vendor/libway/src/compose.rs:167-181`, `:263-279`, ca. 5-7 ms). Ziel: direkt
schreiben, Canvas nur nullen, wo kein Output ihn abdeckt. Gewinn ca. 3-5 ms
(Annahme), keine API-Änderung.

**Phase 2.** Änderung in `~/projects/libway` (nie im Vendor-Ordner,
`vendor/README.md`):
1. `buffer.rs` `convert_rgba`: nach `try_reserve_exact` direkt schreiben,
   Swizzle über `u32::from_le_bytes`. `try_reserve` bleibt, damit OOM weiter
   `LimitExceeded` statt Abbruch ergibt.
2. `compose.rs`: Nullfüllung nur, wenn die Outputs den Canvas nicht vollständig
   abdecken.
3. Sync: `python3 tools/sync-libway.py ../libway`, dann `--check ../libway`.

Was brechen kann: Kanalreihenfolge oder Alpha bei ARGB/ABGR/XRGB/XBGR,
Stride-Padding, Canvas-Lücken bei versetzten oder skalierten Outputs.

Tests: bestehende Format- und Transformtests in libway (`compose.rs:320-357`,
`buffer.rs`) plus neu: alle 8888-Formate mit Stride-Padding ergeben dieselbe
Ausgabe wie vorher; ein Layout mit Lücke bleibt dort transparent schwarz.

Verifikation: in `~/projects/libway` `cargo fmt --check`,
`cargo clippy --all-features --all-targets -- -D warnings`,
`cargo test --locked --all-features`; in Boltsnap zusätzlich
`cargo test --locked --manifest-path vendor/libway/Cargo.toml --all-features`
und `python3 tools/sync-libway.py --check`. Manuell wie B2 plus einmal
`boltsnap area` (Selector-Darstellung).

### B4: RGB-Canvas direkt aus libway

**Phase 1.** Eine neue libway-API (z. B. `capture_desktop_rgb8`) würde in einem
Durchgang aus dem SHM ins RGB-Ziel schreiben, ohne RGBA-Canvas und ohne
Boltsnaps RGB-Drop. Gewinn gegenüber heute ca. 10-13 ms, gegenüber B2+B3 noch
ca. 5 ms (Annahme). Nur für `full` und `active-window`; der Selector braucht
RGBA (`src/platform/linux/capture.rs:326-389`).

**Phase 2.** Erst nach Welle 1+2 neu messen und entscheiden (Entscheidung 5).
Falls ja: neue Funktion in libway `compose.rs`, Boltsnap-Aufruf in
`wayland_full_image`, Tests analog B3.

## 4. Entscheidungen

Bestätigt vom User (2026-09-24): 1 (a), 2 (a), 3 (a), 4 (b) erst messen,
5 (a) nach Welle 1+2 messen, 6 nein, 7 ja.

1. **B1 Protokolländerung freigeben?**
   (a) ja, neues `add_thumb` mit Rückfall auf `add`; (b) nein, B1 entfällt.
   Empfehlung: (a), der größte Einzelgewinn beim Screenshot.
2. **B1: Mit Thumbnail nur den PNG-Header prüfen?**
   (a) ja, nur für denselben User; (b) weiter voll dekodieren (dann entfällt
   der Gewinn). Empfehlung: (a).
3. **A2 umsetzen?** (a) ja; (b) nein. Empfehlung: (a), ca. 10 Zeilen.
4. **A3:** (a) nicht machen; (b) erst eine gsr-Messung (2 s, 640x360, braucht
   deine Freigabe), dann entscheiden; (c) direkt bauen. Empfehlung: (b).
5. **B4 (neue libway-API):** (a) nach Welle 1+2 neu messen, dann entscheiden;
   (b) nicht machen. Empfehlung: (a).
6. **dmabuf in libway nur bei Bedarf binden** (1632 Modifier-Events pro Lauf,
   `vendor/libway/src/capture/events.rs:97-98`, ≤1-2 ms, ändert
   `capabilities()`): Empfehlung: nicht machen.
7. **Nach Welle 1-3 Install + Daemon-Neustart für den Live-Check?** Verliert
   RAM-Shelf-Karten. Empfehlung: ja, einmal am Ende, mit Freigabe.

## 5. Status

| Welle | ID | Item | Commit |
|---|---|---|---|
| 1 | A1 | ein ffprobe weniger pro Save | `09b7a4b`, Retry `90c30d9` |
| 1 | A2 | Tracker beim SIGINT stoppen | `95e3cdc` |
| 1 | B2 | schnellerer RGB-Drop | `3204f01` |
| 2 | B3 | weniger Speicherdurchgänge in libway | libway `d5760c3`, Sync `bd63bb5` |
| 3 | B1 | Thumbnail vom CLI | `ad708a3` |
| 4 | A3 | gsr-stdout statt Exit | |
| 4 | B4 | RGB-Canvas aus libway | |

## 6. Umsetzungsnotizen

- A1: Der neue Test ruft zum ersten Mal ein Fake-`ffprobe` in der parallelen
  Suite auf und schlug einmal mit `ETXTBSY` fehl (frisch geschriebenes Skript,
  anderer Thread forkt). `run_ffprobe` wiederholt den Start jetzt wie
  `run_ffmpeg` (`src/record/finalize.rs:598`), Commit `90c30d9`.
- B2: statt vorab dimensioniertem Puffer In-place-Kompaktierung im RGBA-Puffer
  (Mikro-Benchmark 3840x1080: 3,2 ms heute, 2,3 ms `chunks_exact`, 1,9 ms
  in-place). Live `rgb_and_trim` 5,9-6,7 ms -> 2,0-2,6 ms.
- B3: nur die genullte Allokation (`alloc_zeroed`, `buffer::zeroed`). Der
  u32-Swizzle brachte im Benchmark nichts (1,17 gegen 1,15 ms) und entfiel.
  Canvas in einem frischen Prozess 7,3 -> 5,3 ms; Ende-zu-Ende geht der Gewinn
  im Compositor-Rauschen unter (Capture 42-50 ms gegen vorher 38-55 ms bei je
  5-6 Läufen).
- A3-Messung siehe Abschnitt A3: Datei fertig 9-19 ms nach SIGINT, Exit nach
  162-210 ms.
- B1: CLI-Thumbnail live 3,1 ms (`card_thumbnail`). Rückfall live gegen den
  Daemon von `c50adfe` geprüft: `bad request: unknown cmd: Some("add_thumb")`,
  danach normales `add`, Karte kam an. Der alte Weg kostete dabei
  `shelf_png_prepare` 21,5 ms. Kein Windows-Check möglich (kein Target lokal);
  Windows matcht `Request::Add { source, png, .. }` und sendet nie `add_thumb`.
- B4 neu bewertet nach Welle 1+2: `rgb_and_trim` ist 2 ms, der Canvas in
  libway ca. 5 ms; ein RGB-Canvas direkt aus libway spart davon höchstens
  3-5 ms und braucht eine neue libway-API. Empfehlung jetzt: nicht machen.
- Installiert 2026-09-24 22:30 (`c8cf56b`, Daemon neu gestartet). Live
  `boltsnap full`: `shelf_png_prepare` im Daemon 0,54-0,59 ms statt 21,5 ms,
  `png_transfer_and_shelf_ack` 1,6-5,3 ms statt 24 ms (vorher mit Rückfall
  gegen den alten Daemon gemessen). Beim Start löschte der Daemon die drei
  alten `clip-*.mkv` (A-Welle, `30dcfcb`).
- A3 offen: Messung liegt vor, Entscheidung des Users steht aus.
- B4 nicht umgesetzt (siehe oben).
