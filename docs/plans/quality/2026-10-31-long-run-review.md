# Recording und Replay im Langlauf (Review 2026-10-31)

Status: **Befund, nichts gebaut.** Frage des Nutzers: bleiben lange
Aufnahmen (1 bis 4+ h, 120 fps, NVENC) und ein tagelang laufender
Replay-Puffer sparsam, und klappen sie bis zur gespeicherten Datei?

Fortsetzung von [2026-10-12-recording-reliability-review.md](2026-10-12-recording-reliability-review.md)
(dort A, B, C, D, R, RP; R2 bis R11 und RP1 bis RP8 weiter offen) und
[../replay/2026-10-28-replay-memory.md](../replay/2026-10-28-replay-memory.md).
Eigenes File, weil das Review vom 12.10. sonst über 1000 Zeilen wüchse.

Methode: zwei parallele Read-only-Reviews (Recording, Replay) gegen
`dac0989`, danach jeder P1/P2-Befund selbst im Code oder in `/proc`
gegengelesen. Nichts gebaut, keine Aufnahme, kein Replay-Start, kein
Daemon-Neustart. Belege: Code, `/proc` des laufenden Daemons, `df`, ein
`ffprobe` auf einen vorhandenen Clip, die gsr-6.1.6- und FFmpeg-8.1.3-Quellen
aus `/var/cache/distfiles` (nur gelesen). Ein Reviewer rief zusätzlich
`gpu-screen-recorder --version` auf (nur die Version, kein Capture).

## Kurzfazit

- **Laufende Aufnahme:** Speicher und CPU des Daemons wachsen nicht mit der
  Länge (1-Hz-Check, ein `statvfs`, ein `stat` je Segment; Cursor-Spur wird
  gestreamt). Zwei Dinge halten eine 4-h-Aufnahme trotzdem auf: der Speichern-
  Remux braucht eine zweite volle Kopie auf derselben Platte (L1), und der
  Stall-Check sieht den bekannten Hänger „Bild steht, Ton läuft“ nicht (L2).
- **Replay:** läuft hier seit 2 d 23 h ohne Neustart, Clip nach ~51 h normal
  gespeichert (Export 3,7 s, fsync 2 ms). Ring, Zeitstempel, Suspend, Pipes,
  eigene Prozesse: in Ordnung. Einziger echter Langlauf-Kostenpunkt ist gsrs
  eigener Speicher (LR1), und die Load-Zeile versteckt ihn (LR2).

## Befunde

Prio: P1 Datenverlust oder lange Aufnahme scheitert, P2 echte Kosten oder
Verschlechterung über Stunden, P3 klein.

### Recording

