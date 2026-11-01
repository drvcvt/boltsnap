# Langlauf-Fixes: Plan (2026-10-31)

Status: **Wellen 0 bis 3 umgesetzt (01.11.), Rest geplant.** Gebaut wird
Welle für Welle nach dem Go des Nutzers, ein Commit je ID mit seinem Test.

Herkunft: alle Befunde aus
[2026-10-31-long-run-review.md](2026-10-31-long-run-review.md) (L1 bis L6,
LR1 bis LR7) plus die Korrekturen A8 und A10 und die offenen Punkte R3 und R9
aus [2026-10-12-recording-reliability-review.md](2026-10-12-recording-reliability-review.md).
Geplant in drei parallelen Read-only-Durchgängen gegen `5fc2429`; jede
tragende Stelle danach selbst gegengelesen. Belege: Code, gsr-Quelle
`099a55b` (= installiertes 6.1.6, `/var/db/pkg/media-video/gpu-screen-recorder-9999`),
FFmpeg 8.1.3 (`/var/cache/distfiles`, nur gelesen), std 1.98.1.

Grenzen, die jede Welle einhält: `src/platform/linux` (17 Einträge) und
`src/platform/linux/shelf` (13) bekommen keine neue Datei;
`src/record/session.rs` (1909 Zeilen) wächst nicht; `replay/mod.rs` (972)
wird vor jeder Ergänzung erst kleiner (S1); `record/finalize/tests.rs` (945)
bekommt neue Tests nur als eigene Datei unter `finalize/tests/`.

## Zeilen-Korrekturen am Review

- LR4: `replay/watchdog.rs` hat 205 Zeilen; der Backoff-Reset ist
  `watchdog.rs:42-50` (`Backoff::due`), der stille Neustart `:123-134`.
- LR7: `accept` ist `worker/src/live.rs:156`, `set_read_timeout` `:160-162`.
- L3: ein Aufrufer fehlte, `shelf/recording/start.rs:16-21`
  (`prepare_recording_cache`, gerufen in `start.rs:167`, `:247`,
  `shelf/clients.rs:25`). Das Kontingent liegt jetzt in
  `record/finalize.rs:766-796` (R3 nennt noch 726-741).
- L5: Datei ist `src/shelf/recording.rs:99-102`, Test `:167-171`.

## Wellen

| Welle | IDs | Warum hier |
| --- | --- | --- |
| 0 | S1 | reiner Umzug aus `replay/mod.rs`, damit LR3, LR4, LR7 Platz haben |
| 1 | A8, LR2, LR7, LR6, L5 | klein, isoliert, je eine Datei oder zwei |
| 2 | L2 (mit A10), LR3 | P1 stilles Versagen; beide teilen `capture_log::follow` |
| 3 | L3 (mit R3), R9 | Kontingent und Zielplatte; beide in `watch.rs` |
| 4 | LR4 | baut auf S1 und dem `log!` aus A8 |
| 5 | L1-Probe, dann L1 | braucht das Okay für die Probe und F1 |
| 6 | M1, dann LR1 | braucht das Okay für M1 und F3; nach LR3 |
| doku | L4, L6, LR5 | betreffen den Nutzer derzeit nicht |

Abhängigkeiten:
- A8 vor L2, LR4, LR1: alle neuen Log-Zeilen gehen über `log!`.
- L2 vor LR3: L2 ändert die Signatur von `follow`, LR3 setzt den Filter
  hinein. Der fps-Zähler aus L2 zählt **jede** geparste Zeile, auch die, die
  der LR3-Filter nicht schreibt.
- L2, L3, R9 und L1 ändern alle `shelf/recording/watch.rs` (227 Zeilen):
  in dieser Reihenfolge, je ein Commit.
- LR3 vor M1: sonst hat M1 kein gsr-Log (das Log endet seit 52 h) und
  `VmHWM` steht schon auf 1 GB. M1 braucht einen frischen Replay-Start.
- L1 erst nach der Probe; R9 vorher, weil auch nach L1 ein Save auf eine
  andere Platte R kopiert.

---

## S1: Replay-Start aus `replay/mod.rs` ziehen (Vorarbeit)

**Phase 1.** Keine Verhaltensänderung. `replay/mod.rs` hat 972 Zeilen;
LR3, LR4 und LR7 müssen dort ergänzen.

**Phase 2.**
1. `CAPTURE_PIPE_BYTES`, `companion_path`, `start`, `start_capture`
   (`replay/mod.rs:346-594`, ~250 Zeilen) unverändert nach neuem
   `replay/start.rs` (replay/ dann 11 Einträge). `mod.rs` sinkt auf ~725.
2. `Session` bleibt in `mod.rs`; private Felder sind aus dem Kindmodul
   sichtbar. `watchdog.rs:113` ruft `super::start::start`; `super::hypr` und
   `gsr` werden `super::super::…`.
- Test: keine neuen; `cargo test` grün, `git diff --stat` zeigt nur Umzug.
- Risiko: keins außer Pfaden; der Compiler findet sie.

---

## A8: `eprintln!` auf Hintergrund-Threads panict nicht mehr

**Phase 1.** `eprintln!` panict, wenn stderr (`daemon.log` auf `/`) nicht
schreibbar ist, etwa bei ENOSPC. Auf diesen Threads lässt das die Session
hängen (kein `catch_unwind` in `src/`):
- Finalize (`shelf/recording/finish.rs:59-62`): Stellen
  `record/finalize.rs:80, 160, 270, 349, 402, 508, 843, 856`, Phase bleibt
  Finalizing.
- Stop (`shelf/recording/session.rs:248-251` → `record/session.rs:710`):
  Phase bleibt Stopping.
- Reaper `record/session.rs:762, 765`; Recover `record/recover.rs:113`.

Ziel: Log-Zeilen gehen über ein `log!`, das Schreibfehler verwirft (wie es
`replay/capture_log.rs:50-52` schon macht). Nicht: kein `catch_unwind`, kein
Sweep der ~45 Stellen auf dem Main-Thread (ein Panic dort beendet den Daemon
und landet in der A1-Rettung).

Entscheidung: nur die 12 Stellen in `record/`, weil nur dort ein Panic Zustand
strandet (statt alle Daemon-Stellen: mechanischer Sweep über shelf und replay,
gegen „eine Fähigkeit je Änderung“; neue Stellen aus L2, LR4, LR1 nutzen
`log!` sofort).

**Phase 2.**
1. `src/main.rs` vor `mod config;` (`:7`): `macro_rules! log` mit
   `let _ = writeln!(std::io::stderr(), …)`. `record` und `platform` hängen
   nur an `main.rs` (`src/lib.rs` hat sie nicht), der textuelle Scope reicht.
   Kein `log`-Crate in `Cargo.toml`, kein Namenskonflikt.
2. Die 12 Stellen auf `log!`. In `record/session.rs:761-766` die Arm-Klammern
   weg (spart 4 Zeilen, deckt das `SIG_IGN` aus L2).
