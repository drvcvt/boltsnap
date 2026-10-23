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