| ID | Prio | Auslöser und Folge | Beleg | Sicherheit | Fix und Abwägung |
| --- | --- | --- | --- | --- | --- |
| L1 | P1 | **Eine 4-h-Aufnahme lässt sich hier nicht beenden.** Segmente liegen im Cache auf `/`, `record_dir` (= `save_dir`) auch. Speichern remuxt jede gsr-MKV in eine zweite volle Kopie, darum pausiert die Aufnahme bei frei ≤ `max(2 GiB, 1,05 R + 64 MiB) + 256 MiB`. Bei 36 GB frei: R ≤ ~17,4 GB, also ~2,2 h bei 17,7 Mbit/s (gemessen an einem 1080p120-Clip) oder ~1,1 h bei ~35 Mbit/s (geschätzt für 3840 × 1080 combined). Jeder Save liest und schreibt R noch einmal (17 GB ≈ 45 bis 70 s SATA). | `paths.rs:224` (Cache), `shelf/recording/watch.rs:139-148` (`remux_on_cache` auf gleicher Platte), `:165-172` (`stop_at`), `record/finalize.rs:318-320` (nur eine einzelne `.mp4` spart den Remux), `:809-812` (`remux_space`) | verifiziert (Code), Zahlen aus `df` und einem `ffprobe`, Bitrate inhaltsabhängig | (a) gsr schreibt direkt fragmentiertes MP4; ein Ein-Segment-Save wird ein Rename, keine zweite Kopie. Abwägung: ein Absturz verliert bis zu ein Fragment (~2 s) statt 100 ms; braucht vorher eine Probe, dass ein gekilltes gsr ein abspielbares fMP4 hinterlässt, und dass die Zeitbasis auf dem Frame-Raster bleibt (Cursor-Ruckeln, siehe Memory „ms frame times“). Pause/Resume bleibt ein Concat. (b) Sofort ohne Code: `record_dir` auf `/mnt/data` legen, dann entfällt der 1,05-R-Term (~4,2 h bei 17,7 Mbit/s; dann gilt R9). (c) Restzeit anzeigen (D2). |
| L2 | P1 | **Der Stall-Check (A4) sieht den Hänger mit Ton nicht.** gsrs Audio-Threads schreiben unabhängig vom Video-Thread; hängt der Encoder (Memory „gsr Vulkan encoder hang“: Bild steht, Ton läuft), wächst die MKV mit ~16 KB/s weiter. `Watch::stalled` prüft nur Wachstum: stundenlang eingefrorenes Bild ohne Meldung. Der geplante A4-Live-Test per SIGSTOP hält alle Threads an und bestünde trotzdem. Auch eine sinkende Bildrate fällt nicht auf (cfr dupliziert Frames). | `shelf/recording/watch.rs:10-12`, `:45-58`, `:105-116`; gsr `encoder.c` ~126-197 (`av_write_frame` unter eigenem Mutex), `audio_capture.c` 262, 304, `recorder.c` 697 | verifiziert (Codepfad beider Seiten), nicht reproduziert; seit NVENC kein Hänger mehr gesehen | gsrs sekündliche Zeile `update fps: N` (aus der Video-Schleife, Standard an) als Lebenszeichen: stderr über `capture_log::follow`, keine Zeile 6 s → stalled, N < 0,9 × fps für 3 s → Meldung. Nebeneffekt: Zeitstempel und Größengrenze für gsrs Zeilen (A10). Dateigröße bleibt Rückfallebene. Abwägung: ein Lese-Thread je Segment, hängt am Text von gsrs Log. |
| L3 | P2 | **Die laufende lange Aufnahme füllt das 2-GiB-Kontingent der anderen.** Ab 2 GiB Segment (~8 min bei 35 Mbit/s) scheitern Replay-Clips und Region-Clips mit der falschen Meldung „recording paused because the temporary recording cache reached 2 GiB“, ein Watchdog-Neustart des Replay scheitert vor dem gsr-Start (Replay bleibt bis Aufnahmeende tot), Shelf-Video-Importe scheitern. Nur die Aufnahme selbst überspringt das Kontingent, sobald sie lang ist. | `replay/mod.rs:451`, `:744-747`, `shelf/clients.rs:22-27`, `record/finalize.rs:770-796`, `shelf/recording/watch.rs:118-122` | verifiziert (Pfad) | Dateien der laufenden Session nicht mitzählen, solange sie lang ist; zusammen mit R3 (fremde PIDs). Die Platte schützt weiter `check_recording_reserve`. Meldung korrigieren. |
| L4 | P3 | **Ton kann über Stunden hinter dem Bild zurückfallen (in gsr).** gsr füllt Stille ein, wenn die Soundkarte langsamer als `CLOCK_MONOTONIC` läuft, wirft aber nie Samples weg, wenn sie schneller läuft. Bei +20 ppm 0,29 s nach 4 h, bei +50 ppm 0,72 s. Gemessen hier: −46 ppm (langsam, korrigiert sich selbst). Betrifft also andere Rechner. | gsr `audio_capture.c` 224-238, `recorder.c` 609-612, 682-685 | plausibel, hier nicht wirksam | Nur dokumentieren; erst bei echter Drift upstream melden. |
| L5 | P3 | **Der Timer im Popup steht ab 1 h 40 min auf 99:59.** Genau das „läuft das noch?“. Ein Test schreibt die Grenze fest. | `shelf/recording.rs:99-102` | verifiziert | Ab 1 h `H:MM:SS`; Breite im Popup prüfen, Test anpassen. |
| L6 | P3 | **Lange Smooth-Cursor-Saves** (nur `mellow`/`quick`, der Nutzer nutzt `system`): fehlt einem Segment die Dauer, liest ein ungedrosseltes `ffprobe` jedes Paket der ganzen Datei; die Cursor-Spur wird mehrfach als `Vec` kopiert (~0,4 bis 0,6 GB Spitze bei 4 h und 240 Ereignissen/s, C5 nennt „zig MB“). | `record/cursor_sidecar.rs:29-38`, `:163-198`, `record/finalize.rs:131-136`, `record/cursor.rs:131-145` | plausibel | ffprobe gedrosselt; Dauer aus der Session-Zeit; Spur zeilenweise gleich ausdünnen. |