3. Test in `main.rs` `tests` (`:784`): `log!` auf einen Writer, der immer
   `Err` liefert, darf nicht panicen. Entscheidung: der Makro-Rumpf schreibt
   über eine kleine Funktion `log_to(w: &mut impl Write, args)`, die der Test
   mit einem Fehler-Writer ruft (statt fd 2 per `dup2` auf `/dev/full`: stört
   parallele Tests). Vorher mit `eprintln!`-Semantik nicht ausdrückbar, der
   Test prüft die neue Funktion.

---

## LR2: Load-Zeile zählt Swap mit

**Phase 1.** `app/load.rs:93-100` (`private_of_rollup`) summiert
`Private_Clean + Private_Dirty`; gsr steht mit ~62 MB da, belegt aber mit
Swap ~532 MB. Ziel: `+ SwapPss`, die Bedeutung bleibt „was frei würde, wenn
es stoppt“. Fehlt `SwapPss:`, zählt es 0, die Zeile verschwindet nie. Der
statm-Fallback hat keinen Swap, bleibt so. Nicht: kein eigener Swap-Wert,
kein neuer UI-Text, `WIDEST` (`src/app/preview/load.rs:24`) bleibt.

Entscheidung: `SwapPss`, weil eine geteilte ausgelagerte Seite anteilig
zählt wie der Rest der Zeile (statt `Swap:`: zählt sie in jedem Prozess
voll).

**Phase 2.**
1. `load.rs:99`: `+ kib("SwapPss:").unwrap_or(0)`; Doku `:1-4`, `:77-80`.
2. Test: Fixture in `private_memory_is_the_clean_and_dirty_pages_no_one_else_maps`
   (`:175`) bekommt `SwapPss: 470772 kB`, Erwartung inklusive. Scheitert heute.
3. `docs/replay.md:182-185` „inklusive ausgelagerter Seiten“; Einzeiler im
   Speicher-Plan, Teil 1.
- Risiko: die Zahl springt nach langer Laufzeit. Das ist die Wahrheit.
- UI-Gates: Text und Breite unverändert (gleiche Zahlstelle, `WIDEST`
  deckt vier Stellen); ein Screenshot der Replay-Seite im Dark Theme.

---

## LR7: Socket-Fehler beenden den Worker nicht mehr

**Phase 1.** `worker/src/live.rs:156` (`accept`) und `:160-162`
(`set_read_timeout`) geben `Err` zurück, `main.rs` des Workers endet mit 1,
der Ring ist weg. Ziel:
- EMFILE, ENFILE, ENOBUFS, ENOMEM: 100 ms warten, weiter (`poll` bleibt
  bereit, ohne Pause dreht die Schleife).
- andere `accept`-Fehler (ECONNABORTED, EPROTO, EINTR): weiter.
- `set_read_timeout` scheitert: diese Verbindung fallen lassen.

Entscheidung: Verbindung fallen lassen statt Fehler ignorieren, weil ein
Stream ohne Timeout die einfädige Schleife für immer blockieren kann.

Geschwister: die Steuerschleife des Daemons dreht bei EMFILE mit 100 % CPU
(`replay/mod.rs:619-621`, `let Ok(..) else continue`). Gleiche kurze Pause
dort (+1 Zeile, nach S1). Nicht: der `poll`-Fehlerpfad (`live.rs:146-152`).

**Phase 2.**
1. `live.rs`: `fn accept_wait(&io::Error) -> Option<Duration>` plus Unit-Test
   der Einordnung.
2. Regressionstest in `worker/tests/live.py`: nach Bereitschaft
   `resource.prlimit(pid, RLIMIT_NOFILE, (niedrigster freier fd, hard))`,
   verbinden, `status` senden, nach 1 s lebt der Worker; Limit zurück,
   `status` antwortet. Scheitert heute.
3. Bauen und testen: `cargo test --locked --manifest-path
   src/platform/linux/replay/worker/Cargo.toml` und `live.py`.
- Deploy: der neue Worker neben dem Daemon-Binary (Install-Schritt des
  Nutzers, wie bisher).

---

## LR6: Geerbte fds bekommen CLOEXEC

**Phase 1.** Starter ist Hyprland `exec-once` → `sh` → `boltsnap daemon`
(`~/.config/hypr/hyprlua/autostart.lua:29`). Kein `LISTEN_FDS`, keine
Socket-Aktivierung in `src/`; der Fallback-Spawn `ipc.rs:96-117` gibt nur
stdio weiter. Einzige gewollte Weitergabe ist der Cursor-Feed: `pipe2(O_CLOEXEC)`
(`cursor_track.rs:128`), im Kind gelöscht (`replay/cursor.rs:62-68`).
`daemon.log` liegt auf fd 2.

Ziel: erste Anweisung von `shelf::run_daemon` (`shelf/mod.rs:293`) markiert
fds ab 3 als CLOEXEC. Der Daemon selbst behält sie. Nicht: nichts schließen
(unbekannte fds können dem Daemon selbst gehören).

Entscheidung: `libc::syscall(SYS_close_range, 3, u32::MAX, CLOSE_RANGE_CLOEXEC)`,
weil `libc::close_range` nur für glibc ≥ 2.34 gebunden ist (statt des
Wrappers). Lockfile libc 0.2.186 hat beide Konstanten. Fallback bei ENOSYS
oder EINVAL (Kernel < 5.11): `/proc/self/fd` durchgehen, `fcntl(F_SETFD,
FD_CLOEXEC)`.

**Phase 2.**
1. `ipc.rs` (700 Zeilen) neben `lock_daemon`: `pub fn cloexec_inherited()`
   und `fn cloexec_by_proc()`.
2. Aufruf in `run_daemon` vor `lock_daemon`.
3. Test in `ipc.rs` `tests`: `dup(2)` (ohne CLOEXEC), Helfer rufen,
   `F_GETFD & FD_CLOEXEC` prüfen; dasselbe für den Fallback. Scheitert heute
   (Helfer fehlt).
- Live nach dem nächsten Daemon-Neustart des Nutzers: `/proc/<gsr>/fd` ohne
  `sync_file`.

---

## L5: Popup-Zeit ab 1 h als `H:MM:SS`

**Phase 1.** `fmt_elapsed` (`src/shelf/recording.rs:99-102`) klemmt auf
5999 s. Ziel: unter 1 h `MM:SS`, ab 1 h `H:MM:SS`, keine Obergrenze
(„10:00:00“). Das Popup zeigt nur pausiert oder beim Speichern
(`paint/controls.rs:263-274`), das Format wechselt also nie sichtbar; keine
Motion nötig. Die Panel-Breite kommt nur aus der Button-Reihe
(`paint/controls.rs:68-86`), die Zeit steht rechtsbündig über
`steady_ink_to` (`:125-133`, `:231`): ein längerer Text wächst nach links,
das Panel nie. Nicht: kein Live-Timer während der Aufnahme (D2).

Entscheidung: die geteilte Funktion gilt auch für die eingefrorene
Windows-Shell (`windows/shelf.rs:805`), weil es portable Formatierung ist
(statt einer Windows-Kopie im eingefrorenen Backend).

