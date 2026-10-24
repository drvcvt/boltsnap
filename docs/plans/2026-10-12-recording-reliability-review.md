# Recording und Replay: Verlässlichkeit, Ressourcen, Polish (Review 2026-10-12)

Status: **Welle 1, Welle 2 und B4 umgesetzt (12.10.), automatisiert grün,
live offen.** Rest unverändert Vorschlag; Stand in Abschnitt 4. Vier parallele
Read-only-Reviews (Zuverlässigkeit langer Aufnahmen, Replay-Ressourcen,
Daemon-Footprint, UX), danach Gegenprüfung der schweren Befunde im Code.
Nichts gebaut, nichts aufgenommen, kein Replay gestartet. „Reproduziert“
heißt deshalb überall: **nein, nur im Code nachvollzogen**, außer wo ein
Live-Wert aus `/proc` oder einem Log genannt ist.

Leitfrage des Maintainers: Wer lange aufnimmt, soll nie denken „läuft das noch,
ist die Datei ok, frisst das gerade alles“. Replay soll im Hintergrund kaum
Leistung kosten und beim Speichern sicher funktionieren.

Uncommittete Arbeit beim Review: K5 aus
[comfort-and-config](2026-09-25-comfort-and-config.md) (Notify bei
fehlgeschlagenem Aufnahmestart). D6 unten betrifft genau diese Änderung.

## 0. Kurzfazit

- Der Ruhe-Footprint ist schlank: Daemon live 4,1 MB RSS, etwa 0,36 Wakeups/s
  im Leerlauf, keine GPU-Kontexte, kein Polling ohne Grund. „Bläht auf“ trifft
  auf den Rust-Code nicht zu.
- Die echten Risiken sind **stiller Datenverlust** (A1 bis A4) und **fehlendes
  Feedback** (D1 bis D5). Genau das erzeugt das Gefühl „mist, läuft das noch“.
- Replay kostet laut Messung vom 23.09. gsr etwa 10 % eines Kerns, den Worker
  0,4 %. Der Hebel liegt bei gsr-Parametern (B1, B2, B11) und Prioritäten (B6),
  nicht im Worker. Den Worker durch gsrs natives `-r` zu ersetzen lohnt nicht:
  Byte-Budget, Standbild, Crop aus RAM und Stall-Erkennung gingen verloren.

## 1. Befunde

Format je Befund: Priorität, Auslöser und Folge, Beleg, Fix mit Tradeoffs,
Dateien. Verifiziert = kompletter Pfad gelesen; plausibel = Codepfad belegt,
Laufzeitwirkung nicht.

### A. Aufnahme: Datenverlust und stilles Versagen

#### A1 (P1): Start-Cleanup löscht eine Aufnahme, die einen Daemon-Tod überlebt hat

- Auslöser: Daemon stirbt mitten in einer Aufnahme oder Finalisierung (Panic,
  OOM, SIGTERM beim Logout, Neuinstallation, A2) und startet neu.
- Folge: gsr bekommt per `PR_SET_PDEATHSIG` SIGTERM und hinterlässt ein
  abspielbares Matroska-Segment (100-ms-Cluster). Der nächste Start löscht es
  ohne Hinweis.
- Beleg: `src/platform/linux/paths.rs:239`
  `name.starts_with("boltsnap-") || is_clip(name) || remux_partial`, aufgerufen
  aus `run_daemon` (`shelf/mod.rs:1167`) bei jedem Start. Der Shutdown nach der
  Eventloop (`shelf/mod.rs:1353-1365`) stoppt nur die Kinder, finalisiert nicht.
  Kein SIGTERM-Handler. Verifiziert.
- Fix: Beim Start nicht-leere `boltsnap-segment-*.mkv` samt `.cursor*` nicht
  löschen, sondern als wiederhergestellt nach `record_dir` remuxen (gleiche
  Concat-Logik wie `finalize_group`, nach Segment-ID sortiert) und per Notify
  melden. Die PID im Namen gehört einem toten Daemon, sonst hätte G1s `flock`
  den Start verhindert; ein PID-Check ist deshalb nicht nötig.
  Tradeoff: Waisen belegen Platz, bis der Nutzer sie löscht; bei vollem
  Datenträger gilt A3. Zusätzlich SIGTERM/SIGINT über `signalfd` in die
  Eventloop, damit ein normales Beenden die Aufnahme regulär finalisiert.
- Dateien: `paths.rs`, `shelf/mod.rs` (`run_daemon`), `record/finalize.rs`.
- Entscheidung offen (E1 unten): Ziel der Wiederherstellung.

#### A2 (P1): Verschwindet der Monitor des Shelf-Layers, endet der Daemon samt Aufnahme

- Auslöser: Hotplug/KVM-Umschalten des Outputs, auf dem der Shelf-Layer liegt.
- Folge: Compositor schließt den Layer, der Daemon beendet sich, die Aufnahme
  ist mit A1 weg. Replay stirbt mit.
- Beleg: `shelf/mod.rs:3575-3582`, `closed` für den Shelf-Layer setzt
  `self.exit = true`. Codepfad verifiziert; dass Hyprland beim Abziehen `closed`
  schickt, folgt aus dem Layer-Shell-Protokoll, live nicht geprüft.
- Fix: Bei `closed` des Shelf-Layers `self.layer = None` setzen und beim
  nächsten Bedarf neu auf einem vorhandenen Output anlegen; nie beenden.
  Tradeoff: Shelf-Zustand muss den Neuaufbau überstehen (Karten bleiben im
  Modell, nur die Surface wird neu).
- Dateien: `shelf/mod.rs`.

#### A3 (P1): Der Reserve-Stopp kann eine Aufnahme unspeicherbar machen

- Auslöser: Freier Platz fällt auf 2 GiB, Aufnahme pausiert. Speichern remuxt
  Matroska nach MP4 im selben Cache und braucht `Größe + max(64 MiB, 5 %)`.
- Folge: Jede so gestoppte Aufnahme über etwa 1,9 GiB scheitert mit
  „insufficient disk space: need more than N bytes“. Auf dieser Platte
  realistisch (K5: 288 MB frei). `record_dir` auf einer anderen Platte hilft
  nicht, weil der Remux immer im Cache läuft.
- Beleg: `finalize.rs:249` `ensure_free_space(&tools.segment_dir,
  source_size(segments)?)`, `finalize.rs:735-745`, Reserve
  `RECORDING_DISK_RESERVE_BYTES` in `finalize.rs:50`, Prüfung in
  `tick_recording` (`shelf/mod.rs:2700-2721`). Verifiziert.
- Fix, zwei Teile:
  1. Zielgerichteter Remux: Geht die Aufnahme auf Platte (lang oder Disk-Save),
     direkt als `.part` in `record_dir` remuxen und per `renameat2` publizieren.
     Dann braucht nur das Zielsystem Platz, und auf einer anderen Platte
     reicht der Cache.
  2. Dynamische Reserve für den Rest: pausieren bei
     `frei < max(2 GiB, 1,05 × Segmentbytes + 64 MiB)` und vorher bei etwa
     doppelter Schwelle einmal warnen („noch N GB, etwa M Minuten“).
  Tradeoff: Teil 2 begrenzt eine Shelf-Aufnahme auf etwa die Hälfte des freien
  Platzes; das ist ehrlicher als eine Aufnahme, die sich nicht speichern lässt.
- Dateien: `finalize.rs`, `shelf/mod.rs`.

#### A4 (P1): Ein hängender gsr wird nie erkannt

- Auslöser: Encoder oder Treiber hängt, Prozess lebt weiter (bekannt:
  h264_vulkan bei 1080p240; GPU-Reset nach Suspend).
- Folge: Der Timer zählt weiter, die Datei wächst nicht. Bemerkt erst beim
  Anschauen.
- Beleg: `tick_recording` prüft nur `child.try_wait()` und Platz
  (`shelf/mod.rs:2686-2721`). Verifiziert.
- Fix: Im vorhandenen 1-s-Check die Größe jedes aktiven Segments lesen. Wächst
  sie 5 s nicht, greift der vorhandene `AfterStop::Recover`-Pfad (pausieren,
  Segmente behalten, melden). `-fm cfr` plus `flush_packets=1` lassen die Datei
  auch bei Standbild stetig wachsen, Fehlalarme sind deshalb unwahrscheinlich.
  Tradeoff: Ein extrem langsames Dateisystem kann auslösen; Schwelle 5 bis 10 s.
- Dateien: `shelf/mod.rs`, `record/session.rs` (`last_size`, `last_growth` je
  `ActiveRecorder`).

#### A5 (P2): Cursor-Tracker stirbt still, der Cursor friert im Video ein

- Gilt nur für `record_cursor = mellow|quick` (aktuell `system`).
- Auslöser: Output-Layout ändert sich während der Aufnahme (Hotplug anderer
  Monitor, Modus, Skalierung) oder Protokollfehler.
- Beleg: `vendor/libway/src/capture/cursor.rs:278` liefert `LayoutChanged`,
  `cursor_track.rs:276` beendet den Thread per `?`, nur `eprintln!`.
  `Tracker::is_finished` ist `#[cfg(test)]`; das Plugin hält den letzten
  Zustand. Verifiziert.
- Fix: `is_finished` produktiv machen und im Tick wie A4 in den
  Recover-Pfad gehen („Cursor-Tracking verloren“). Später optional:
  Reconnect im Tracker. Dasselbe für den Replay-Watchdog (B12).
- Dateien: `cursor_track.rs`, `shelf/mod.rs`, `replay/mod.rs`.

#### A6 (P2): Audioquelle wird beim Start festgenagelt