### Replay

| ID | Prio | Auslöser und Folge | Beleg | Sicherheit | Fix und Abwägung |
| --- | --- | --- | --- | --- | --- |
| LR1 | P2 | **gsr hält ~445 MB anonymen Speicher, den es nicht mehr nutzt.** Nach 70 h: `VmHWM` 1 004 616 kB, `VmSwap` 470 772 kB, `VmRSS` 165 300 kB (31.10. selbst gelesen). Eine anonyme VMA von 630 MB, davon 425 MB im Swap. Der Speicher-Plan maß ~35 MB bei 30 s. Zwischen drei Proben im Minutenabstand unverändert: ein früherer Ausreißer, kein stetiges Wachsen. Irgendwann hielt gsr also ~1 GB resident auf einem 15-GB-Rechner, der jetzt 8,4 GB im Swap hat, und hat dabei Seiten anderer (des Spiels) verdrängt. | `/proc/841899/{status,smaps}`. Nicht unser Code; gsr schreibt blockierend, ein langsamer Worker staut also nicht in gsr. Quelle offen (glibc-mmap-Fallback oder NVIDIA-Treiberpool). | gemessen, Ursache unbekannt | Erst messen (M1). Wiederholt es sich: Watchdog startet gsr neu, wenn privat + Swap eine Schwelle reißen, über den vorhandenen Weg (RP3, `retire`); der alte Ring bleibt bis zum Wechsel speicherbar. Glibc-Tunables erst, wenn glibc bewiesen ist. |
| LR2 | P3 | **Die Load-Zeile lässt Swap weg.** Sie zeigt gsr mit ~62 MB, gsr belegt mit Swap ~532 MB. Versteckt LR1. | `platform/linux/app/load.rs:93-100` (`Private_Clean + Private_Dirty`) | verifiziert | `Swap:` aus demselben `smaps_rollup` dazu (hier privat: SwapPss ≈ Swap). Die Zahl steigt nach langer Laufzeit, das ist die Wahrheit. |
| LR3 | P3 | **Die Log-Grenze behält den Anfang, nicht das Ende.** `replay-capture.log` 4,2 MB, erste Zeile 28.10. 22:16, letzte 29.10. 17:32: die letzten ~52 h ohne gsr-Zeilen. Jeder Neustartversuch, der gsr startet, rotiert nach `.old`; nach zwei Fehlversuchen ist das Log der gestorbenen Session weg. | `replay/capture_log.rs:13-15`, `:44-48`; Rotation `replay/mod.rs:510-516` vor der Bereit-Schleife `:570-593` | verifiziert (gemessen) | Über der Grenze rotieren statt aufhören, oder `update fps` innerhalb 10 % vom Soll nach der ersten Minute weglassen (~99 % des Volumens); rotieren erst, wenn die neue Session bereit ist. |
| LR4 | P3 | **Eine flatternde Aufnahme startet endlos neu, still und ohne Backoff.** Wird sie bereit und hängt wieder, alle ~15 s ein Neustart (CUDA-Kontext, NVENC-Probe, neuer Worker, Log-Rotation), ohne Meldung, ohne Zähler. | `replay/watchdog.rs:200-207` (neuer Name setzt `Backoff` zurück), `replay/mod.rs:524-531`, `:388-440`, `replay/watchdog.rs:282-292` | plausibel (in 70 h kein Neustart) | Sessions, die binnen ~5 min sterben, als eine Fehlerserie zählen; jeden Neustart loggen; `restarts` in den Status. |
| LR5 | P3 | **Smooth-Cursor-Tracker laufen die ganze Replay-Session.** Mit `mellow`/`quick` startet Replay je Monitor einen Tracker (C1: ~60 Wakeups/s), tagelang. Der Speicher-Plan, Teil 4, sagt „läuft nur bei Smooth-Cursor-Aufnahmen“: für Replay falsch. Hier nicht aktiv (`system`). | `replay/cursor.rs:26-70`, `replay/mod.rs:499-507` | verifiziert (Code), Kosten nicht gemessen | C1 fixen oder Replay standardmäßig mit System-Cursor; Plantext korrigieren. |
| LR6 | P3 | **Geerbte fds ohne CLOEXEC gehen an jedes Kind.** Der Daemon hält fds 203 und 216 (`anon_inode:sync_file`, Flags 0) von seinem Starter; gsr, Worker und jedes ffmpeg erben sie und halten DMA-Fences für die ganze Session. Wächst nicht. | `/proc/{841737,841899,841901}/fd`; `replay/process.rs:212-233` | verifiziert (`/proc`), Herkunft plausibel | Einmal beim Daemon-Start `close_range(3, ~0, CLOSE_RANGE_CLOEXEC)`; der Cursor-Feed löscht sein CLOEXEC schon selbst in `pre_exec` (`replay/cursor.rs:62-68`). |
| LR7 | P3 | **Ein vorübergehender Socket-Fehler beendet Worker und Ring.** Jeder `accept`-Fehler (ECONNABORTED, EMFILE) oder ein gescheitertes `set_read_timeout` lässt `live::run` mit `Err` enden. | `worker/src/live.rs:246`, `:250-252` | verifiziert (Pfad), selten | `accept`-Fehler überspringen (bei EMFILE kurz warten), `set_read_timeout`-Fehler ignorieren. |