**Phase 2.**
1. `fmt_elapsed` ändern; Test `:167-171` ersetzen: 3599 → „59:59“,
   3600 → „1:00:00“, 7000 → „1:56:40“, 36000 → „10:00:00“. Die alte
   „99:59“-Erwartung bricht absichtlich.
2. Neuer Test in `src/shelf/paint.rs` neben `:411`: für jeden Titel
   („Recording“, „Paused“, „Save failed“, „Saving…“, „Recorder stopped“,
   „Recorder stalled“, „Cache full“, „Disk check failed“, „Low disk space“)
   gilt Tinten-Ende + `CTL_PAD` ≤ Tinten-Anfang von „10:00:00“, mit
   `fallback_popup_font` und der echten Popup-Font, wenn ladbar.
- UI-Gates: 1 Überlauf (längster Titel gegen längste Zeit, Test oben),
  3 rechter Rand bleibt 16 (gemessen), Breite ändert sich nie
  (konstruktiv), 6 kein neuer Text, 7 ein Dark-Screenshot des pausierten
  Popups bei 1:40:00. 2, 4, 8 unverändert. `tabular-nums`: ab_glyph kennt
  kein `tnum`, `steady_ink_to` hält nur die rechte Kante; ausreichend, weil
  das Popup nicht tickt.

---

## L2 (mit A10): Stall über gsrs fps-Zeile

**Phase 1.**
- Heute: stalled nur, wenn die `.mkv` 6 s nicht wächst
  (`shelf/recording/watch.rs:12`, `:45-58`, Prüfung `:105-116`).
  wf-recorder hat gar keinen Check (`:106-107`), bleibt so. Bei Stall:
  `tick_recording` (`shelf/recording/session.rs:38-59`) → `AfterStop::Recover`
  → `events.rs:150-160`: Pause, Segmente bleiben, Notify „the recorder stopped
  writing video for 6 s“, Popup-Titel „Recorder stalled“ (`controls.rs:208`).
- gsr-Belege: Zeile `gsr info: update fps: %d, damage fps: %d` (`log.c:20`,
  `recorder.c:560`) aus `recorder_update_fps_counters` (`recorder.c:553-566`),
  gerufen in `recorder_tick_video` (`:847`) direkt vor dem Encode (`:848`,
  `avcodec_send_frame` `:698`), in der Hauptschleife (`:936-937`). Hängt der
  Encoder, steht die Zeile. Etwa jede 1 s, CLOCK_MONOTONIC, auf stderr. Nur
  `-v no` schaltet sie ab (`args_parser.c:294`); `gsr::args` setzt kein `-v`
  (`platform/linux/gsr.rs:177-202`). Mit `-fm cfr` ist die Zahl die
  Bildrate.
- stderr der Aufnahme geht heute roh per `Stdio::inherit()`
  (`record/session.rs:342`) nach `daemon.log` (dup2 `shelf/mod.rs:273-289`):
  ohne Zeitstempel, ohne Grenze (A10).

Ziel:
- Datei wächst 6 s nicht → stalled wie bisher.
- Neu: hat das Segment schon eine fps-Zeile geliefert („scharf“) und kommt
  6 s keine neue → stalled, Meldung „the recorder stopped encoding video for
  6 s“, gleicher Recover-Pfad.
- N unter 0,9 × fps für 3 Berichte → eine Log-Zeile, die Erholung ebenso
  (`Pace`, `capture_log.rs:60-96`).
- Jede gsr-Zeile mit Zeitstempel, höchstens 4 MiB je Segment, in
  `daemon.log` (A10).

Randfälle: bis zur ersten Zeile gilt nur der Wachstumscheck; andere Zeilen
zählen nicht (nur was `reported_fps` parst, `capture_log.rs:99-102`); neues
Format oder `-v no` → nie scharf, fail open; Suspend: beide Seiten
CLOCK_MONOTONIC; jedes Segment ist ein eigener Prozess mit eigenem Zähler.
Annahme: DPMS aus hält gsrs KMS-Schleife nicht an (falls doch: falscher
Stall bei Monitor aus; Live-Check unten).

Nicht: kein gsr-Neustart, keine Erkennung bei wf-recorder, keine Notify bei
niedriger fps.

Entscheidungen:
- niedrige fps nur ins Log, weil eine Notify während der Aufnahme mit ins
  Video kommt und cfr die Datei gültig hält (statt Notify: sichtbarer, stört
  aber genau das Produkt).
- `capture_log` bleibt in `replay/` und wird `pub(crate)`, weil shelf schon
  `super::super::replay::process` nutzt (`shelf/font.rs:26`) und
  `platform/linux` keine neue Datei verträgt (statt Umzug: reiner Umbau).
- weiter in `daemon.log` statt eigener Datei, weil gsr-Fehler neben den
  Daemon-Meldungen stehen und M3 genau dort liest.
- `SIG_IGN` für SIGPIPE im `pre_exec`, weil gsr nach Daemon-Tod per
  PDEATHSIG SIGTERM bekommt, beim Beenden auf stderr loggt und sonst vor dem
  Trailer stirbt. Belegt: std setzt SIGPIPE vor den `pre_exec`-Closures auf
  SIG_DFL (`std/src/sys/process/unix/unix.rs:369-373`, Closures `:388`), unser
  `SIG_IGN` bleibt.
- wf-recorder auch pipen, weil es nichts kostet und Zeitstempel bringt; es
  druckt keine `\r`-Fortschrittszeilen (wf-recorder 0.6.0 `main.cpp`).

**Phase 2.**
1. `replay/mod.rs:2`: `pub(crate) mod capture_log;`.
2. `capture_log.rs`: `follow` wird `pub(crate)`, bekommt
   `fps: Option<(u32, String)>` (Soll plus Name für die Meldung) und gibt
   ein `Arc<AtomicU32>` zurück, das je geparster fps-Zeile hochzählt.
   `Pace::new(fps, what)`, Meldung „{what} slowed to …“. Replay-Aufrufer
   (`start.rs` nach S1) übergeben `(settings.fps, "replay capture")`.
3. `record/session.rs:342`: `Stdio::piped()`; im `pre_exec` (`:343-354`)
   `libc::signal(SIGPIPE, SIG_IGN)` (+1, A8 spart 4).
4. `shelf/recording/watch.rs`: Felder `reports` und `frames` je Pfad;
   `listen(&mut self, recorders, fps)` nimmt `child.stderr.take()` und ruft
   `follow`; der Rumpf von `stalled` (`:46-57`) wird ein Helfer
   `progress(map, path, value, now)`, den Datei- und Frame-Check teilen;
   `stalled` liefert den Grund; `keep_only` (`:61-64`) beschneidet alle Maps.
5. **Jede** Spawn-Stelle bekommt `listen` (ein ungelesener Pipe blockiert
   gsr nach ~25 min, ein gedroppter tötet es): `spawn_initial_segment`
   (`start.rs:98-137`, deckt `:175`, `:257`) und Resume
   (`shelf/recording/session.rs:154`, `fps` vor dem `match` holen).
   `rg spawn_segment` findet nur diese zwei Importstellen.
6. Test in `gsr.rs` (`:343`): `args` enthält nie `-v`.