- Auslöser: Default-Sink wechselt während einer langen Aufnahme (Headset,
  HDMI-Audio).
- Folge: Rest der Aufnahme stumm. Resume nutzt dieselben Namen.
- Beleg: `audio.rs:112` `format!("{sink}.monitor")`, `gsr.rs:216`
  `device:{source}`, Resume `shelf/mod.rs:2860`. Plausibel; ob gsr bei
  `default_output` dem Wechsel folgt, ist nicht live geprüft.
- Fix: Für gsr `default_output`/`default_input` übergeben, `pactl` nur noch als
  Start-Plausibilität. wf-recorder-Mix behält feste Namen. Live prüfen.
- Dateien: `audio.rs`, `gsr.rs`.

#### A7 (P2): Das gespeicherte Ergebnis wird nicht geprüft

- Beleg: einzige Prüfung `require_nonempty` (`finalize.rs:655`). Verifiziert.
- Folge: Eine 20-Minuten-Aufnahme mit 4 Minuten Inhalt meldet Erfolg.
- Fix: `elapsed` in den `FinalizeRequest`, nach dem Remux per ffprobe Dauer und
  Audiostream prüfen (Probe existiert schon in `cursor_sidecar.rs`). Unter
  95 % der Laufzeit oder fehlendes Audio: Notify mit beiden Werten, Datei
  bleibt. Kosten etwa 50 ms.
- Dateien: `finalize.rs`, `session.rs`.

#### A8 (P2): Ein hängender oder abgestürzter Finalize blockiert die Sitzung

- Beleg: `shelf/mod.rs:3217-3237` nimmt `session.completed` per `mem::take`,
  Thread ohne `catch_unwind`; `run_ffmpeg` (`finalize.rs:618-640`) ohne Frist.
  Plausibel, keine konkrete Panic-Stelle gefunden.
- Folge: Phase bleibt `Finalizing`, keine neue Aufnahme, nur ein Neustart hilft
  (und der löscht mit A1).
- Fix: Segmentliste vorher klonen, `catch_unwind` → `Finalized(Err)` mit den
  Segmenten, ffmpeg mit großzügiger größenabhängiger Frist killen.
- Dateien: `shelf/mod.rs`, `finalize.rs`.

#### A9 (P3): CLI-Fallback startet den Daemon im Terminal-Prozessgruppenverbund

- Beleg: `ipc.rs:96-103` `Command::new(exe).arg("daemon")…spawn()` ohne eigene
  Sitzung. Schließt man das Terminal, kann SIGHUP Daemon und Aufnahme töten.
  Plausibel.
- Fix: `pre_exec` mit `setsid()`. Eine Zeile.

#### A10 (P3): Logs unbegrenzt, Replay-Neustart überschreibt das Crash-Log

- `daemon.log` rotiert nur beim Start, gsr-stderr ist geerbt (`session.rs:342`).
  `capture_log()` (`replay/mod.rs:315-319`) leert das Replay-Log bei jedem
  Start, auch beim Watchdog-Neustart 15 s nach dem Tod, also genau dann, wenn
  man die Ursache lesen will. Live: das Log vom 07.10. endet mit
  `non monotonically increasing dts`.
- Fix: Bei Start nach `.old` rotieren statt leeren; Größenkappe etwa 1 MB.

### B. Replay: Ressourcen und Speichersicherheit

Kostenmodell: gsr greift per KMS ab, wandelt und encodet auf der GPU (NVENC),
AAC auf der CPU, und schreibt NUT in eine Pipe. Der Worker demuxt etwa 107
Pakete/s bei 60 FPS, legt je Paket einen `Arc<Packet>` in einen GOP-Ring und
verdrängt ganze GOPs. Snapshots teilen die GOPs per `Arc` (0,14 ms für 60 s),
Remux kopiert keine Nutzdaten. Der Daemon fragt alle 5 s den Status ab.

#### B1 (P1): Replay ohne Bitratenmodus, RAM und Länge folgen der Bewegung im Spiel

- Beleg: `replay/mod.rs:465-506` setzt `-q very_high` ohne `-bm`. Ohne `-bm`
  wählt gsr Konstantqualität, also unbegrenzte Bitrate. gsrs Doku empfiehlt CBR
  ausdrücklich für den Replay-Puffer. Normale Aufnahme setzt `-bm qp` bewusst.
  Fehlendes Flag verifiziert, reale Mbit/s im Spiel nicht gemessen.
- Folge: 60 s brauchen 7,5 MB je Mbit/s. Mit B2 heißt das bei hoher Bewegung
  still kürzere Clips.
- Fix: `-bm cbr -q <kbps>` mit einer Bitrate, die zum Budget passt.
  Tradeoff: Bei viel Bewegung feste statt adaptiver Qualität; Standbild-Text in
  der Auswahl kann weicher werden. Neuer Konfigwert → Entscheidung E2.
- Dateien: `replay/mod.rs`, `src/replay/settings.rs`, `src/config.rs`.

#### B2 (P1): Standardbudget hält 60 s nur bis etwa 24 Mbit/s, und niemand sagt es

- Beleg: `worker/src/live.rs:102` `let budget = memory / 3;` bei
  `memory_mib = 512` (`src/replay/settings.rs:15`) = 170,7 MiB Live-Ring. Bei
  50 Mbit/s bleiben etwa 28 s, bei 100 Mbit/s etwa 14 s. Status meldet
  `duration_us`, aber nicht, dass Bytes statt Zeit begrenzen. Verifiziert
  (Rechnung), reale Bitrate offen.
- Fix: Mit B1 eine Bitrate wählen, die `seconds` sicher in das Live-Drittel
  passt (oder Standardspeicher anheben), und `bytes`, `budget` und
  `limited_by_bytes` in `replay status` ausgeben, damit Tray/Notify „nur 28 s“
  sagen können. Optional später: Live-Ring dynamisch bis 2/3, weil ein
  Remux-Save die Pakete nur teilt.
- Dateien: `live.rs`, `src/replay/wire.rs`, `config.rs`, `docs/replay.md`.

#### B3 (P2): 64-KiB-Capture-Pipe

- Beleg: `replay/mod.rs:508` `Stdio::piped()`, kein `F_SETPIPE_SZ` im Repo.
  64 KiB sind bei 40 Mbit/s etwa 13 ms. Ein IDR-Frame liegt bei 200 bis 600 KB.
  Läuft der Worker unter Spiellast ein paar ms später, blockiert gsrs Write.
  Ob das gsrs Capture-Schleife staut, ist plausibel, nicht geprüft.
- Fix: Nach dem Spawn `fcntl(fd, F_SETPIPE_SZ, 1 MiB)`, Fehler ignorieren.
  Kostet 1 MiB Kernel-Speicher, gibt 80 bis 200 ms Luft.
- Dateien: `replay/mod.rs`.

#### B4 (P2): Fällt der Input aus, ist die gespeicherte Historie unrettbar

- Beleg: `live.rs:60-66` `snapshot()` scheitert bei gesetztem `error` oder
  3 s ohne Videopaket; Watchdog ersetzt die Sitzung nach 3 × 5 s
  (`replay/mod.rs:229,245`). Verifiziert.
- Folge: Genau nach einem Absturz oder Hänger, wenn man die letzten 60 s will,
  geht Speichern nicht mehr, und kurz danach ist der Ring weg.
- Fix: `save`/`freeze` auf dem vorhandenen Ring zulassen, wenn der Input endete
  (Clip endet dann am Ausfall, Notify sagt das). Watchdog behält den alten
  Worker bis zum nächsten erfolgreichen Start bzw. einige Minuten.
- Dateien: `live.rs`, `replay/mod.rs`.

#### B5 (P2): Fehlgeschlagener Export lässt seine Teil-Datei liegen

- Beleg: `worker/src/export.rs:116` und `worker/src/crop.rs:231-256` entfernen
  die Partial bei Fehler nicht; nur der Daemon-Start räumt (G2). Bei ENOSPC
  bleibt die volle Platte voll. Verifiziert.
- Fix: Partial auf jedem Fehlerpfad entfernen, Name nur ins Log.

#### B6 (P2): Hintergrundarbeit konkurriert ungebremst mit dem Spiel

- Beleg: `run_ffmpeg` (`finalize.rs:618-640`) ist ein nacktes `Command` ohne
  nice, auch für den Combined-Re-Encode (`build_combined_args`, aktuelle
  Konfig `record_both_mode = "combined"`). Ebenso Thumbnail-ffmpeg
  (`shelf/thumbnail.rs:44`), Replay-Standbild-ffmpeg (`process::spawn(…, None)`)
  und der Remux-Thread im Worker samt `sync_all`. Nur der Crop-Encode läuft mit
  nice 19. ionice gibt es nirgends. Verifiziert, Wirkung nicht gemessen.
- Fix: Ein gemeinsamer `pre_exec`-Helfer in `replay/process.rs` (nice 19 plus
  `ioprio_set(IOPRIO_CLASS_IDLE)`), genutzt von `run_ffmpeg`, Thumbnail und
  Standbild; im Worker-Exportthread dasselbe per `setpriority`/`ioprio_set` auf
  die Thread-ID. gsr selbst bleibt unverändert (Echtzeitpfad).
  Tradeoff: Finalize und Karte kommen unter Last später; Idle-IO kann unter
  Dauerlast verhungern, deshalb für den Finalize eher `best-effort 7` statt
  `idle`.
- Dateien: `replay/process.rs`, `finalize.rs`, `shelf/thumbnail.rs`,
  `worker/src/export.rs`, `worker/src/crop.rs`.

#### B7 (P2): Replay wählt den gsr-Codec über das System-ffmpeg