### Korrekturen an bestehenden Einträgen

- **A4** („umgesetzt“) ist blind für den Hänger mit Ton (L2); sein SIGSTOP-
  Live-Test kann das nicht zeigen.
- **A8** („keine konkrete Panic-Stelle gefunden“): `eprintln!` panict, wenn
  stderr nicht schreibbar ist. stderr des Daemons ist `daemon.log` auf `/`;
  der Finalize-Thread ruft es in `record/finalize.rs:80, 160, 270, 349, 402,
  508, 843, 856`. Läuft `/` voll (ein anderer Prozess, die Aufnahme-Reserve
  verhindert es sonst), panict der Finalize-Thread und die Session bleibt in
  Finalizing. `replay/capture_log.rs:50-52` macht es schon richtig
  (`writeln!`, Fehler ignoriert).
- **A10:** das Aufnahme-gsr läuft ohne `-v no` und ohne `capture_log`; seine
  `update fps`-Zeile landet ohne Zeitstempel und ohne Grenze in
  `daemon.log` (~160 KB/h), rotiert nur beim Daemon-Start. L2 löst das mit.
- **Speicher-Plan, Teil 4:** Smooth-Cursor-Plugin läuft auch im Replay (LR5).

## Geprüft und in Ordnung

- Wakeups beim Aufnehmen: 250-ms-Loop und 1-Hz-Check (`shelf/mod.rs:238-246`,
  `shelf/recording/session.rs:38-47`); nichts ist O(Länge); `Watch::sizes`
  bleibt begrenzt.
- Daemon-Speicher beim Aufnehmen: ein `PathBuf` je Segment; keine
  Audio-Puffer (`record/audio.rs` ist nur `pactl`); Cursor-Spur gestreamt mit
  250-ms-Flush.
- gsr-stdout bei der Aufnahme ungelesen, aber gsr schreibt dort erst beim Stop,
  boltsnap liest bis EOF (`record/session.rs:807-823`).