Tests:
- `watch.rs`: `frames_that_stop_while_the_file_grows_stall` (Größe wächst,
  Berichte 1..3, dann Stillstand → nach 6 s stalled). Heute meldet der
  Wachstumscheck nichts.
- `no_reports_falls_back_to_growth`; bestehender Test `:185-197` an die neue
  Signatur.
- `capture_log.rs`: Zähler nach zwei fps-Zeilen und einer anderen auf 2.

Risiken: eine vergessene `listen`-Stelle (Schritt 5); ein Lese-Thread je
Segment, endet bei EOF; viele Pausen summieren die 4-MiB-Grenze.

Live (mit Okay): 30 s aufnehmen → ~30 Zeilen „ZEIT gsr info: update fps“ in
`daemon.log`; Pause und Resume, Zeilen laufen weiter; `kill -STOP` auf gsr
pausiert weiterhin nach ~6 s; einmal Monitor per DPMS aus während der
Aufnahme (prüft die Annahme). M3 (echter Encoder-Hang) bleibt offen.

---

## LR3: Capture-Log behält das Wichtige

**Phase 1.** Schreiben endet bei 4 MiB (`capture_log.rs:15`, `:45-48`),
~19 h bei einem Bericht je Sekunde. `open` (`:18-22`) rotiert, sobald gsr
gestartet ist (Replay-Start, `mod.rs:512`), vor der Bereit-Schleife
(`:570-593`): zwei gescheiterte Neustarts löschen das Log der gestorbenen
Session.

Ziel:
- (a) `update fps`-Zeilen im Soll nach den ersten 60 Berichten weglassen,
  eine je 3600 bleibt als Lebenszeichen; jeder Bericht unter 90 % und jede
  andere Zeile bleibt. Gilt für Replay und, über L2, auch die Aufnahme.
- (b) `open` schreibt nach `replay-capture.log.starting`; neues
  `promote(dir)` benennt `log → .old` und `.starting → log` erst, wenn der
  Start bereit ist. Ein gescheiterter Versuch bleibt in `.starting` bis zum
  nächsten.
- Die 4-MiB-Grenze bleibt als Notbremse, mit einer Marker-Zeile beim
  Erreichen.

Entscheidungen: Filter statt Größen-Rotation, weil Rotation `.old`
überschreibt, also genau das Crash-Log (der Filter nimmt ~99 % des
Volumens); zählend (60 Berichte) statt zeitlich, weil deterministisch
testbar. Bekannte Grenze: ein gsr, das Warnungen spammt, verliert weiter das
Ende.

Nicht: der Worker → `daemon.log`-Weg (`fps = None`, kein Filter).

**Phase 2.**
1. `capture_log.rs`: `struct Keep` mit `fn keep(&mut self, line, got:
   Option<u32>) -> bool`, in `follow` (`:45`); `open` auf `.starting`;
   `pub(super) fn promote(dir)`.
2. `replay/start.rs` (nach S1, früher `mod.rs:584-585`): `promote` vor
   `return Ok(session)`, nur wenn das Log offen ist.
3. Tests: `every_line_gets_the_time_and_the_old_log_stays` (`:127`) neu;
   zwei `open` ohne `promote` lassen `replay-capture.log` stehen (scheitert
   heute); 70 ruhige Berichte plus ein langsamer ergeben 61 Zeilen.
4. `docs/replay.md`, Abschnitt Logs und Load.
- Risiko: Umbenennen während der Lese-Thread schreibt; unter Linux folgt der
  fd der Inode, unkritisch.

---

## L3 (mit R3): Kontingent zählt nur, was zum Shelf will

**Phase 1.**
- Aufrufer des 2-GiB-Kontingents (`record/finalize.rs:766-786`, Meldung
  `:788-796`): `watch.rs:120` (nur solange nicht lang), `start.rs:19`
  (Start), `clients.rs:22-27` (Import), `replay/mod.rs:451` (Replay-Start
  und jeder Watchdog-Neustart), `replay/mod.rs:744-747` (Clip).
- Ursache: `read_dir` summiert jede Datei im Cache, auch die Segmente der
  langen Session und `boltsnap-segment-<totepid>-*`-Waisen. Jeder Aufrufer
  zeigt „recording paused …“.
- Ziel: eigene Segmente (mit `.ts`, `.cursor*`) zählen nicht, solange die
  Session lang ist; fremde PIDs zählen nie (R3). Waisen belegen weiter Platz,
  `check_recording_reserve` begrenzt das. Jeder Aufrufer sagt, was gerade
  scheitert.
- Identität: Replay läuft im Daemon (`replay/mod.rs:596`), eigene Segmente
  heißen `boltsnap-segment-<std::process::id()>-`; eine Session je Daemon.
- Randfälle: `shelf_to_disk_after = 0` → nie lang, zählt immer (wie heute);
  als rettbar behaltene eigene `.mp4` zählen, bis die Session lang ist.
- Nicht: Grenze und Reserve bleiben.

Entscheidung: ein prozessweites `AtomicBool RECORDING_ON_DISK` in
`finalize.rs`, gesetzt aus `check_recording` jede Sekunde (`watch.rs:118`),
weil Replay- und Import-Threads `Daemon.recording` nicht sehen (statt eigene
Segmente nie zu zählen: dann pausiert ein Clip eine kurze Aufnahme, statt
selbst zu scheitern). Veraltet heilt sich: eine neue Session hat vor ihrem
ersten Check ~0 Bytes; eine pausierte lange Session bleibt `true`, richtig.

**Phase 2.**
1. `finalize.rs`: reine `fn cache_bytes(dir, own_pid, on_disk) ->
   Result<u64, String>`, parst den Präfix, `NotFound` toleriert wie heute;
   `check_recording_cache_capacity` ruft sie mit dem Static. Neutrale
   Meldung „the temporary shelf cache is full (X GB of 2 GiB); save or
   dismiss shelf items“.
2. Präfix je Aufrufer: `watch.rs:120` „recording paused: …“, `start.rs:167`
   und `:247` „cannot start recording: …“ (an den Aufrufstellen, weil der
   Import `prepare_recording_cache` teilt), `clients.rs:25-26` „cannot add
   the video to the shelf: …“, `replay/mod.rs:451` „replay cannot start: …“,
   `:744` „clip not saved: …“.
3. Tests in neuem `record/finalize/tests/cache.rs`: Segment fremder PID mit
   `LIMIT` Bytes → `check_recording_cache_limit` Ok (scheitert heute);
   `cache_bytes` mit eigenem Segment: 0 bei `on_disk`, sonst alles. Der
   String in `tests.rs:618` wird angepasst. Nur `cache_bytes` mit expliziten
   Parametern testen, damit das Static keine Tests verkoppelt.
- Risiko: `finalize.rs` wächst (~20 Zeilen); zusammen mit L1 bleibt sie
  unter ~925.

---

## R9: Zielplatte von `record_dir` während der Aufnahme prüfen