- Beleg: `replay/mod.rs:421-466` mappt eine ffmpeg-CLI-Probe auf gsrs `-k`;
  normale Aufnahme nutzt `gsr::info()`/`gsr::choose` (`gsr.rs`). Fehlt NVENC im
  System-ffmpeg (Neubau), fällt Replay still auf h264_vulkan, den bekannten
  Hänger bei hoher FPS. Verifiziert.
- Fix: Capture-Codec über `gsr::choose`, ffmpeg-Probe nur für den Crop-Encoder.
  Weichen sie ab, läuft Replay, und nur Crop meldet seine Grenze.
- Dateien: `replay/mod.rs`, `gsr.rs`.

#### B8 (P3): OOM-Killer bevorzugt nicht die Helfer

- gsr und Worker (bis etwa 510 MB) laufen mit Standard-`oom_score_adj`.
- Fix: `+500` in `/proc/self/oom_score_adj` im vorhandenen `pre_exec`
  (`process.rs:6-18`). Drei Zeilen; Anheben braucht keine Rechte.

#### B9 (P3): Ein einzelnes schiefes Audiopaket beendet die ganze Sitzung

- Beleg: `src/replay/ring.rs:65-67` und `worker/src/ring.rs:21-27` lehnen
  nicht monotone oder dauerlose Pakete ab, der Capture-Thread endet per `?`.
  Plausibel; der Log-Fund vom 07.10. lag noch in gsrs Muxer.
- Fix: Schiefe Audiopakete verwerfen und zählen, nur Videostruktur ist fatal.

#### B10 (P3, erst messen): RSS gegen gezählte Bytes

- Je Paket etwa vier Allokationen, gemischte Größen. Soak lief mit
  512-Byte-Paketen (`docs/performance.md`), nicht realistisch.
- Erst: Worker loggt minütlich `VmRSS` gegen `ring.bytes()` über einige
  Stunden. Nur bei Drift: `malloc_trim(0)` nach Export, sonst nichts.

#### B11 (P3, erst messen): Leerlaufkosten durch `-fm cfr`

- cfr encodet auf dem ruhigen Desktop jedes Duplikat. vfr würde das sparen,
  bräche aber die 3-s-Stall-Erkennung (nur Video zählt, `live.rs:54-55`).
- Erst messen: `nvidia-smi` Leistung und P-State mit und ohne Replay bei 60 FPS
  im Leerlauf. Lohnt es, vfr plus Audio-Liveness als Stall-Signal (B12).
  Replay-FPS bleibt unabhängig von `record_fps = 120` bei 60; das ist der
  größte billige Hebel und schon Standard.

#### B12 (P3): Audio- und Cursor-Liveness im Replay unbeobachtet

- `updated` zählt nur Video (`live.rs:54-55`); stirbt der Cursor-Tracker
  (A5) oder bleibt Audio aus, merkt der Watchdog nichts.
- Fix: `audio_age_ms` und `cursor_alive` in den Status, Watchdog meldet.

### C. Daemon-Footprint

Gemessen live (`/proc`, 10,4 h Laufzeit): 4,1 MB RSS, Spitze 27,6 MB,
7 Threads, 32 FDs, etwa 0,03 % CPU im Mittel. Leerlauf wartet mit
`dispatch(None)`, Popup zeichnet nur bei Sekundenwechsel, Tray koalesziert,
keine GPU-Kontexte. Das ist schlank; die Punkte hier sind Feinschliff.

#### C1 (P2): Smooth-Cursor-Tracker weckt etwa 60-mal pro Sekunde

- Nur `mellow|quick`. Beleg: `cursor_track.rs:275` `poll_event(50 ms)`,
  `vendor/libway/src/capture.rs:416` kappt jede Wartezeit auf 20 ms.
  Verifiziert, nicht gemessen.
- Fix: libway wartet den vollen Timeout, Tracker 200 ms; Stopp fällt mit gsrs
  eigener Abbauzeit zusammen.

#### C2 (P2): Region-Rahmen rechnet jedes Pixel der Fläche und hält sie als shm

- Gilt bei `record_show_frame = true` (Standard; aktuell aus).
- Beleg: `src/shelf/paint.rs:309-330` läuft über alle `w × h` Pixel mit zwei
  SDF-Auswertungen; `shm` der vollen Fläche (`shelf/mod.rs:2566`). Bei 4K etwa
  33 MB für die ganze Aufnahme, Start blockiert geschätzt 0,1 bis 0,3 s den
  Main-Thread.
- Fix billig: nur das Randband rechnen (etwa 100× weniger CPU). Voll: vier
  schmale Surfaces oder Viewporter. Erst billig, Speicher danach bewerten.

#### C3 (P3): libway klont alle Outputs bei jedem Cursor-Event

- `vendor/libway/src/capture/cursor.rs:278` ruft `output_snapshot()` pro
  Event; bei 1000-Hz-Maus tausende Allokationen/s. Fix: nur das eigene Output
  vergleichen oder bei `wl_output`-Änderung prüfen.

#### C4 (P3): Hauptloop kann am Replay-Mutex hängen

- `Running::get()` (`replay/mod.rs:50`) nimmt den Session-Mutex im Main-Thread,
  den `start` und Watchdog über einen bis zu 5 s langen Status-Call halten.
  Plausibel. Fix: `AtomicBool` spiegeln.

#### C5 (P3): Cursor-Spur wird beim Finalize komplett geladen

- `cursor_sidecar.rs:30-50` `read_to_string` plus Parse plus Merge. Mehrstündige
  Aufnahmen kosten geschätzt zig MB kurzzeitig. Fix: zeilenweise streamen.

#### Nicht im Code, aber Platte

`~/.cargo/bin` hält 17 `boltsnap.before-*`-Backups (104 MB), die gitignorten
`target`-Ordner im Checkout zusammen über 2 GB. Kein Code-Fix; Aufräumen nur
auf ausdrücklichen Wunsch.

### D. UX und Vollständigkeit

#### D1 (P1): Popup verschluckt Fehler und zeigt den Grund einer Pause nicht

- Beleg: `shelf/mod.rs:3456` und `:3835` `let _ = self.handle_recording_action`
  für Resume per Klick/Space/Esc; `paint.rs:556-562` kennt nur Titel, `last_error`
  wird nie gezeichnet. Recovery-Pausen (A4, Absturz, Platz) öffnen das Popup
  nicht. Verifiziert.
- Fix: `Err` wie im Tray-Pfad per Notify melden, `last_error` gekürzt unter den
  Kopf, nach `AfterStop::Recover` `show_recording_controls`.

#### D2 (P1): Während der Aufnahme keine Größe, kein freier Platz, kein Audiozustand

- Beleg: Snapshot (`protocol.rs:90-97`) kennt state, elapsed, scope, outputs,
  actions_enabled, error. Tray-Titel ist konstant, kein Tooltip
  (`tray.rs:277`). Das Popup existiert nur in Pause. Verifiziert.
- Fix: `bytes`, `free_bytes`, `audio` aus dem vorhandenen 1-s-Check in den
  Snapshot; Tray-Tooltip „Aufnahme 03:12 · 412 MB · 18 GB frei · System+Mic“,
  1 Hz nur während der Aufnahme. Additive Felder im öffentlichen
  `recording watch --json` → Entscheidung E3.

#### D3 (P2): Kein Erfolgsfeedback, Karten sagen nicht, was sie sind

- Beleg: `Finalized(Ok)` ruft nur `add_finalized_cards` (`shelf/mod.rs:3091-3105`);
  die automatische Umleitung langer Aufnahmen nach `record_dir` ist unsichtbar.
  Videokarten zeigen nur ▶, keine Dauer, nicht „gespeichert“ vs. „temporär“.
- Fix: Notify mit Pfad bei dauerhaftem Speichern und bei der Umleitung; Dauer
  und Gespeichert-Marke auf der Karte (überschneidet V3).

#### D4 (P2): Finalize kann wie ein Hänger aussehen

- „Saving…“ ist statisch; aus Tray/CLI gestartet gibt es nur einen
  ausgegrauten Menüpunkt. Combined-Re-Encode, Remux und Copy über Dateisysteme
  dauern. Neue Aufnahme ist bis dahin blockiert.
- Fix: Verstrichene Speicherzeit im Popup (Tick zeichnet ohnehin sekündlich),
  Notify „Speichert…“ erst nach 3 s. Fortschritt per `ffmpeg -progress` optional.

#### D5 (P2): Rohe Fehlermeldungen ohne Handlung

- Beispiele: „recorder exited unexpectedly: exit status: 1“ (gsr-stderr landet
  im Daemon-Log oder nirgends), Byte-Zahlen statt GB, beim Startversuch heißt
  die Cache-Meldung „recording paused“.
- Fix: gsr-stderr je Sitzung in eine eigene Logdatei (wie `capture_log`) und
  letzte Zeile an die Meldung hängen; Bytes als GB; häufige Fälle als Satz mit
  Handlung („N GB freigeben, dann Alt+Print → Speichern“); Kontext statt
  festem „paused“ in `check_recording_cache_limit`.
- Dateien: `session.rs`, `audio.rs`, `finalize.rs`.

#### D6 (P2): Steuerbefehle melden Fehler nur auf stderr; K5 meldet Clip-Fehler doppelt

- Beleg (uncommittete K5-Änderung): `record_flow` meldet jeden Fehler von
  `start_recording_flow`; der Clip-Zweig (`main.rs:349-354`) meldet vorher
  schon per `replay::notify_error`. Ein fehlgeschlagener Clip gibt zwei
  Toasts, einer davon fälschlich „Recording: …“. Verifiziert.
  `recording pause|resume|save-*|discard|show-controls` und `stop` per Hotkey
  melden gar nicht.