- Remux: `-loglevel error`, nice 19, IO-Priorität 7; Combined bei gleicher
  Skala ist eine Stream-Kopie, kein Re-Encode.
- Zeitstempel Aufnahme: gsr-cfr-pts sind ein Frame-Zähler; Remux mit
  `-video_track_timescale 120` trifft die Frames exakt; Cursor u64 µs / f64 ms;
  `elapsed_ms` u64; MP4-Atome 64 Bit bei Bedarf.
- Stop-Frist: gsrs Trailer bei 4 h ~150 KB, weit in 1,5 s.
- Threads, Prozesse, fds je Segment: alle enden, keine Zombies.
- Platte knapp: eine Warnung 2 GiB vorher, dann Pause mit erhaltenen
  Segmenten.
- Replay-Ring: Verdrängung nach Zeit und Bytes auf GOP-Grenzen, Audio mit
  (`replay/ring.rs:87-138`); Worker-Höchststand hielt das Budget, Heap nach
  70 h 26 bis 33 MB (Trim aus dem Speicher-Plan wirkt).
- Nichts wächst im Replay: Zähler u64, `card::FROZEN` ein Platz, Auswahl
  läuft nach 120 s ab; Daemon nach 70 h 11 Threads, 27 fds, 7,7 MB RSS.
- NUT-Transport: `syncpoints=none`, kein Index, 63-Bit-pts-Wrap.
- Zeitstempel Replay: i64 µs über i128 (`replay/time.rs`), kein i32/u32/f32 im
  pts-Pfad; Export rechnet exakt in der Eingangs-Zeitbasis.
- Suspend: gsr und `Instant` nutzen `CLOCK_MONOTONIC`, kein pts-Sprung, kein
  falscher Stall.
- CPU nach 70 h: Daemon 0,03 %, Worker 0,33 %, gsr 26,8 % eines Kerns (B11).
- Pipes: beide stderr bis EOF gelesen; Capture-Pipe 1 MiB; ein langsamer
  Worker blockiert gsr statt eine Schlange wachsen zu lassen.
- Clip nach Stunden: Kosten hängen nur am Ringinhalt; Teil-Dateien bei Fehler
  entfernt.

## Messungen, die noch fehlen (alle read-only oder winzig)

- **M1 (LR1):** einen Tag lang minütlich `VmHWM`, `VmData`, `VmSwap` des
  Replay-gsr in den Scratchpad loggen, den Sprung gegen `daemon.log` und
  Desktop-Ereignisse (DPMS, Vollbild, Modeset, Monitor aus) legen.
- **M2 (L1):** echte Bitrate der Combined-120-fps-Aufnahme: `ffprobe` auf die
  nächste normale Aufnahme.
- **M3 (L2):** beim nächsten eingefrorenen Bild zweimal `stat` der `.mkv` im
  Abstand von 10 s, während `update fps` in `daemon.log` stoppt.
- **M4:** Suspend mit NVIDIA: `etimes` von gsr vor und nach einem normalen
  Suspend.

## Vorschlag zur Reihenfolge

1. **L2**: Stall über gsrs fps-Zeile (P1, stilles Versagen über Stunden),
   mit A10 und der A8-Korrektur (`writeln!` im Finalize).
2. **L3**: Kontingent ohne die laufende lange Session, mit R3 (fremde PIDs)
   und richtiger Meldung.
3. **L1**: erst die fMP4-Probe (gekilltes gsr, Zeitbasis), dann Ein-Segment-
   Save als Rename. Bis dahin Entscheidung des Nutzers: `record_dir` auf
   `/mnt/data`?
4. **LR2** sofort (klein), **M1** parallel laufen lassen, danach über einen
   Speicher-Neustart für gsr entscheiden (LR1).
5. Kleinkram: L5, LR3, LR4, LR6, LR7.
6. Nur dokumentiert: L4, L6, LR5 (betreffen den Nutzer derzeit nicht).

Jeder Punkt wird vor dem Bau hier bis auf Dateien, Schritte und
Regressionstests ausgeplant.