**Phase 1.** Liegt `record_dir` auf einer anderen Platte und ist die
Session lang (`long && separate && !composed`), prüft heute niemand deren
Platz; der Save scheitert spät (Segmente bleiben). Gebraucht für die
Zwischenlösung F2 und auch nach L1, weil ein Save über Dateisysteme R kopiert
(`copy_then_remove` braucht `remux_space`).

Ziel: Pause, wenn `frei(record_dir) ≤ remux_space(aufgenommen) + SLACK`;
eine Warnung `WARN_AHEAD_BYTES` (2 GiB) vorher; Titel „Low disk space“, die
Meldung nennt den Ordner. Nicht: keine Mount-Überwachung; ein ausgehängtes
Ziel fällt auf das Gerät des Elternordners zurück, gilt als „gleiche
Platte“, die vorsichtige Seite.

**Phase 2.** `watch.rs`: `separate_disk: Option<bool>` wird
`target: Option<Option<PathBuf>>` (der Pfad gecacht, kein `Config::load()`
je Sekunde), dazu `target_warned`; `space()` wird
`verdict(&mut warned, free, stop_at)`. Test: Ziel stoppt bei
`remux_space(R) + SLACK` und warnt einmal.

---

## LR4: Flatternde Replay-Aufnahme

**Phase 1.** Jeder geglückte Neustart gibt einen neuen Namen, `Backoff::due`
(`watchdog.rs:42-50`) setzt die Serie zurück: eine Session, die bereit wird
und wieder stirbt, startet alle ~15 s plus Probe neu (`replay/mod.rs:388-440`),
ohne Log (`watchdog.rs:123-134`).

Ziel:
- Eine vom Watchdog gestartete Session, die unter 5 min nach dem Start
  stirbt, zählt als Fehlschlag derselben Serie; Wartezeiten aus
  `retry_delay` (30, 60, 300 s).
- Eine Notify beim ersten Flattern einer Serie: „replay capture stopped again
  N s after restarting; next try in 30 s“.
- Lebte sie 5 min oder länger, beginnt die Serie neu; manuelles Start/Stop
  auch (unbekannter Name, wie heute).
- Jeder Neustart und sein Ausgang per `log!` in `daemon.log`; ebenso
  `restart_failed` (`:146`).
- `restarts` im Replay-Status: geglückte Watchdog-Neustarts seit dem
  letzten manuellen Start.

Keine öffentliche Schnittstelle bricht: `src/protocol.rs` kennt Replay nicht;
der Status ist ein ad-hoc `serde_json::Value` (`replay/mod.rs:718-795`), die
Leser (`app/save.rs:114-321`) greifen per Schlüssel zu. Entscheidung: Feld
additiv, nur solange eine Session existiert, weil ältere Daemons es einfach
nicht haben (statt Versionssprung: unnötig).

Nicht: keine Anzeige im UI, kein endgültiges Aufgeben.

**Phase 2.**
1. `watchdog.rs`: `Backoff` bekommt `born: Option<Instant>` und
   `fn restarted(&mut self, name, now)` (behält `failures`, löscht
   `retry_at`); `due` entscheidet beim ersten toten Blick auf eine `born`-
   Session: unter `FLAP = 300 s` → `failed(now)` plus Notify-Flag, sonst
   Reset.
2. Ok-Zweig: `session.restarts = old.restarts + 1`, dann `restarted`.
3. `Session` bekommt `restarts: u32`; Status (`mod.rs:718-736`, `:786-795`)
   setzt `response["restarts"]`.
4. Tests: stirbt 20 s nach Neustart → vor +30 s nicht fällig; drittes
   Flattern → 300 s; lebte 6 min → sofort fällig; unbekannter Name setzt
   zurück. Scheitert heute (neuer Name setzt die Serie zurück).
5. `docs/replay.md`: Watchdog-Absatz und Status-Zeile `:29`.
- Risiko: ein echter Einzeltod kurz nach einem Neustart wartet 30 s länger.
  Hinnehmbar.
- Live (mit Okay): gsr zweimal binnen 5 min per SIGKILL → zwei Log-Zeilen,
  eine Notify, zweiter Versuch frühestens nach 30 s, `restarts: 2`.

---

## L1: Kein zweiter voller Datensatz beim Speichern

**Phase 1.**
- Heute: gsr schreibt MKV (`gsr.rs:177-201`, `-c mkv`,
  `cluster_time_limit=100`). Jeder Save remuxt mit `-video_track_timescale
  fps` (`finalize.rs:322-345`), weil Matroska fest 1 ms hat
  (`matroskaenc.c:2593`, Memory „ms frame times“). Die Abkürzung
  `finalize.rs:317-320` greift nur für eine einzelne `.mp4`, heute nur von
  wf-recorder. `watch.rs:139-148` reserviert `remux_space(R)` auf `/`.
- Weg (a) aus der Quelle: ab lavf 62.6.101 setzt gsr für mp4 selbst
  `movflags=+hybrid_fragmented` und `use_editlist=1`
  (`src/recorder/muxer.c:86-103`, gerufen `recorder.c:138`). Installiert ist
  lavf 62.12.103. Beim Schreiben entsteht ein fragmentiertes MP4 (vorn
  `free`, dann ftyp+moov, dann moof/mdat, `movenc.c` ~8486-8506); der
  Trailer schreibt ein normales moov ans Ende und macht erst dann das vordere
  `free` zu `mdat` (`movenc.c` 8715-8775). Ein abgebrochenes gsr hinterlässt
  also ein spielbares fMP4, ein sauberer Stop ein normales MP4 mit moov am
  Ende, wie heute nach dem Remux.
- Fragmente: Standard eines je Keyframe (~2 s bei `-keyint 2`);
  `frag_duration=100000` bringt es auf ~100 ms wie die Cluster heute.
- Zeitbasis: Video 1/fps (`video_codec.c:57-60`), movenc verdoppelt bis
  ≥ 10000 (`movenc.c:8235-8237`): bei 120 fps 15360, 128 Ticks je Frame,
  exakt für jede ganze fps. Damit braucht es keinen Remux für das Raster.
- Ziel: ein Ein-Segment-Save auf demselben Dateisystem ist ein Rename,
  keine zweite Kopie, keine 45 bis 70 s I/O. Die Aufnahme pausiert erst an
  der Reserve (2 GiB + 256 MiB), nicht bei der halben freien Platte. Trifft
  den Nutzer: Combined ist ein gsr-Strom (`record/session.rs:503-505`).
- Randfälle: Pause/Resume, wf-recorder und alte MKV-Waisen laufen weiter
  über Concat; ein mitten im Save gekilltes gsr hinterlässt ein
  unfertiges fMP4, das wird erkannt und doch remuxt.
- Nicht: Replay-Clips bleiben MKV-basiert; keine Restzeit-Anzeige (D2); kein
  Wegfall des Concat bei mehreren Segmenten.

**Probe (braucht das Okay des Nutzers):** nimmt den Bildschirm auf, ein
zweites NVENC-gsr neben dem Replay-gsr, ~45 MB je Datei im Scratchpad.