- Fix: `inspect_err(notify_error)` im Clip-Zweig entfernen, `record_flow`
  wählt das Präfix nach Herkunft; `recording_control` auf Linux ebenfalls über
  `notify`. Einheitlicher Summary-Text (heute „boltsnap“ vs. „Boltsnap replay“).

#### D7 (P2): Audio ist unsichtbar und ohne Tray-Schalter

- Kein `TrayAction` für `audio_enabled` (`tray.rs:90-105`); nur Selector oder
  Konfig. Stummgeschaltetes Mikro wird nicht erkannt.
- Fix: Tray-Häkchen „Ton aufnehmen“, Audiozustand im Popup-Kopf und Tooltip
  (D2), beim Start `pactl get-source-mute` prüfen und melden.

#### D8 (P2): Replay-Zustand ohne „startet“, „bereit“, „neu gestartet“

- Tray-Haken heißt „Sitzung existiert“ (`replay/mod.rs:46-52`), Start kann bis
  60 s dauern, Watchdog startet still neu (Historie weg) und hat keinen Sender,
  um das Tray zu aktualisieren (`replay/mod.rs:247-310`). Autostartfehler nur
  `eprintln`. Kein Backoff bei Dauerabsturz.
- Fix: `replay_state` (stopped/starting/ready/stalled) im `TraySnapshot` und
  Menütext; Watchdog bekommt einen `DaemonEvent`-Sender; Notify bei „bereit“,
  „neu gestartet, Verlauf verworfen“ und Autostartfehler; Neustart mit Backoff.

#### D9 (P2): Regions-Clip ohne Fortschritt, Clip-Karten nicht erkennbar

- Nach dem Klick ist der Selector zu, der CPU-Crop läuft unsichtbar (Socket-
  Timeout 1900 s, `replay/mod.rs:76`). Clip-Karte sieht aus wie eine Aufnahme.
- Fix: Notify „Clip wird erstellt…“ nach 2 s; Quelle „Replay“ auf der Karte.

#### D10 (P2): Ungespeicherte Aufnahmen verschwinden bei Karte 21

- `MAX_CACHED_CARDS = 20` (`shelf/mod.rs:347`), Verdrängung löscht auch
  temporäre Videos (`shelf/mod.rs:1901-1908`). Verifiziert.
- Fix: Temporäre Videos von der Kartenzahl ausnehmen und nur über die
  vorhandene 2-GiB-Quote begrenzen, oder vor dem Löschen melden.

#### D11 (P3): Wortlaut und Zustände im Tray

- Recording und Finalizing teilen denselben roten Punkt; Discard zeigt
  „Saving recording…“ (`session.rs:283`); „Move to shelf after Disk Save“
  verschiebt nichts; Einstellungen während der Aufnahme gelten erst für die
  nächste, ohne Hinweis.
- Fix: eigener Zustand Discarding, eigener Finalizing-Ton, Menütexte
  korrigieren, „(gilt ab nächster Aufnahme)“.

#### D12 (P3): Doku veraltet

- `docs/replay.md:10` beschreibt Start/Stop-Untermenü, Tray hat ein Häkchen;
  `docs/replay.md` nennt noch H.264 Vulkan als getesteten Pfad; README und
  `docs/codebase.md` (E1) zitieren „SAVE FAILED“/„CONFIRM“ in Versalien, V9
  hat das geändert; `boltsnap doctor` prüft nichts zur Aufnahme (V4 offen).

#### D13 (P3): Alt+Print pausiert stillschweigend

- Während der Aufnahme pausiert Alt+Print und öffnet die Controls mit
  exklusivem Tastaturgriff, ohne Erinnerung bei langer Pause. README nennt den
  Hotkey „state-aware toggle“.
- Fix: README präzisieren; Notify, wenn eine Aufnahme länger als etwa 5 min
  pausiert.

### Bereits solide (geprüft, kein Risiko)

- Unerwartetes gsr-Ende: `try_wait` alle 250 ms, Pause mit behaltenen
  Segmenten und Notify.
- Stopp-Eskalation SIGINT → SIGTERM → SIGKILL; gekilltes mkv bleibt abspielbar.
- Doppeltes Stop, Stop während Pausing/Finalizing/Discarding: `can_accept`
  lehnt ab, Test deckt alle Phasen.
- Fehlgeschlagenes Speichern behält Segmente, Resume gesperrt (E1).
- Atomares Publizieren per `.part` + `renameat2(NOREPLACE)`.
- Exakte Frame-Zeiten per `-video_track_timescale`.
- Cursor-Plugin-Speicher begrenzt, Feed nicht blockierend, CLOCK_MONOTONIC.
- Keine wachsenden Handles oder Threads; Watcher gekappt (G4).
- Replay-Ring: echt begrenzt inklusive Audio und GOP-Grenzen, Snapshots per
  `Arc`, Remux ohne Kopie, Export mit Quote und `NOREPLACE`.

## 2. Entscheidungen

Maintainer 12.10.: Welle 1 und die Replay-Punkte umsetzen, E1 bis E4 nach
Empfehlung entscheiden. E1, E2 und E4 gelten so; E3 gehört zu D2 (Welle 3) und
ist damit noch nicht umgesetzt.

| # | Frage | Entscheidung |
| --- | --- | --- |
| E1 | Wohin mit wiederhergestellten Aufnahmen (A1)? | Remuxen nach `record_dir` als `…-recovered.mp4`, Notify mit Pfad. Shelf ist RAM und nach einem Absturz der falsche Ort. |
| E2 | Replay-Bitrate (B1, B2): neuer Konfigwert `[replay] bitrate_kbps`? | Ja, CBR. Standard aus `memory_mib` und `seconds` abgeleitet, sodass 60 s sicher ins Live-Drittel passen; explizit gesetzt gewinnt. Kein stilles Ändern expliziter Werte. |
| E3 | Additive Felder `bytes`, `free_bytes`, `audio` in `recording watch --json` (D2)? | Ja, additiv; bestehende Felder bleiben. |
| E4 | Remux direkt nach `record_dir` (A3 Teil 1) ändert den Finalize-Ablauf. | Ja; ohne das bleibt jede große Aufnahme an die Cache-Platte gebunden. |

## 3. Umsetzungsplan

Jede Welle einzeln committen, je Befund ein Commit. AGENTS.md: neue Features
(E1 bis E3, D2, D7-Schalter) erst nach Zustimmung.

### Welle 1: kein stiller Datenverlust (A1, A2, A4, A3, D6, D1, A9)

1. D6: Doppel-Notify im Clip-Zweig entfernen (Teil der laufenden K5-Arbeit).
2. A2: `closed` des Shelf-Layers baut neu auf statt zu beenden.
3. A1: Start-Cleanup überspringt nicht-leere Segmente und stellt sie wieder
   her; SIGTERM/SIGINT per `signalfd` → regulärer Stopp mit Finalize.
4. A4: Wachstumscheck im 1-s-Tick, Recover-Pfad.
5. A3: Remux in `record_dir` bei Disk-Ziel; dynamische Reserve plus
   Vorwarnung.
6. D1: Fehler im Popup sichtbar, Popup nach Recovery-Pause öffnen.
7. A9: `setsid` im CLI-Fallback.

Abhängigkeiten: A1 vor A3 (nutzen dieselbe Concat-Funktion). A4 und D1 teilen
den Recover-Pfad; D1 danach, damit die neue Meldung gleich sichtbar ist.

Regressionstests (plattformneutral, wo möglich): `is_orphan_rec_file` gegen
Segmentnamen; Reserve-Formel als reine Funktion; Wachstumsdetektor als reine
Zustandsmaschine mit simulierten Größen; Recovery-Remux mit zwei synthetischen
mkv-Segmenten wie die vorhandenen Finalize-Tests.

Live offen danach: Monitor des Shelf abziehen während einer Aufnahme; Daemon
per SIGTERM beenden und neu starten; gsr per SIGSTOP einfrieren (A4).

### Welle 2: Replay sparsam und vorhersehbar (B1, B2, B3, B6, B7, B5, B8)

1. B7: Codecwahl über `gsr::choose`.
2. B1 + B2: CBR mit abgeleiteter Bitrate (nach E2), `bytes`/`budget`/
   `limited_by_bytes` im Status.
3. B3: Pipe auf 1 MiB.
4. B6: gemeinsamer Prioritäts-Helfer für ffmpeg-Kinder und Worker-Exportthread.
5. B5: Partials auf Fehlerpfaden entfernen.
6. B8: `oom_score_adj` im `pre_exec`.

Tests: Argumentbau für gsr (CBR, Bitrate aus Budget); Budgetrechnung als reine
Funktion; vorhandene Worker-Tests für Export-Fehlerpfade um „keine Partial
übrig“ erweitern.

Live offen danach: reale Mbit/s in einem Spiel bei CBR; RSS-Verlauf über
2 h (B10); Leerlaufleistung mit/ohne Replay (B11); Spiel-Frametimes während
eines Vollbild- und eines Crop-Exports.

### Welle 3: man weiß jederzeit, was läuft (D2, D3, D4, D5, D7, D8, A5, A7, A10)

Reihenfolge: D5 (Log je Sitzung) → A7 (Ergebnisprüfung) → D3 → D4 → D2 (nach
E3) → D7 → A5 → D8 mit B4 und B12 → A10.

### Welle 4: Feinschliff (A6, A8, B9, C1 bis C5, D9 bis D13, K4)

Nach Bedarf; A6 erst nach Live-Test, ob gsr `default_output` folgt. B10 und B11
nur, wenn die Messung aus Welle 2 Bedarf zeigt.

### Umsetzungsnotizen 12.10.

Abweichungen vom Plan, je in einer Zeile:

- A1 ohne `signalfd`: Ein SIGTERM beendet den Daemon wie bisher, gsr bekommt
  über `PR_SET_PDEATHSIG` SIGTERM, und der nächste Start rettet die Segmente.
  Ein Finalize im Signalhandler hätte beim Logout ohnehin keine Zeit.
- A2 baut den Shelf-Layer nicht sofort neu, sondern beim nächsten Karten-Add:
  Beim `closed` kann das sterbende Output noch gelistet sein, und ein sofortiger
  Neuaufbau dort würde wieder `closed` auslösen. Bestehende Karten bleiben im
  Modell und erscheinen mit der nächsten Karte wieder.
- D1 öffnet das Popup nach einer Fehlerpause **nicht**: es greift die Tastatur
  exklusiv und würde mitten im Spiel die Eingabe stehlen. Stattdessen nennt die
  Notification den Grund, und der Popup-Titel zeigt beim nächsten Öffnen
  „Recorder stopped“, „Recorder stalled“, „Cache full“, „Low disk space“ oder
  „Disk check failed“ statt „Paused“. Fehler aus Popup-Klick, Space und Esc
  gehen als Notification raus.
- A3: Combined-Aufnahmen mit getrennten Streams (nur wf-recorder) remuxen ihre
  Einzelteile weiter im Cache, nur das komponierte Ergebnis entsteht in
  `record_dir`. Die Reserve rechnet in diesem Fall den Remux auf der Cache-Platte
  mit. Stopp-Schwelle: `max(2 GiB, Größe + max(64 MiB, 5 %)) + 256 MiB`, die
  Warnung kommt einmal 2 GiB davor. Kann das Zielverzeichnis nicht angelegt
  werden, scheitert ein Disk-Save jetzt vor dem Remux und behält alle Segmente
  unverändert (Test `a_failed_combined_disk_save_can_be_saved_again` angepasst).
- A4 prüft nur gsr-Segmente (`.mkv`): wf-recorder schreibt nur bei
  Bildänderung, ein stilles Bild wäre ein Fehlalarm. Fenster 6 s.
- B2 ohne `limited_by_bytes`: Status liefert `bytes` und `budget_bytes`, daraus
  folgt es. Abgeleitete Bitrate: Live-Drittel / (Sekunden + 1) × 0,9 − 128 kbps,
  2 bis 80 Mbit/s; Standard 20 994 kbps.
- B6: Probes, Thumbnails, Finalize-ffmpeg, Crop-Encoder und Worker-Exportthread
  laufen mit nice 19 und Best-Effort-IO 7 (nicht `idle`, damit Dauerlast sie
  nicht aushungert). Normal bleiben gsr, die Selector-Vorschau (der Nutzer
  wartet darauf) und die hyprctl-Abfragen im Screenshot-Pfad.
- B5 löscht die Partial nur, wenn der Clip unfertig ist. Scheitert erst das
  Publizieren, ist die Partial der fertige Clip; sie bleibt und der Fehler
  nennt sie.
- B7: Findet das System-FFmpeg keinen Hardware-Encoder, startet Replay trotzdem
  mit Worker-Encoder `none`, und nur Regions-Clips melden die Lücke.
- B4 zusätzlich umgesetzt (Teil des Replay-Wunschs): `save` und `freeze`
  arbeiten auf dem gehaltenen Ring, auch wenn der Input tot oder 3 s still ist;
  die Antwort trägt `input_lost`, `boltsnap replay save` meldet dann, dass der
  Clip am Ausfall endet. Der Watchdog ersetzt die Sitzung weiter nach etwa
  15 s.
- Strukturregel: `shelf/mod.rs` (5041), `record/finalize.rs` (1738) und
  `shelf/paint.rs` (1395 Zeilen) lagen schon über 1000. Vor den Änderungen
  wanderten ihre Tests nach `shelf/tests.rs` und `shelf/focus_tests.rs`,
  `record/finalize/tests.rs`, und die Controls nach `shelf/paint/controls.rs`.
  Neue Logik liegt in `shelf/watch.rs` und `record/recover.rs`. Danach:
  `shelf/mod.rs` etwa 4030, `paint.rs` etwa 1200, `finalize.rs` etwa 830 Zeilen;
  die ersten beiden bleiben über dem Limit und brauchen noch einen echten Split.

## 4. Stand

| Befund | Status |
| --- | --- |
| A1 | umgesetzt (`record/recover.rs`, `paths::clean_orphan_rec_files(keep)`); Tests für Namen, Gruppierung, Fallback ohne Join; live offen |
| A2 | umgesetzt; live offen (Monitor des Shelf abziehen) |
| A3 | umgesetzt (Remux ins Ziel, `watch::Space`); Tests für Schwelle und Warnung; live offen |
| A4 | umgesetzt (`watch::Watch::stalled`); Test der Zustandsmaschine; live offen (gsr per SIGSTOP) |
| A9 | umgesetzt (`setsid` im CLI-Fallback); live offen |
| D1 | umgesetzt mit obiger Abweichung; live offen |
| D6 | umgesetzt in der K5-Änderung (`main.rs`); live offen |
| B1, B2 | umgesetzt (`-bm cbr`, `Settings::video_kbps`, `bitrate_kbps`, Status `bytes`/`budget_bytes`); Tests der Rechnung und des Parsers; live offen: reale Qualität und Mbit/s |
| B3 | umgesetzt (`F_SETPIPE_SZ` 1 MiB); live offen |
| B4 | umgesetzt; live offen (gsr während Replay beenden, dann speichern) |
| B5 | umgesetzt (Export und Crop); live offen |
| B6, B8 | umgesetzt (`process::{in_background, background_thread}`, `oom_score_adj` 500); live offen |
| B7 | umgesetzt; live offen |
| übrige | Befund dokumentiert, nicht umgesetzt |

Checks am 12.10. nach der Umsetzung: `cargo fmt --check` (Root und Worker)
grün; `cargo clippy --all-targets` ohne Warnung; `cargo test` Root 25 + 338
grün (3 ignoriert), Worker 26 grün (eine vorbestehende Warnung `output_setup`
unbenutzt im Worker). Kein `cargo check --target x86_64-pc-windows-msvc`: das
Target ist hier nicht installiert; geteilt geändert sind nur
`shelf/paint/controls.rs` (neuer Parameter, nur Linux ruft es) und
`replay/settings.rs` (reine Rechnung). Nichts installiert, kein Live-Test: der
Live-Test braucht Zustimmung und einen Daemon-Neustart.

Review-Checks davor: Lesen von Code und Doku; `/proc` des laufenden Daemons;
`gpu-screen-recorder --version`/`--help` und Manpage.

## 5. Nachreview 2026-10-23

Read-only-Reviews (Recording, Replay) gegen den Stand 9ef1cc0, mit Fokus auf
den Code von Welle 1 und 2. Nichts reproduziert. Jeder Befund am 23.10.
selbst im Code gegengelesen; „verifiziert“ heißt kompletter Pfad bestätigt,
„plausibel“ heißt Codepfad bestätigt, Laufzeitwirkung nicht bewiesen.

### R. Recording