```sh
P=/tmp/claude-1000/-home-mt-projects-boltsnap/0b71c020-3ffd-4770-82b0-d551e996ca2f/scratchpad/plan-l1; mkdir -p "$P"
A="-w DP-3 -c mp4 -f 120 -fm cfr -k h264 -q very_high -bm qp -keyint 2 -cursor yes -fallback-cpu-encoding no -a device:default_output -ac aac -ffmpeg-opts flush_packets=1;frag_duration=100000"
timeout -s KILL 10 gpu-screen-recorder $A -o $P/kill.mp4 2>$P/kill.log
\time -f %e timeout -s INT 10 gpu-screen-recorder $A -o $P/clean.mp4 2>$P/clean.log
for f in kill clean; do head -c 16 $P/$f.mp4 | xxd
  ffprobe -v error -show_entries format=duration:stream=codec_type,time_base,start_time -of compact $P/$f.mp4
  ffprobe -v error -select_streams v:0 -show_entries packet=pts -of csv=p=0 $P/$f.mp4 | awk 'NR>1{c[$1-p]++}{p=$1}END{for(d in c)print d,c[d]}'
  nice ffmpeg -v error -xerror -i $P/$f.mp4 -f null - && echo decodes; done
grep -i "buggy\|hybrid" $P/*.log
printf "file '%s'\n" $P/clean.mp4 $P/clean.mp4 >$P/l.txt; ffmpeg -v error -f concat -safe 0 -i $P/l.txt -c copy $P/j.mp4
```

Danach dieselbe pts-Prüfung auf `j.mp4`. Optional je 60 s MKV und MP4,
`VmRSS` von gsr alle 10 s. Aufräumen per `safe-rm`.

Bestanden, wenn: `clean.mp4` hat `mdat` an Byte 12-16, `kill.mp4` `free`
und dekodiert ganz; ein einziges pts-Delta (128) über die Datei und über die
Concat-Naht; Video-`start_time` 0; letzter pts von `kill.mp4` binnen ~0,15 s
vor dem Ende; keine „buggy FFmpeg“-Warnung.

**Phase 2 (nur wenn die Probe besteht).**
1. `gsr.rs:181`, `:199-200`: `"mp4"`, `flush_packets=1;frag_duration=100000`;
   Test `:364` anpassen.
2. `record/recover.rs`: `pub(crate) fn is_gsr_segment(path)` (`.mkv` oder
   Name endet auf `.gsr.mp4`); `segment_name` (`:21-29`) streift `.gsr.mp4`
   vor `.mp4` ab (Labels sind `[A-Za-z0-9_]`, eindeutig); Modul-Doku `:1-4`;
   Tests dort.
   Entscheidung: die Markierung im Dateinamen, weil sie Absturz und A1-
   Rettung übersteht und `session.rs` nicht wachsen darf (statt `gsr: bool`
   auf `ActiveRecorder`: mehr Zeilen in `session.rs`, geht bei der Rettung
   verloren). wf-recorder-`.mp4` muss „nicht gestreamt“ bleiben.
3. `record/session.rs` ohne Zeilenzuwachs: `:432` →
   `if gsr_codec.is_some() { "gsr.mp4" } else { "mp4" }`; `is_streamed`
   (`:626-628`) ruft `is_gsr_segment`.
4. `watch.rs`: `:107` nutzt `is_gsr_segment`, Doku `:10-11`; Remux-Raum nur,
   wenn wirklich gefügt wird (`composed`, mehr als ein Segment, oder eine
   aktive `.mkv`).
5. `finalize.rs`: Raster-Prüfung `:331-335` → `all(is_gsr_segment)`;
   Abkürzung `:317-320` nur, wenn kein unfertiges Hybrid (`fn
   finished_mp4(path)` liest 16 Bytes: unfertig, wenn 4..8 und 12..16 beide
   `free`); bei `SaveDestination::Shelf` ein Ergebnis, das noch
   `boltsnap-segment-*` heißt, nach `work_path(segment_dir, "final", …)`
   umbenennen (sonst sähe die Shelf-Karte wie eine Waise aus, A1 würde sie
   „retten“ und L3 zählte sie als Session). ~25 Zeilen.
6. Tests in neuem `record/finalize/tests/streamed.rs`: Disk-Save einer
   fertigen `.gsr.mp4` mit scheiterndem Fake-ffmpeg gelingt mit gleicher
   Inode; eine unfertige ruft ffmpeg; der Shelf-Kartenpfad beginnt nicht mit
   `boltsnap-segment-`. In `watch.rs`: `stop_at` für ein einzelnes gsr-mp4
   ist Reserve + Slack.

Risiken:
- gsr-RAM wächst um ~72 B je Sample (`MOVIentry`), ~124 MB nach 4 h bei
  120 fps (Annahme aus der Quelle, die optionale RSS-Messung prüft es).
  Neben LR1 auf 15 GB nicht egal.
- Trailer-Dauer bei langen Dateien ungemessen; die Stop-Frist für gestreamte
  Segmente ist 1,5 s (`record/session.rs:676-678`), ein Kill dort lässt ein
  fMP4, das Schritt 5 remuxt. Annahme: ein abgeschnittenes zweites moov
  überspringt FFmpegs Demuxer („duplicated MOOV“); die Probe deckt das nicht
  ab, Test mit abgeschnittener `clean.mp4` nachziehen.
- gsr auf FFmpeg < 8: kein Hybrid, eine dauerhaft fragmentierte Datei, gsr
  warnt selbst („buggy FFmpeg“, `muxer.c:99`). Dokumentieren: „gsr gegen
  FFmpeg ≥ 8“ in README-Voraussetzungen. Ein automatischer Rückfall auf MKV
  ist nicht geplant.
- `use_editlist` könnte die Smooth-Cursor-Spur verschieben; die Probe prüft
  `start_time`.

---

## LR1: gsr-Speicher (erst M1, Fix nur bei Befund)

**M1 (braucht das Okay des Nutzers, nach LR3 und einem frischen
Replay-Start).** Ein `python3 -I`-Skript mit `nice -n 19`, nur stdlib,
minütlich eine TSV-Zeile nach
`/tmp/claude-1000/-home-mt-projects-boltsnap/0b71c020-3ffd-4770-82b0-d551e996ca2f/scratchpad/m1/samples.tsv`
(~0,4 MB je Tag), Ereignisse nach `…/m1/events.log`. Befunde danach in die
Review-Datei, der Scratchpad kann weg.
- Je Zeile: Zeit (Format wie Capture-Log), `boottime − monotonic`
  (Sprung = Suspend), gsr-PID je Zeile neu gesucht (Kind des Daemons, comm
  `gpu-screen-reco`, cmdline mit `-o /dev/stdout`) plus `etimes`;
  aus `status` VmHWM, VmRSS, RssAnon, RssFile, VmData, VmSwap, Threads; aus
  `smaps_rollup` Private_Clean, Private_Dirty, Swap, SwapPss, Anonymous; aus
  `smaps` die größte anonyme VMA (Bereich, Größe, Rss, Swap), Zahl der
  anonymen VMAs, Rss der `/dev/nvidia*`-Mappings; fd-Zahl und
  `anon_inode:dmabuf`; Worker privat + Swap; `MemAvailable`, `SwapFree`;
  je Monitor Name, DPMS, Modus über `hyprctl -j monitors` (read-only).
- Ereignis-Thread liest Hyprlands `.socket2.sock` (`monitoradded`,
  `monitorremoved`, `fullscreen`, `focusedmon`, `configreloaded`). Annahme:
  socket2 gibt es auch unter der Lua-Config.
- Auswertung: jeder Sprung > 64 MB in privat + Swap oder VmHWM gegen
  events.log, `replay-capture.log`, `daemon.log` (±2 min); welche Mapping
  wuchs: `[heap]` oder 64-MB-Arenen → glibc, eine große anonyme VMA neben
  den nvidia-Mappings → Treiber.

**Fix (nur wenn M1 residentes Wachstum oder wiederholte Sprünge zeigt).**
Ein einmaliger Ausreißer, der im Swap liegt, kostet keinen RAM: dann reichen
LR2 und ein manuelles Stop/Start, gebaut wird nichts.
- Maß wie LR2 (`Private_Clean + Private_Dirty + SwapPss`) aus
  `/proc/<capture>/smaps_rollup`, alle 60 s, nur bereit und ohne gehaltenen
  Puffer; Basis = erste Probe ≥ 2 min nach Bereitschaft.
- Grenze `max(2 × Basis, Basis + 256 MiB)`. Entscheidung: relativ statt fest
  512 MiB, weil die Basis je Monitor-Setup und GPU unbekannt ist.
- Hysterese: 5 Proben in Folge darüber → Neustart; eine unter 0,9 × Grenze
  setzt zurück. Höchstens ein Speicher-Neustart je 6 h, sonst nur Log.
- Weg: `watchdog.rs:91-139` wird `fn restart(reason)` (geteilt mit LR4);
  `stop_capture`, `start`, `replace`, `retire`, der alte Ring bleibt bis zum
  Wechsel speicherbar; zählt nicht als Flattern.
- Sichtbar: eine Notify „Replay restarted GPU Screen Recorder: it held X MB
  (normal ~Y MB); the buffer starts over“, eine Log-Zeile mit Zahlen.
- Parser nach `replay/process.rs` (mit `#[allow(dead_code)]`, der Worker
  bindet die Datei ein), genutzt von `load.rs` und Watchdog.
- Test: reine `Memory::sample(now, bytes)` für Basis, 5-Proben-Auslöser,
  Rücksetzen und Cooldown.

---

## Nur dokumentiert

- **L4** (Ton-Drift in gsr): hier −46 ppm, korrigiert sich selbst. Upstream
  melden erst bei echter Drift. Kein Code.
- **L6** (lange Smooth-Cursor-Saves): nur dokumentieren, weil der Nutzer
  `system` nutzt (nichts davon läuft), L1 den Auslöser „keine Dauer“
  wahrscheinlich entfernt (fertiges Hybrid-MP4 hat immer eine Dauer), und
  Cursor-Timing eine Ruckel-Vorgeschichte hat. Kleinster Fix, falls später:
  `in_background` plus zeilenweises `BufRead`, das in `packet_duration` nur
  die letzten zwei pts hält (`cursor_sidecar.rs:163-201`);
  `tracks.into_values()` statt `.clone()` (`finalize.rs:131-136`); `merge`
  nimmt `Vec<Vec<Sample>>` by value (`cursor.rs:131-145`).
- **LR5** (Smooth-Cursor-Tracker im Replay): Text in
  [../replay/2026-10-28-replay-memory.md](../replay/2026-10-28-replay-memory.md)
  Teil 4 (`:359-362`) korrigieren: „läuft mit mellow/quick die ganze
  Replay-Session, ~60 Wakeups/s je Monitor (C1)“. `docs/replay.md:117-121`
  stimmt schon. C1 bleibt in Welle 4 des 12.10.-Reviews (vendored libway,
  `vendor/libway/src/capture.rs:416`).

## Grundsätzliche Entscheidungen

1. **F1, Container der Aufnahme.** (a) gsr schreibt Hybrid-MP4: Save wird ein
   Rename, Absturzverlust ~100 ms wie heute, gsr braucht mehr RAM (~124 MB
   bei 4 h), gsr muss gegen FFmpeg ≥ 8 gebaut sein. (b) MKV bleibt,
   `record_dir` auf `/mnt/data` (455 GB frei, eigene Platte), nur Config,
   braucht R9. (c) MKV unverändert speichern: verworfen, ms-Zeiten bringen
   das Ruckeln zurück. **Empfehlung: (b) sofort, (a) nach bestandener
   Probe.**
2. **F2, `record_dir` auf `/mnt/data`.** Das ist die Config des Nutzers.
   Empfehlung: ja, nach R9. Ich ändere sie nur auf ausdrückliches Ja.
3. **F3, LR1-Neustart-Politik** (nur falls M1 einen Befund liefert). Ein
   gsr-Neustart wirft die Historie weg; der alte Ring lässt sich nur in der
   2 bis 10 s Lücke speichern. (A) automatisch mit den Schutzregeln oben;
   (B) nur Notify „Replay neu starten gibt X MB frei“; (C) wie A, aber erst
   wenn der Nutzer idle ist (`ext-idle-notify`, neuer Daemon-Code), harte
   Grenze bei 2 × weich sofort. **Empfehlung: nach M1; resident und
   wiederkehrend → A, einmalig im Swap → B oder nichts.**
4. **Live-Proben**: L1-Probe, M1, und je ein Live-Check nach L2 und LR4
   (Aufnahme, `kill -STOP`/SIGKILL auf gsr). Laut Memory nur mit Okay.

## Prüfungen je Commit

`cargo fmt --check`, `nice -n 19 ionice -c3 cargo test` (jobs = 2), für LR7
zusätzlich der Worker-Crate. Release-Build nur am Ende einer Welle, die
installiert werden soll; vorher `free -g` und `df -h /` (Anvil, wenn knapp).
Doku je Commit: Status hier, im Review vom 31.10. und, wo betroffen, im
Review vom 12.10. und in `docs/replay.md`.

## Status

| Welle | ID | Item | Commit |
| --- | --- | --- | --- |
| 0 | S1 | Replay-Start nach `replay/start.rs` | `9b377cc` |
| 1 | A8 | `log!` statt `eprintln!` in `record/` | `fa4e560` |
| 1 | LR2 | Load-Zeile mit Swap | `012d137` |
| 1 | LR7 | Socket-Fehler beenden den Worker nicht | `efce9e9` |
| 1 | LR6 | geerbte fds CLOEXEC | `8f0feb3` |
| 1 | L5 | Popup-Zeit `H:MM:SS` | `704dce0` |
| 2 | L2 | Stall über fps-Zeile, mit A10 | `85694c4` |
| 2 | LR3 | Capture-Log filtert, rotiert erst bei Bereitschaft | `2aebaef` |
| 3 | L3 | Kontingent ohne lange Session und Waisen (R3) | `822e4b4` |
| 3 | R9 | Platz von `record_dir` prüfen | siehe Log |
| 4 | LR4 | Flattern zählt als Serie, `restarts` | |
| 5 | L1 | Probe, dann Hybrid-MP4 und Rename | |
| 6 | LR1 | M1, dann Entscheidung F3 | |
| doku | L4, L6, LR5 | nur Doku | |