| ID | Prio | Auslöser und Folge | Beleg | Sicherheit | Fix und Tradeoff |
| --- | --- | --- | --- | --- | --- |
| R1 | P1 | gsr-Aufnahme, Save scheitert, nachdem mindestens ein Output schon remuxt ist (Combined: Compose scheitert, z. B. Platz oder NVENC; Separate mit Ziel Shelf: Output 2 scheitert). Die Segmente sind dann gelöscht, die einzige Kopie heißt `boltsnap-final-*`/`boltsnap-combined-*` im Cache. Der nächste Daemon-Start (Update, Logout, Neustart) **löscht sie still**, weil die A1-Rettung nur `boltsnap-segment-*` behält. Genau der Verlust, den A1 verhindern sollte. Der Test `failed_combined_encode_removes_partial_output_and_keeps_inputs` nutzt `.mp4`-Eingaben ohne Remux und sieht das nicht. | `record/finalize.rs:101-104` (`remux_dir` = Cache für Shelf und Combined), `280` (`work_path(dir, "final", …)`), `303-310` (Segmente gelöscht), `115-128` (`failure(error, ready)`), `record/recover.rs:22` (nur `boltsnap-segment-`), `paths.rs:241` (`starts_with("boltsnap-")` löscht den Rest) | verifiziert | Bei gescheitertem Save jede rettbare Datei im Cache per Rename ins Segment-Muster bringen (`segment_path`, frische ID, Session-PID, Label `combined` für ein komponiertes File); A1 rettet sie dann. Rename im selben Ordner, billig. Alternative: Manifest pro Session (robuster, neues Dateiformat). |
| R2 | P2 | Die Rettung startet vor dem Rest des Daemons, und das ffmpeg des Finalize hat kein PDEATHSIG. Stirbt der Daemon während der Rettung (früher `?` nach dem Start, Absturz beim Start, Neustart des Nutzers), läuft ffmpeg weiter und legt eine fertige Kopie `boltsnap-final-*.mp4` in `record_dir`; die Segmente bleiben, der nächste Start remuxt erneut. Jeder Start eine volle Kopie, parallele Remuxe bestehen `ensure_free_space` gemeinsam. Gleiches gilt für einen unterbrochenen Disk-Save (A3): halbe oder fertige Arbeitsdatei im Nutzerordner, der Plan versprach eine `.part`. | `shelf/mod.rs:1172-1178` (Rettung vor Wayland-Connect und Socket-Bind), `record/finalize.rs:654-666`, `platform/linux/replay/process.rs:36-45` (`in_background`: nur nice und ioprio) | verifiziert (Codepfad) | (1) `save_in_background` erst nach Eventloop und Listener. (2) `PR_SET_PDEATHSIG(SIGKILL)` plus getppid-Prüfung in `run_ffmpeg_in`. (3) ffmpeg schreibt `….mp4.part` mit `-f mp4`, der Start räumt nur eigene `boltsnap-final-*.part` in `record_dir`. Ein fast fertiger Remux wird beim Tod verworfen; die Rettung wiederholt ihn. |
| R3 | P2 | Waisen zählen ins 2-GiB-Cache-Kontingent: nach einem Absturz mit über 2 GiB Segmenten verweigert jeder neue Start, eine laufende Aufnahme pausiert in 1 s mit „Cache full“; scheitert die Rettung (Ziel voll oder nicht gemountet), bleibt das so. Der Hinweis „save or dismiss shelf recordings“ hilft nicht, die Waise liegt nicht im Shelf. | `record/finalize.rs:726-741` (zählt jede Datei in `rec_dir`), `shelf/mod.rs:547-551`, `shelf/watch.rs:120`, `record/recover.rs:93` | verifiziert | `boltsnap-segment-*` fremder PIDs nicht mitzählen (die eigene Session zählt ohnehin separat). Waisen belegen weiter Platz, die Reserve begrenzt das. |
| R4 | P2 | A4-Fall selbst: hängt gsr unkillbar (D-State), blockiert `child.kill(); child.wait()` ohne Frist; die Phase bleibt `Pausing`, Save und Discard werden abgelehnt, die Stall-Notification kommt nie. Nur ein Daemon-Neustart hilft (Daten über A1 sicher). | `record/session.rs:840-841`, `shelf/mod.rs:2954-2970`, `3052` | plausibel | Nach SIGKILL etwa 3 s `try_wait`, dann aufgeben: nicht-leeres Segment gilt als behalten, Fehler „recorder could not be killed“, `Child` an einen Reaper-Thread. Ein unkillbarer Prozess kann eine NVENC-Session halten. |
| R5 | P2 | `rename_noreplace` fällt nur bei ENOSYS zurück; NFS und FUSE (sshfs) liefern EINVAL. Mit `record_dir` dort scheitert jeder Disk-Save; bei der Rettung sind die Segmente vor dem gescheiterten `publish` schon gelöscht, die Notification „kept in the cache“ stimmt dann nicht. | `platform/linux/files.rs:58-63`, `record/recover.rs:69-71`, `111` | plausibel (errno der Dateisysteme nicht getestet) | EINVAL und EOPNOTSUPP wie ENOSYS behandeln (Hardlink-Fallback, dann `create_new` plus Kopie); die Rettung nennt den tatsächlichen Ort. |
| R6 | P3 | PID-Wiederverwendung (gleiche PID nach Reboot) kann eine noch nicht gerettete Waise überschreiben (gsr `-o` kürzt) oder zwei Sessions zu einer zusammenfügen. | `record/session.rs:317`, `576-584`, `record/recover.rs:45` | plausibel, selten | Startzeit-Token im Namen (SEGMENT_ID aus `timestamp()` seeden). |
| R7 | P3 | Die Rettung prüft nicht, ob ein nachlaufender gsr das Segment noch schreibt (gsr hängt laut Kommentar manchmal beim Stop). | `record/session.rs:345`, `674-675` | plausibel | Segmente jünger als etwa 10 s überspringen und später einmal neu versuchen. |
| R8 | P3 | Kontingent-Prüfung scheitert, wenn eine Datei zwischen `read_dir` und `stat` verschwindet (Replay-Export, Rettung): falsche Pause „Cache full“. | `record/finalize.rs:731-733` | verifiziert (Pfad), kleines Fenster | NotFound überspringen. |
| R9 | P3 | `record_dir` auf einer anderen Platte: deren freier Platz wird während der Aufnahme nie geprüft, der Save scheitert spät (Segmente bleiben). A3 Teil 2 gilt dort nicht. | `shelf/watch.rs:139-147` | verifiziert | Dann auch `record_dir` per statvfs mit `remux_space(recorded)` prüfen. |
| R10 | P3 | Discard nach gescheitertem In-Place-Disk-Save lässt die remuxte Arbeitsdatei in `record_dir` liegen. | `shelf/mod.rs:3265` (`path.starts_with(&cache)`) | verifiziert | Auch eigene `boltsnap-final-<pid>-*` im `record_dir` der Session löschen. |
| R11 | P3 | Abweichungen der Rettung vom Plan: `.cursor*`-Spuren der Waisen werden gelöscht (kein `X.cursor.json`); ein abgebrochenes wf-recorder-`.mp4` ohne moov wird als „Recovered“ gemeldet, ist aber unabspielbar; `fps: None` bringt das ms-Ruckeln zurück. | `paths.rs:216-217`, `record/finalize.rs:274-276`, `record/recover.rs:68` | verifiziert | Cursor-Spuren mit behalten, ffprobe-Prüfung vor der Erfolgsmeldung, fps aus den Segmenten lesen. |

Geprüft und in Ordnung (Auszug): Segment-Namen und numerische Sortierung,
`find` überspringt Symlinks und leere Dateien, Cleanup nach dem flock (G1),
PDEATHSIG von gsr im richtigen Thread, Stop-Eskalation, `Watch::stalled`,
`stop_at` nach Planformel, `can_accept` in allen Zwischenphasen,
Retry-Schleifen terminieren, kein Panic bei seltsamen Namen oder
gsr/ffprobe-Ausgaben.

### Umsetzungsplan Nachreview (Recording)

Reihenfolge (vom Nutzer am 23.10. bestätigt): R1, dann die Shelf-Regressionen
S6/S5 und S1, dann RP2/RP3, danach das Settings-UI. R1 ist unten bis auf
Funktionsebene geplant; R2 bis R11 sind skizziert und werden vor ihrer
Umsetzung genauso ausgeplant.

#### R1 im Detail

**Verhalten danach:** Scheitert ein Save, nachdem ein Teil schon remuxt ist,
liegen alle Dateien, die der Save im Cache erzeugt hat, unter dem
Segment-Muster `boltsnap-segment-<pid>-<id>-<label>.mp4`. Solange der Daemon
läuft, ändert sich für den Nutzer nichts (Save erneut oder Discard). Stirbt
der Daemon vorher, rettet der nächste Start sie wie jede andere Waise nach
`record_dir` (A1) und meldet das.

**Fehlerpfade, die rettbare Arbeitsdateien im Cache zurücklassen** (alle in
`finalize_recording`, `record/finalize.rs`):

| Stelle | Ziel | Was in `ready` steht |
| --- | --- | --- |
| `:117` `finalize_group` eines späteren Outputs scheitert | Shelf (beide Modi) oder Disk+Combined (`remux_dir` = Cache, `:101-104`) | fertige `boltsnap-final-*.mp4` der früheren Outputs im Cache, Original-Segmente der übrigen |
| `:128` `compose_outputs` scheitert | Combined, Shelf oder Disk | `boltsnap-final-*.mp4` aller Outputs im Cache |

Nicht betroffen: Disk+Separate (`remux_dir` ist `record_dir`, die Datei liegt
schon dort; das unschöne Arbeitsnamen-Problem ist R10), `move_to_disk_with`
(Quellen liegen in `record_dir`), der Einzel-MP4-Schnellweg (`:274-276`, die
Datei hat schon das Segment-Muster).

**Änderungen:**

1. `record/session.rs`: `segment_path` (`:576`) wird `pub(super)`. Es nutzt
   denselben Zähler `SEGMENT_ID`, also kollidiert eine umbenannte Datei nie mit
   einem Segment derselben Session.
2. `record/finalize.rs`: neue Funktion
   `keep_recoverable(ready, cache) -> BTreeMap<Option<String>, Vec<PathBuf>>`.
   Für jeden Pfad, der direkt in `cache` liegt und mit `boltsnap-final-` oder
   `boltsnap-combined-` beginnt: `fs::rename` auf
   `session::segment_path(cache, output, "mp4")`. Klappt der Rename nicht,
   bleibt der alte Pfad drin und `eprintln!` nennt den Grund (nicht schlechter
   als heute). Alles andere bleibt unverändert.
3. Die beiden Stellen `:117` und `:128` geben
   `failure(error, keep_recoverable(ready, &tools.segment_dir))` zurück.
   `session.completed` (`shelf/mod.rs:3117`) bekommt damit die neuen Pfade,
   Save erneut und Discard arbeiten auf ihnen weiter.
4. `record/recover.rs`: keine Änderung nötig, `segment_name` (`:22-30`) nimmt
   `.mp4` schon an. Das Label einer Area-Aufnahme ist `area` (Default in
   `segment_path`).
5. README (Abschnitt Recording, Satz zu „If the daemon dies mid-recording“):
   „or after a save that failed“ ergänzen.

**Randfälle:**

- Save erneut nach dem Fehler: `finalize_group` nimmt für eine einzelne MP4
  den Schnellweg und gibt dieselbe Datei zurück. Ziel Disk: `move_to_disk`
  verschiebt sie unter ihren endgültigen Namen. Ziel Shelf: die Karte zeigt auf
  eine Datei mit Segment-Namen. Die Folge: stirbt der Daemon später, rettet A1
  auch diese Shelf-Karte nach `record_dir`, statt sie wie andere Shelf-Videos
  zu verwerfen. Das ist mehr Datenerhalt als heute und wird so dokumentiert.
- Discard: `finish_discard` löscht Pfade im Cache (`shelf/mod.rs:3265`), die
  umbenannten Dateien eingeschlossen.
- Combined scheitert am Compose: gerettet werden die einzelnen Outputs, nicht
  das komponierte Bild. Das Nachkomponieren bleibt Sache eines erneuten Saves
  im laufenden Daemon.
- Ein späterer Start derselben Session ist nach einem gescheiterten Save
  gesperrt (`RESUME_BLOCKED`, `session.rs:220`, `315`; ein neuer Start wird
  abgelehnt, solange die Session existiert, `shelf/mod.rs:2396`), also kommen keine neuen
  Segmente mit gleichem Label dazu, die die Rettung fälschlich anhängen würde.
- Die Cursor-Spuren der Originale gehen bei einem gescheiterten Save heute
  schon verloren (nur ein erfolgreicher Save schreibt `X.cursor.json`). Das
  ändert R1 nicht, R11 behandelt es.

**Tests** (`record/finalize/tests.rs`, mit den vorhandenen Fake-ffmpeg-Helfern
und einem neuen, der beim zweiten Aufruf scheitert, gesteuert über eine
Zählerdatei):

1. `keep_recoverable` benennt `boltsnap-final-*` und `boltsnap-combined-*` im
   Cache um, lässt Original-Segmente und Dateien außerhalb des Caches stehen,
   und `recover::find(cache)` findet die umbenannten unter dem richtigen Label.
2. Separate, Ziel Shelf, zwei Outputs mit `.mkv`-Segmenten, das zweite Remux
   scheitert: `recoverable_segments` enthält für Output 1 eine
   `boltsnap-segment-*.mp4`, für Output 2 die Original-`.mkv`; `recover::find`
   liefert zwei Gruppen.
3. Combined mit `.mkv`-Eingaben, Compose scheitert: beide remuxten Dateien
   sind im Segment-Muster und werden gefunden. Damit deckt der Test den Fall ab,
   den `failed_combined_encode_removes_partial_output_and_keeps_inputs` mit
   `.mp4`-Eingaben übersieht.

**Abnahme:** die drei Tests grün plus die bestehenden, `cargo fmt --check`,
`cargo clippy --all-targets`, `cargo test`. Ein Live-Test ist nicht geplant:
ein echter Compose-Fehler lässt sich nicht gezielt auslösen, ohne die Platte
zu füllen oder ffmpeg zu ersetzen, und beides kommt auf dieser Maschine nicht
infrage (siehe Memory „no heavy local benchmarks“). Die Fake-ffmpeg-Tests sind
der Beleg.

#### R2 bis R11 (Skizze, wird vor der Umsetzung ausgeplant)

- **R2:** Startreihenfolge (`save_in_background` erst nach Eventloop und
  Listener), PDEATHSIG in `run_ffmpeg_in`, `.part`-Ausgabe; Test: Argumente
  enthalten `-f mp4` und `.part`, Cleanup löscht nur eigene Muster.
- **R3, R8:** Kontingent ohne fremde Waisen, NotFound überspringen.
- **R4, R5:** Kill mit Frist und Reaper; EINVAL/EOPNOTSUPP-Fallback.
- R6, R7, R9, R10, R11 mit Welle 3.

Je Schritt `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`.

| ID | Stand |
| --- | --- |
| R1 | umgesetzt 23.10.: `keep_recoverable` in `record/finalize.rs`, an beiden Fehlerstellen; `session::segment_path` ist `pub(super)`; README. Abweichungen: Rename über `rename_noreplace` statt `fs::rename` (überschreibt nie); die Tests liegen in `record/finalize/tests/recoverable.rs`, weil `tests.rs` (943 Zeilen) sonst über 1000 gewachsen wäre. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 25 + 343 grün (3 ignoriert). Kein Live-Test (siehe Abnahme). |
| R2 bis R11 | dokumentiert, nicht umgesetzt |

### RP. Replay

Kein P1. Der Code von B1 bis B8 entspricht dem Plantext; Ring, Snapshot und
Zeitstempel sind solide. Jeder Befund am 23.10. selbst gegengelesen.

| ID | Prio | Auslöser und Folge | Beleg | Sicherheit | Fix und Tradeoff |
| --- | --- | --- | --- | --- | --- |
| RP1 | P2 | Region-Clip über fast den ganzen Schirm aus 60 s Spielinhalt bei `memory_mib = 512`: der Crop kodiert mit konstantem QP 18 ohne Ratenkappe, das Ausgabelimit ist das Live-Drittel (etwa 170 MiB, also etwa 24 Mbit/s). ffmpeg wird mittendrin beendet, „crop output exceeds its disk budget“, der Clip ist nach fast der ganzen Kodierzeit weg. | `worker/src/live.rs:289-295`, `worker/src/crop.rs:206-207` (`-qp 18`), `247-250`, `replay/mod.rs:733-736` | plausibel (reale QP-18-Raten nicht gemessen) | Rate aus der Quelle kappen: `maxrate ≈ video_kbps × min(1, Crop-Fläche/Bildfläche) × 1,5` mit VBR/CQ 18 (nvenc `-rc vbr -cq 18 -maxrate -bufsize`, vaapi `QVBR`). Kann Detail bei viel Bewegung weicher machen; mehr als ihre CBR-Rate hat die Quelle ohnehin nicht. |
| RP2 | P2 | Ohne `[replay] output` übernimmt Replay das Ziel `output:NAME` der Aufnahme-Prefs **ohne Fallback**: ist der Monitor abgesteckt (KVM, Hotplug), scheitern Start, Autostart und jeder Watchdog-Neustart mit „configured replay monitor is unavailable“, obwohl der Nutzer nie einen Replay-Monitor gesetzt hat. Normale Aufnahmen fallen auf den fokussierten zurück. | `platform/linux/replay/mod.rs:400-412`, Vergleich `record.rs:117-128` | verifiziert | Nur ein explizites `[replay] output` ist strikt; das geerbte Ziel fällt wie bei Aufnahmen auf den fokussierten Monitor zurück und meldet das. |
| RP3 | P3 | Der Watchdog beendet den nach B4 noch speicherbaren Ring, **bevor** er weiß, ob der Neustart klappt; scheitert der (RP2, Reserve unter 2 GiB, voller Cache), bleibt kein Puffer und keine Session. Die Umsetzungsnotiz zu B4 nennt die Abweichung, nicht diese Folge. | `platform/linux/replay/mod.rs:289-309` (`guard.take()`, `drop(dead)`, dann `start`) | verifiziert | Nur die tote Capture beenden, den alten Worker während `start()` behalten, nur bei `Ok` tauschen; bei `Err` alte Session behalten und mit Backoff erneut versuchen. Kurz zwei Live-Drittel im RAM. |
| RP4 | P3 | Explizites `bitrate_kbps` wird nicht gegen das Budget geprüft (z. B. 64 MiB mit 200000 kbps): der Start meldet bereit, die erste GOP sprengt das Budget, die Capture endet, der Watchdog startet alle 15 bis 20 s still neu, der Tray zeigt „an“. | `config.rs:181-183`, `replay/settings.rs:38-40`, `replay/ring.rs:102-106`, `worker/src/live.rs:49-52`, `platform/linux/replay/mod.rs:302-303` | verifiziert (Rechnung) | Config ablehnen, wenn `(kbps + 128) × 1000/8 × 2 > memory_mib·MiB/3`, mit dem nötigen `memory_mib` in der Meldung (passt zu E2, keine stille Änderung expliziter Werte). |
| RP5 | P3 | B4-Lücke: nach Input-Verlust kann der Selector nie clippen, obwohl der Worker das Einfrieren des gehaltenen Rings erlaubt; nur `replay save` geht. | `platform/linux/replay/mod.rs:153` (`ready != true`), `worker/src/live.rs:68-78`, `183-189` | verifiziert | Statt `ready` auf `duration_us > 0 && !busy && !selection_active` prüfen und im Selector „capture stopped; clip ends at the failure“ zeigen. |
| RP6 | P3 | Region-Clip unter Volllast: nice 19 bekommt etwa 1,5 % CPU, die Frist `3 × Dauer + 30 s` beendet ffmpeg, die Meldung „crop encoder failed“ sagt nicht, dass es ein Timeout war. | `worker/src/crop.rs:226-239`, `261-262` | plausibel | Timeout im Watcher merken und „crop export timed out under load“ melden; optional Frist nach Fortschritt statt Wanduhr. |
| RP7 | P3 | Clip wird als gescheitert gemeldet und trotzdem als unsichtbare Waise im Cache veröffentlicht: 600-s-Crop nahe seiner Frist überschreitet das Daemon-Timeout von 1900 s, oder der Shelf bestätigt `ReplayClipReady` nicht in 5 s. Keine Karte, zählt bis zum nächsten Start ins Kontingent. | `platform/linux/replay/mod.rs:76`, `778-779`, `worker/src/crop.rs:230`, `264` | plausibel, Randfall | Daemon-Timeout aus der Worker-Frist ableiten; bei Timeout oder fehlender Bestätigung Karte nachreichen oder Datei löschen statt sie liegen zu lassen. |
| RP8 | P3 | Enges Rennen: der Selector beendet sich, während ein Freeze unterwegs ist und die PNG in den Socket-Puffer passt; die Auswahl bleibt bis zu 120 s gehalten, `replay save` und Clip sind so lange blockiert, bis zu ein Drittel RAM extra. | `platform/linux/replay/mod.rs:786-794`, `select_skia/mod.rs:331-333`, `worker/src/live.rs:225-230` | plausibel, selten | Prepare-Thread vor dem Ende des Selectors joinen oder abbrechen. |