## Umsetzung Wellen 0 und 1 (01.11.)

Je Commit `cargo fmt --check` und `cargo test` grün (zuletzt 174 + 419,
11 ignoriert); für LR7 zusätzlich der Worker-Crate (`cargo test --locked`,
27 grün) und `worker/tests/live.py`. Nicht installiert, kein Daemon-Neustart,
nichts live geprüft.

- **S1:** reiner Umzug, `diff` gegen den alten Block zeigt nur Pfade und
  `pub(super)`. `replay/mod.rs` 724 Zeilen, `replay/start.rs` 263.
- **A8:** wie geplant; `log!` in `main.rs` über `log_to`, Test mit einem
  Writer, der `ENOSPC`-artig scheitert. `record/session.rs` 1907 Zeilen.
- **LR2:** wie geplant. Kein Screenshot: Text und Layout gleich, `WIDEST`
  deckt 99999 MB.
- **LR7, Abweichung:** statt `accept_wait` mit Fehler-Einordnung pausiert
  jeder `accept`-Fehler 100 ms, weil das kürzer ist und ein seltener
  ECONNABORTED die anderen Clients nur 100 ms aufhält. Damit entfällt der
  Unit-Test der Einordnung; `live.py` prüft EMFILE per `prlimit` und
  scheiterte vor dem Fix („Too many open files (os error 24)“, Worker
  beendet). `set_read_timeout`-Fehler verwerfen die Verbindung. Die
  Steuerschleife des Daemons pausiert ebenso.
- **LR6:** wie geplant; Test markiert ein `dup(2)` über beide Wege.
  Live-Beleg (`/proc/<gsr>/fd` ohne `sync_file`) erst nach dem nächsten
  Daemon-Neustart.
- **L5:** wie geplant. Gate 1 als Test `titles_clear_a_ten_hour_time`
  (Fallback-Font) und im ignorierten `controls_shots` mit den echten
  Desktop-Fonts (500/600), beide grün. Screenshot `controls-paused-long.png`
  (pausiert bei 1:40:00, Dark) angesehen: Zeit rechtsbündig 16 vom Rand,
  Titel frei, Panel-Breite unverändert.
- Windows: `fmt_elapsed` ist geteilt und gilt dort mit; kein
  `cargo check --target x86_64-pc-windows-msvc` gelaufen (Backend
  eingefroren).
- **Live, 01.11. nach Install (Stand `3ea608f`) und Daemon-Neustart:**
  Replay kam von allein hoch. LR6: `/proc/<gsr>/fd` ohne `sync_file`,
  `daemon.log` und Lock; die zwei `sync_file` hält nur der Daemon. LR2:
  frisch gestartet `SwapPss` 0, gsr 146 MB, Worker 16 MB; der Unterschied
  zur alten Zahl zeigt sich erst, wenn gsr in den Swap rutscht.

## Umsetzung Welle 2 (01.11.)

- **L2 mit A10:** wie geplant, zwei Abweichungen im Kleinen: `capture_log`
  ist `pub(crate)`, `follow` `pub` darin (wie `process`); der Name in `fps`
  ist `&'static str` statt `String`. `listen` sitzt nicht in
  `spawn_initial_segment` (hat kein `Watch`), sondern direkt nach den drei
  Spawns: `start.rs` Bereich und Ausgaben, Resume in
  `shelf/recording/session.rs`. `Pace::note` nimmt jetzt die Zahl, `follow`
  parst einmal und zählt. `record/session.rs` 1909 Zeilen (Planstand).
  Tests: `frames_that_stop_while_the_file_grows_stall`,
  `no_reports_falls_back_to_growth`, Zähler in
  `every_line_gets_the_time_and_the_old_log_stays`, `-v` in
  `args_cover_codec_audio_and_region`; `cargo test` 174 + 421 grün. README
  nennt den neuen Grund. Live offen (Aufnahme, Pause/Resume, `kill -STOP`,
  DPMS), braucht Install und Okay.
- **LR3:** wie geplant, ohne eigene `Keep`-Struktur: `Pace::routine(got,
  count)` entscheidet mit dem Zähler aus L2, den `follow` ohnehin führt.
  Marker-Zeile „log full at 4 MiB, later lines left out“. Tests:
  `after_the_first_minute_only_slow_reports_and_other_lines_are_kept`
  (70 ruhige + ein langsamer + eine andere Zeile → 62),
  `a_start_that_never_gets_ready_keeps_the_last_log` (scheitert am alten
  `open`), `every_line_…` mit `promote`. `cargo test` 174 + 423 grün.
  `docs/replay.md` beschreibt Filter und `.starting`. Live offen: nach dem
  nächsten Replay-Start `replay-capture.log` mit 60 Berichten, danach still.

## Umsetzung Welle 3 (01.11.)

- **L3 mit R3:** `RECORDING_ON_DISK` und `cache_bytes(dir, own_pid,
  on_disk)` wie geplant; `Watch::reset` setzt das Flag zurück, sobald keine
  Aufnahme mehr läuft (statt nur „heilt sich“). Abweichungen: (1) `NotFound`
  war entgegen dem Plan heute **nicht** toleriert; jetzt übersprungen (R8 aus
  dem 12.10.-Review, gleiche Funktion). (2) Präfixe nur dort, wo der Aufrufer
  nicht schon sagt, was scheitert: Start „cannot start recording: “, Import
  „cannot add the video to the shelf: “, Clip „clip not saved: “. Die
  Aufnahme-Pause hat schon „Recording paused: “ (`events.rs:159`), Replay-Start
  „Replay: “ (Tray) bzw. „restart failed: “ (Watchdog). (3) Die Reserve-Meldung
  „recording stopped to preserve …“ ist ebenso falsch an Start und Clip und
  heißt jetzt „the disk is down to its 2 GiB reserve (X GB free)“. Tests in
  `record/finalize/tests/cache.rs`: Waise fremder PID mit 2 GiB → Ok
  (scheitert am alten Code), eigene Segmente 1025 → 5 Bytes mit `on_disk`.
  `cargo test` 174 + 425 grün. README nennt Clips, Importe und Waisen.
- **R9:** wie geplant (`target: Option<Option<PathBuf>>`, `target_warned`,
  `verdict`). Abweichungen: (1) geprüft wird bei **jeder** langen Aufnahme
  mit getrenntem `record_dir`, auch `composed`, weil die fertige Datei dort
  auch nach dem Zusammenfügen im Cache landet (Kopie). (2) `Space::Stop`
  trägt Zahlen statt Text; die Meldung entsteht in `told` und doppelt nicht
  mehr „Recording paused: recording paused …“, sondern „only X GB free in
  DIR, saving needs Y GB“. Geprüft wird der vorhandene Ordner (oder sein
  Elternordner, wenn `record_dir` noch fehlt). Test
  `a_separate_target_keeps_room_for_the_finished_file_and_warns_on_its_own`.
  `cargo test` 174 + 426 grün.