Geprüft und in Ordnung (Auszug): Ring-Eviction nach Zeit und Bytes ohne
GOP-Split, Budget auch bei weiterlaufendem Audio, A/V-Offset aus einem
Startzeitpunkt, Timescale-Raster nach `write_header` (movenc nimmt bei 60 fps
15360, nicht 1/fps wie `docs/replay.md` sagt: Doku nachziehen), Remux prüft
`pb->error`, B5-Teil-Dateien auf jedem Fehlerpfad entfernt, PDEATHSIG am
richtigen Thread, keine fd-Lecks, B6/B8 senken nur den Export-Thread, CBR-Rechnung
ohne Overflow, kein Panic bei seltsamer gsr/ffmpeg-Ausgabe.

### Umsetzungsplan Nachreview (Replay)

1. **RP2** und **RP3** zusammen (beide in `platform/linux/replay/mod.rs`,
   zusammen verlieren sie heute Puffer und Session bei einem Hotplug).
   Details unten.
2. **RP1** (Ratenkappe im Crop, Argumente als Test), **RP4** (Validierung in
   `config.rs` mit Test), **RP5** (Gate in `preview_target`).
3. RP6, RP7, RP8 mit Welle 3; `docs/replay.md` Timescale-Satz korrigieren.

#### RP2 und RP3 im Detail

**Verhalten danach:**

- Monitorwahl beim Start (manuell, Autostart, Watchdog): ein explizites
  `[replay] output` bleibt strikt; fehlt der Monitor, scheitert der Start mit
  „configured replay monitor DP-1 is unavailable“. Kommt das Ziel nur aus
  `record_default_target = "output:DP-1"` und der Monitor fehlt, nimmt Replay
  den fokussierten und meldet einmal „DP-1 is disconnected; replay uses DP-3“,
  wie es Aufnahmen schon tun (`record.rs:117-128`).
- Watchdog-Neustart: die alte Session bleibt Besitzer des Rings, bis die neue
  läuft. `replay save` und der Selector-Clip (nach RP5) bedienen sich bis dahin
  am gehaltenen Ring. Erst ein erfolgreicher Start tauscht. Scheitert er, bleibt
  die alte Session (ohne Capture) und damit der speicherbare Puffer; der
  Watchdog versucht es mit wachsendem Abstand erneut (15 s, 30 s, 60 s, dann
  alle 5 min) und meldet den Fehler einmal pro Fehlerserie statt bei jedem
  Versuch (heute kommt alle 15 s eine Notification).

**Gegenprüfung am Code (24.10.), vor der Umsetzung:**

- Der Worker bleibt nach dem Ende seines Inputs am Leben und hält den Ring
  speicherbar: `capture` setzt nur `state.error`, `snapshot` liefert den Ring
  trotzdem, `status` meldet dann `ready: false`
  (`worker/src/live.rs:33-57`, `68-79`, `161-180`). Den alten Worker zu
  behalten bringt also wirklich einen speicherbaren Puffer.
- `process::terminate` kehrt bei einem schon beendeten Kind sofort zurück und
  wartet sonst bis 2 s (`process.rs:84-101`).
- Den Fallback gibt es schon als reine Funktion: `record::resolve_record_outputs`
  (`record.rs:100-136`, getestet `record.rs:233-282`) mit der Meldung
  „configured output DP-1 is disconnected; using focused output“. RP2 nutzt
  sie statt eigener Logik (Abweichung vom Wortlaut oben).
- **Lücke im Plan:** bleibt die alte Session während `start()` im Mutex, kann
  ein `replay save` in dieser Zeit einen Export auf dem alten Worker starten
  (`live.rs:247` `"save" if !busy`). Ein Tausch, der die alte Session sofort fallen
  lässt, würde diesen Export mittendrin beenden. Deshalb geht die alte Session
  nach dem Tausch an `retire`, das sie erst fallen lässt, wenn ihr Worker
  nichts mehr hält (siehe 4.).

**Änderungen:**

1. `platform/linux/replay/monitor.rs` (neu):
   `replay_monitor(explicit: Option<&str>, inherited: &RecordDefaultTarget, monitors: &[Monitor]) -> Result<(Monitor, Option<String>), String>`.
   Explizit: strikt, sonst „configured replay monitor DP-1 is unavailable“.
   Ohne explizites Ziel: `Output(name)` geht durch `resolve_record_outputs`
   (Fallback plus Meldung), jedes andere Aufnahmeziel (`Focused`, `Both`)
   nimmt wie heute den fokussierten Monitor. Ersetzt `mod.rs:400-412`.
2. `Session` bekommt `notice: Option<String>` (die Fallback-Meldung) und
   `stop_capture(&mut self)`: beendet `capture`, dann den Cursor-Tracker
   (`cursor = None`), in derselben Reihenfolge wie `Drop`. Der Worker bleibt.
3. Meldung: der manuelle Start (auch Autostart, der über denselben
   Socket-Befehl läuft) und der Watchdog nehmen `notice` aus der neuen Session
   und melden sie über `notify_error`, einmal pro Start.
4. `platform/linux/replay/watchdog.rs` (neu, `watchdog` zieht aus `mod.rs`
   um; `mod.rs` schrumpft): statt `guard.take()` und `drop(dead)` ruft der
   Watchdog unter dem Lock `stop_capture` auf (bis 2 s Lock; in dieser Zeit
   ist die Session ohnehin nicht `ready`, `preview_target` gäbe nichts) und
   lässt sie im Mutex. Danach unter dem Lock:
   - `cancel_start` gesetzt: Stop hat die alte schon genommen, die neue fällt.
   - `Ok(new)`: `guard.replace(new)`, Lock frei, die alte an `retire`.
   - `Err`: alte bleibt, `Backoff::failed`, Meldung nur beim ersten Fehler
     einer Serie, `eprintln!` bei jedem.
   `retire(old)` läuft in einem eigenen Thread: solange der alte Worker
   `holds_buffer` meldet, jede Sekunde neu fragen, höchstens so lange wie das
   Save-Timeout (1900 s, dafür eine Konstante statt der Zahl in
   `socket_call`); dann fällt die Session und beendet ihren Worker.
5. Backoff statt `after_restart` (Abweichung: Tausch und Abbruch sind drei
   Zeilen ohne Entscheidung, testbar ist die Zeitlogik):
   `Backoff { name, failures, retry_at }` mit `due(name, now)` (setzt sich
   zurück, sobald die Session einen anderen Namen hat, also nach Erfolg,
   manuellem Start oder Stop) und `failed(now) -> bool` (true beim ersten
   Fehler). `retry_delay(n)`: 30 s, 60 s, dann 300 s. Der erste Versuch kommt
   wie heute nach drei Fehlblicken (15 s); ein Versuch braucht weiter drei
   Fehlblicke **und** `due`.
6. Stop während eines Neustarts: unverändert, `stop` nimmt die (alte) Session
   und setzt `cancel_start`. Ein laufender `retire` beendet seinen Export
   noch; das ist Absicht, Stop soll keinen begonnenen Clip zerstören.

**Randfälle (zusätzlich zu oben):**

- Manueller Start bei gehaltener alter Session: der Start-Befehl sieht die
  Capture beendet und ersetzt die Session wie heute (`guard.take()`), außer
  sie hält eine Auswahl oder einen Export. Er bleibt der Ausweg.
- Ein `save`, der den Namen der alten Session vor dem Tausch gelesen, den
  Worker aber noch nicht erreicht hat, kann `retire` verpassen und scheitert
  dann mit einem Fehler. Kein stiller Verlust; das gleiche Fenster hat der
  manuelle Start heute.
- Speicher im schlimmsten Fall: alter Live-Ring plus alter Export plus neuer
  Live-Ring, je ein Drittel, also das Gesamtbudget.

**Tests:**

1. `replay_monitor`: explizit vorhanden, explizit fehlt (Fehler mit Name),
   geerbt vorhanden, geerbt fehlt (fokussierter plus Meldung), `Focused` und
   `Both` (fokussierter, keine Meldung), kein fokussierter Monitor (Fehler).
2. `Backoff`: Abstände 30, 60, 300, 300 s; `failed` ist nur beim ersten Mal
   true; `due` vor und nach `retry_at`; ein neuer Name setzt alles zurück.

**Abnahme:** die Tests grün, `cargo fmt --check`, `cargo clippy --all-targets`,
`cargo test`. Live mit Zustimmung: Replay läuft, Recording-Ziel ist der zweite
Monitor, der wird in Hyprland abgeschaltet (Lua-Config: der genaue
`hl.monitor`-Aufruf wird vorher an der API geprüft, `hyprctl keyword` greift
dort nicht); erwartet: eine Meldung, Replay
läuft auf dem fokussierten weiter, `boltsnap replay save` liefert vorher wie
nachher einen Clip.

Live offen, mit Zustimmung: RP1 mit einem großen Region-Clip aus einem Spiel.

| ID | Stand |
| --- | --- |
| RP2, RP3 | umgesetzt 24.10. nach dem Plan oben (`replay/monitor.rs`, `replay/watchdog.rs`, `Session::stop_capture`, `notice`, `retire`, `SAVE_TIMEOUT`); `docs/replay.md` nachgezogen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 25 + 350 grün (3 ignoriert). Live 24.10. (DP-3, explizites `output`): gsr der Replay-Session per SIGKILL beendet; Status `stalled`, `replay save` auf dem alten Ring lieferte 14,75 s mit `input_lost`; nach etwa 20 s neue Session `running`, alter Worker beendet, ein gsr, ein Worker; `replay save` danach 39,5 s. Nicht live: der Fehlerpfad (Neustart scheitert, Backoff) und der geerbte Fallback, weil die Config des Nutzers ein explizites `output` hat; beide decken die Tests. |
| RP1, RP4 bis RP8 | dokumentiert, nicht umgesetzt |
