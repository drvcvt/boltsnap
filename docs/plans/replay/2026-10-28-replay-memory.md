# Replay-Speicher: ehrliche Anzeige, dann weniger (2026-10-28)

**Anlass:** Vergleich mit Rewa (60 MB unter Windows), Befund im Nachtrag von
[2026-10-28-replay-both-displays.md](2026-10-28-replay-both-displays.md).
Nutzer 28.10.: „okay dann machen wir 1 und planen danach 2“.

1. Die Last-Anzeige zählt nur, was dem Puffer gehört (dieser Plan, Teil 1).
2. Konstante Qualität mit Byte-Deckel statt konstanter Bitrate (Teil 2, erst
   geplant, dann geprobt).
3. Weniger Treiber-Speicher in gsr (Teil 3, Nutzer 28.10.: „koennen wir
   irgendwie noch weiter die usage runterdruecken?“).
4. Suche nach weiterer Last und Fehlern, die Leistung kosten (Teil 4,
   Nutzer 28.10.: „wir quetschen den saft raus wo es geht“).

## Teil 1: Speicher als privater Speicher

Stand: **gebaut 28.10., wie geplant.** `load.rs` `private` und
`private_of_rollup`; Tests `private_memory_is_the_clean_and_dirty_pages_no_one_else_maps`,
Prozessbaum-Test mit Rollup und ohne (statm-Rückfall). `replay_load_now`
gegen den laufenden Puffer (beide Monitore): 33 % eines Kerns, **336 MB**
(gsr 158, Worker 170, kms-Helfer per statm); vorher zeigte die Zeile ~560 MB
RSS. fmt, clippy `-D warnings`, `cargo test` 164 + 415 grün. Live im Fenster
offen (neues Binary, Fenster neu öffnen).

### Befund

`linux/app/load.rs` `resident` summiert RSS aus `/proc/<pid>/statm`. RSS
zählt jede eingeblendete Seite, auch geteilte Bibliotheken, die andere
Prozesse genauso nutzen und die beim Beenden des Puffers nicht frei würden.
Live mit beiden Monitoren (`smaps_rollup`, 28.10.):

| Prozess | RSS | privat (Clean + Dirty) | davon anonym |
| --- | --- | --- | --- |
| gsr | 250 MB | 159 MB | 19 MB |
| Worker | 187 MB | 170 MB | 166 MB |
| `gsr-kms-server` | 1,7 MB | nicht lesbar (Capabilities) | |

Die 121 MB `/dev/nvidiactl` von gsr sind privat eingeblendeter Treiber-
speicher; sie zählen als privat mit (der Treiber gibt sie beim Beenden frei).

### Verhalten

- Die Zahl rechts in „Load“ ist die Summe des **privaten** Speichers der
  Puffer-Prozesse: `Private_Clean + Private_Dirty` aus
  `/proc/<pid>/smaps_rollup` (USS, das, was beim Beenden frei wird; wie der
  private Working Set unter Windows). Text und Layout bleiben gleich.
- Nicht lesbares `smaps_rollup` (der `gsr-kms-server` mit Capabilities, ein
  Kernel ohne die Datei): für diesen Prozess `statm` resident − shared (sein
  anonymer Teil), damit er nicht verschwindet.
- Prozess weg zwischen Suche und Lesen: fehlt in der Summe wie heute.
- Kosten: ein Lesen von `smaps_rollup` für gsr dauert ~3 ms (gemessen
  28.10.), einmal je Sekunde für drei Prozesse.

### Code

- `src/platform/linux/app/load.rs`: `resident` → `private` (Rollup lesen und
  parsen, Rückfall auf `statm`), Doku-Kommentare oben und an `Meter`.
- `docs/replay.md` Abschnitt Load: „private memory“.

### Abnahme

- Test: Rollup-Zeilen → Summe von Private_Clean und Private_Dirty in Bytes;
  ohne Rollup → statm (resident − shared) × Seite; der bestehende
  Prozessbaum-Test liest über `private`.
- `replay_load_now` (ignoriert, liest nur `/proc` des laufenden Puffers):
  Zahl ≈ Summe der privaten Spalte oben (~330 MB statt ~440 MB RSS).
- fmt, clippy, `cargo test`.

**Macht nicht:** keine andere CPU-Messung, kein VRAM, keine neue Zeile oder
Erklärung im UI, kein Sparen (das ist Teil 2).

## Teil 2: Konstante Qualität mit Bitrate-Deckel

Stand: **gebaut 28.10., wie geplant, P3 live auf ruhigem Bild bestanden
(unten).** Offen: Last-Zeile bei viel Bewegung live (der Deckel ist nur
synthetisch in P1 gemessen).

### Befund

- Heute: `-bm cbr -q <video_kbps>` (`replay/mod.rs` `start_capture`;
  `replay/settings.rs` `video_kbps`, bei 30 s und 512 MiB 41,4 Mbit/s). CBR
  füllt auch ein ruhiges Bild auf die volle Rate auf; der Ring hält deshalb
  immer ~155 MB (Worker 166 MB anonym, Teil 1). Der Kommentar dort lehnt
  konstante Qualität ab: „grows with motion and silently shortens the clip
  once the byte budget evicts“. Das gilt für reines QP ohne Deckel.
- gsr 6.1.6 (`src/recorder/video_codec.c`, Quelle `099a55b`): bei CBR setzt
  es `bit_rate = rc_max_rate = rc_buffer_size = video_bitrate` (Z. 90–95)
  und `rc=cbr` in die Optionen (`video_set_rc`, Z. 297); danach liest es
  `-ffmpeg-video-opts` **in dasselbe Dictionary** (Z. 414 f.,
  `av_dict_parse_string`, Flags 0 = überschreibt). Eigene `rc`, `cq`, `b`
  ersetzen also gsrs Werte; `maxrate` und `bufsize` bleiben auf unserer Rate.
- FFmpeg 8.1 `h264_nvenc`: `-rc vbr` plus `-cq` (0–51, „target quality
  level … for constant quality mode in VBR rate control“).
- Der Encoder ist `Codec::encoder` aus `gsr::choose` (`h264_nvenc` usw. bei
  NVIDIA, `*_vulkan`/`*_vaapi` sonst).
- Ring-Nutzlast `Arc<[u8]>` je Paket über glibc-malloc (`replay/ring.rs`);
  ob der Worker nach einer Bewegungs-Spitze Speicher zurückgibt, ist
  ungemessen (FIFO-Freigabe lässt Löcher unten im Heap).

### Verhalten

- Bei NVENC nimmt gsr **VBR mit Zielqualität** (`rc=vbr;cq=<N>;b=0`) unter
  der heutigen Rate als Deckel (`maxrate = bufsize = video_kbps`, wie heute
  von gsr gesetzt). Ruhiges Bild (Desktop, viele Anime-Szenen): wenige
  Mbit/s, der Ring belegt entsprechend wenig. Volle Bewegung: der Deckel
  greift, Größe und Qualität wie heute. Die gehaltenen Sekunden bleiben
  `seconds` (der Ring evictiert nie früher als heute, weil keine Sekunde
  größer wird als heute).
- `memory_mib` bleibt die Obergrenze und das Budget des Rings; die Seite
  sagt „Video at up to about 41 Mbit/s“ statt „at about“ (gilt auch für
  CBR, bleibt also wahr).
- Die Last-Zeile zeigt den echten, jetzt wechselnden Verbrauch.
- Vulkan, VAAPI (AMD/Intel): unverändert CBR (andere Optionsnamen, hier
  nicht prüfbar).

### Proben vorher (Abbruch, wenn eine scheitert)

- **P1, synthetisch, nur FFmpeg mit `h264_nvenc`** (kein Desktop, keine
  gsr-Aufnahme; je Lauf 5 s, 3840 × 1080 bei 60 fps, Dateien im Scratchpad,
  danach per `safe-rm` weg, zusammen < 200 MB): Quellen ruhig (ein echter
  Screenshot, still, plus ein kleiner bewegter Teil) und viel Bewegung
  (`testsrc2` mit Rauschen). Je CBR 41,4 Mbit/s gegen VBR mit `cq` 19, 23,
  26 und `b=0`, `maxrate = bufsize = 41,4M`, sonst die Optionen von gsr
  (`bf=0`, `+cgop`, GOP 60, Preset wie `-tune performance`). Messen: Bytes
  je Sekunde, größte 1-s-GOP (**muss ≤ 1,1 × CBR-Sekunde** sein, sonst
  kürzt der Ring), PSNR/SSIM gegen die Quelle. Wahl von `cq`: das größte,
  bei dem ruhig PSNR ≥ 45 dB und bewegt PSNR ≥ CBR − 0,3 dB.
- **P2, Worker-Speicher nach einer Spitze, synthetisch** (bestehender
  Live-Test `worker/tests/live.py` mit eigener Quelle): 10 s viel Bewegung,
  dann 40 s ruhig; Worker-RSS (anonym) folgen. Fällt sie nach dem Ausräumen
  nicht, nach jedem Evict eines GOP `malloc_trim(0)` (höchstens einmal je
  Sekunde; libc, kein neues Crate) und P2 noch einmal.
- **P3, echt mit gsr, nur mit Okay des Nutzers** (Desktop-Probe): nach dem
  Bau Replay neu starten (Knopf in den Einstellungen), `replay-capture.log`
  ohne Fehler zu `rc`/`cq`, Last-Zeile ruhig und bei Bewegung, ein
  Vollbild-Clip: `ffprobe` Bitrate und Länge 30 s.

### P1, gemessen 28.10.

`h264_nvenc` (System-FFmpeg 8.1.3) mit gsrs Optionen (`tune ll`,
`forced-idr`, GOP 60, `bf=0`, `+cgop`, CABAC, High), 5 s 3840 × 1080 @ 60,
`maxrate = bufsize = 41435k`; Skript `probes/p1.py` im Scratchpad der
Session (nicht im Repo). PSNR/SSIM gegen dieselbe Quelle neu erzeugt.

| Quelle | Modus | Mbit/s | größte 1-s-GOP | PSNR | SSIM |
| --- | --- | --- | --- | --- | --- |
| ruhig (2 Screenshots + bewegtes 160-px-Feld) | CBR | 41,43 | 41,43 | 50,46 | 0,99983 |
| | VBR cq 19 | 2,31 | 2,48 | 50,16 | 0,99932 |
| | VBR cq 23 | 1,99 | 2,13 | 49,76 | 0,99877 |
| | VBR cq 26 | 1,71 | 1,83 | 49,23 | 0,99839 |
| Schwenk (240 px/s) | CBR | 41,43 | 41,43 | 44,67 | 0,99968 |
| | VBR cq 19 | 1,33 | 1,43 | 44,66 | 0,99967 |
| | VBR cq 23 | 1,21 | 1,30 | 44,62 | 0,99960 |
| | VBR cq 26 | 1,11 | 1,20 | 44,55 | 0,99946 |
| Rauschen (Worst Case) | CBR | 41,55 | 44,47 | 35,54 | 0,7919 |
| | VBR cq 19 | 40,37 | 44,71 | 35,52 | 0,7920 |
| | VBR cq 23 | 40,37 | 44,71 | 35,52 | 0,7920 |
| | VBR cq 26 | 40,11 | 42,44 | 35,52 | 0,7918 |

Der Deckel hält: die größte Sekunde (44,71) bleibt wie bei CBR (44,47)
unter 1,1 × 41,4 = 45,6 Mbit/s. **Abweichung vom Kriterium:** gewählt ist
cq **19**, nicht das größte zulässige (26): 26 spart auf ruhigem Bild nur
0,6 Mbit/s (~2 MB auf 30 s) und kostet 0,9 dB; 19 liegt ruhig 0,3 dB und
im Schwenk 0,01 dB unter CBR.

### P2, gemessen 28.10.

Skript `probes/p2.py` (Scratchpad): FFmpeg speist den Worker (`live … 30
512 h264_nvenc`) in Echtzeit, 12 s Rauschen 1920 × 1080 (VBR cq 23 unter
41,4M), dann 45 s ruhige Fläche; je Sekunde `Anonymous` aus
`smaps_rollup` und `bytes` aus dem Status.

| t | ohne Trim: anonym / Ring | mit Trim: anonym / Ring |
| --- | --- | --- |
| 12 s (Spitze) | 56,0 / 53,0 MB | 56,0 / 53,0 MB |
| 35 s | 56,6 / 28,3 MB | 31,9 / 28,3 MB |
| 41 s (Spitze ausgeräumt) | 56,6 / 2,1 MB | 7,3 / 2,1 MB |
| 56 s | 56,6 / 2,1 MB | 7,3 / 2,1 MB |

Ohne Trim behält glibc den Speicher der ausgeräumten GOPs. **Gebaut:**
`worker/src/live.rs` `capture`: nach dem Push, außerhalb des Locks, einmal
je Sekunde `libc::malloc_trim(0)`, wenn der Ring seit dem letzten Trim um
mehr als `TRIM_AFTER` (4 MiB) unter seinen Höchststand fiel. Worker-Tests
27 grün, `cargo fmt --check`, clippy `-D warnings` ohne eigene Meldung
(die vorhandene `dead_code`-Meldung zu `process.rs` `output_setup` stammt
nicht von hier, mit `git stash` gegengeprüft). Nicht installiert.

### P3, live 28.10.

Gebaut und installiert: `video_opts` in `replay/mod.rs` (Test
`only_nvenc_trades_constant_bitrate_for_quality_under_the_cap`), Hinweis „Video
at up to about“, Worker mit Heap-Trim. fmt, clippy `-D warnings`, `cargo test`
164 + 416 grün. Daemon neu, beide Monitore, gsr läuft mit
`-ffmpeg-video-opts bf=0;flags=+cgop;rc=vbr;cq=19;b=0`.

- `replay-capture.log`: keine Meldung zu `rc`, `cq` oder `b`; 60–61 fps.
- Nach 29 s ruhigem Desktop: Ring **17,1 MB** (vorher ~155 MB, ≈ 4,7 Mbit/s
  für beide Monitore), Worker privat 22,7 MB (vorher 170).
- `replay_load_now`: 21 % eines Kerns, **223 MB** (vorher 336 MB); der Rest
  ist fast ganz gsr (~195 MB privat, überwiegend Treiber-Speicher).
- `boltsnap replay save` (Vollbild, Monitor unter dem Zeiger): 1920 × 1080,
  60 fps, 1772 Frames, **29,57 s**, 0,62 Mbit/s, Export 4,4 s.
- Nicht live gemessen: Last-Zeile bei viel Bewegung (Desktop-Probe, nicht
  ohne Anlass); P1 zeigt, dass die größte Sekunde dann wie bei CBR bleibt.

### Code (nach den Proben)

1. `src/platform/linux/replay/mod.rs`: `fn video_opts(encoder: &str) ->
   String` (`bf=0;flags=+cgop`, bei `*_nvenc` dazu `;rc=vbr;cq=<N>;b=0`),
   Kommentar an `-bm cbr` neu (CBR setzt den Deckel, die Optionen machen
   daraus VBR mit Zielqualität). Test: NVENC bekommt VBR, Vulkan und VAAPI
   nicht.
2. Je nach P2: `worker/src/live.rs` (oder `ring.rs` im Worker-Pfad)
   `malloc_trim` nach Evicts.
3. `src/app/pages/mod.rs` `memory_hint`: „up to about“; Copy-Gate.
4. `docs/replay.md` (Bitrate-Abschnitt), README falls dort CBR steht, dieser
   Plan mit Messwerten.

### Was brechen könnte

- gsrs FFmpeg (eigene Version) lehnt `b=0` mit VBR ab oder ignoriert `cq`:
  P3 sieht es im Log und an der Rate; dann `b` weglassen und neu messen.
- NVENC überschreitet `maxrate` kurz (VBV): P1 misst die größte Sekunde;
  die 10 % Reserve in `video_kbps` fängt kleine Spitzen.
- Region-Clips kodieren im Worker neu (`crop.rs`) mit eigener Rate:
  geprüft 28.10., `worker/src/crop.rs` setzt für NVENC und Vulkan fest
  `-qp 18` (VAAPI `-global_quality 18`) und übernimmt keine Quell-Bitrate.
- Thumbnails, Vorschau, Vollbild-Saves kopieren nur: unberührt.

### Abnahme

P1-Tabelle hier eingetragen; P2 RSS-Verlauf; Test `video_opts`; fmt,
clippy, `cargo test`; P3 mit Last-Zeile ruhig und bewegt und einem Clip.

**Macht nicht:** keinen Schalter in den Einstellungen, nichts für Vulkan,
VAAPI oder CPU, keine Änderung an Aufnahmen, Länge oder Speicher-Budget,
keinen Ring auf der Platte.

## Teil 3: NVENC-Sitzung schlanker

Stand: **gebaut 28.10., wie geplant, live bei 60 fps bestanden (unten).**
Offen: hohe Replay-fps (bis 240) live; nur synthetisch ohne Verlust.

### Befund

Nach Teil 2 (ruhiger Desktop, beide Monitore): Last 223 MB. gsr privat
154 MB, davon `/dev/nvidiactl` 121 MB (vom Treiber eingeblendeter Host-
Speicher für CUDA und NVENC), anonym 14 MB, `libcuda` 9 MB; Worker 23 MB,
kms-Helfer 0,4 MB. gsr nutzt die System-Libs (`libavcodec.so.62`, dieselben
wie `ffmpeg` 8.1.3), einen CUDA-Kontext (`src/cuda.c` `cuCtxCreate_v2`) und
setzt für NVIDIA weder `surfaces` noch `async_depth`
(`src/recorder/video_codec.c` ab Z. 311).

**P4, synthetisch 28.10.** (`probes/p4/p4.sh`, `speed.sh` im Scratchpad):
`ffmpeg` mit `hwupload_cuda` und gsrs NVENC-Optionen (VBR cq 19 unter
41,4M), 3840 × 1080 @ 60, `/dev/nvidiactl` privat nach 5 s:

| Variante | `/dev/nvidiactl` |
| --- | --- |
| nur CUDA-Kontext (kein NVENC) | 61,9 MB |
| NVENC Standard | 113,0 MB |
| `surfaces=4` / `3` / `2` / `1` | 113,0 / 100,5 / 88,0 / 75,6 MB |
| `delay=0`, `CUDA_MODULE_LOADING` lazy/eager | 113,0 MB (kein Effekt) |
| `CUDA_DEVICE_MAX_CONNECTIONS=1` | 72,0 MB |
| beides (`connections 1`, `surfaces=2`) | **47,0 MB** |

Der Standard sind 4 Surfaces (~12,5 MB je Surface bei 3840 × 1080). CUDA legt
je Hardware-Queue Befehlspuffer an (Standard 8); gsr kodiert über einen
einzigen Stream, mehr Queues bringen ihm nichts. Durchsatz so schnell es geht
(1200 Frames, Schwenk): Standard 215, `surfaces=2` 208, beides 210 fps
(Grenze ist dort das CPU-Upload von `ffmpeg`, nicht NVENC; kein messbarer
Verlust).

### Verhalten

- Bei NVENC startet Replays gsr mit `CUDA_DEVICE_MAX_CONNECTIONS=1` und
  `surfaces=2` in `-ffmpeg-video-opts` (FFmpeg setzt damit `async_depth` 1:
  ein Frame in Arbeit, einer im Flug). Erwartet: gsr ~66 MB weniger, Last
  ~160 MB auf ruhigem Bild.
- Vulkan, VAAPI, Aufnahmen: unverändert.
- `surfaces=1` nicht: dann kodiert FFmpeg synchron (`async_depth` 0).

### Code

1. Neu `src/platform/linux/replay/encode.rs`: `video_opts` (aus `mod.rs`
   verschoben, bei `*_nvenc` dazu `;surfaces=2`) und `fn nvenc(command,
   encoder)`, das die Umgebung setzt; Test aus `mod.rs` mit.
   `mod.rs` sinkt dadurch unter 980 Zeilen (vorher 993).
2. `mod.rs` `start_capture`: ruft beide.
3. Doku: `docs/replay.md` Bitrate-Abschnitt, dieser Plan.

### Was brechen könnte

- Bei hohen fps (Replay bis 240) staut NVENC mit nur einem Frame im Flug:
  `daemon.log` meldet dann „replay capture slowed“. P4 sah keinen Verlust,
  live ist es nur bei 60 fps prüfbar ohne Desktop-Last; bleibt als Risiko
  stehen.
- Die Variable greift nur, wenn gsr CUDA nach dem Start lädt (tut es,
  `gsr_cuda_load` lädt `libcuda` per `dlopen`).

### Abnahme

Test `video_opts`; fmt, clippy, `cargo test`; live (Daemon-Neustart):
`/dev/nvidiactl` von gsr per `smaps`, `replay_load_now`, `update fps` im
Capture-Log bei 60 fps.

**Macht nicht:** nichts an Aufnahmen-gsr, Vulkan, VAAPI; kein `surfaces=1`;
kein Schalter.

### Live 28.10.

`encode.rs` mit Test
`only_nvenc_trades_constant_bitrate_for_quality_and_a_lean_session`; fmt,
clippy `-D warnings`, `cargo test` 164 + 416 grün; `mod.rs` 972 Zeilen.
Daemon neu, beide Monitore, 60 fps; gsr läuft mit `surfaces=2` und
`CUDA_DEVICE_MAX_CONNECTIONS=1` (in `/proc/<pid>/environ`).

- gsr `/dev/nvidiactl` **54,8 MB** (vorher 120,8), wie P4 vorhersagte.
- `replay_load_now` nach 36 s ruhigem Desktop: 19 % eines Kerns, **146 MB**
  (vorher 223, vor Teil 2 336). Ring 19,7 MB, Worker 26 MB.
- `replay-capture.log`: 60–61 fps, keine neue Meldung; `daemon.log` ohne
  „replay capture slowed“.

Was bleibt, ist der Boden: CUDA-Kontext und GL in gsr (~55 MB Treiber,
~35 MB Heap und anonym), Worker mit Ring (~26 MB auf ruhigem Bild). NVENC
geht unter FFmpeg nur über CUDA; weniger bräuchte einen anderen Encoder-Weg
(gsrs Vulkan-Encoder hing hier schon bei Aufnahmen), nicht vorgesehen.

## Teil 4: Was sonst noch kostet

Stand: **gebaut 28.10., wie geplant.** `load.rs` `children`, Prozessbaum-
Test mit `task/<tid>/children` (zwei Daemon-Threads); fmt, clippy
`-D warnings`, `cargo test` 164 + 416 grün. `replay_load_now` findet gsr,
kms-Helfer und Worker wie vorher (21 % eines Kerns, 134 MB). Die
Kinder-Liste des Daemons zu lesen kostet 0,09 ms statt ~5 ms für den Scan
(Python-Messung, beides). Neu geöffnetes Fenster (Replay-Seite, DP-3 mit
240 Hz), 30 s: Mess-Thread **0,5 %** (vorher 1,5 %).

### Befund (gemessen 28.10., nur `/proc`, Skripte `probes/p5/` im Scratchpad)

CPU je Prozess, ruhiger Desktop, Replay beide Monitore 60 fps, Settings-
Fenster offen:

| Prozess | CPU (eines Kerns) | privat |
| --- | --- | --- |
| gsr | 19 % ruhig, 35–46 % bei Bewegung im Browser | 89–118 MB |
| `gsr-kms-server` | 2,4 % | |
| Worker | 0,4 % | 26–40 MB (Ring) |
| Settings-Fenster | **1,5 %** (30 s gemessen) | 24 MB |
| Daemon | 0,1 % | 4 MB |

- **gsr:** Hauptthread 12,8 % (~2 ms CPU je Frame für zwei Monitore, 485
  freiwillige Wechsel/s: wartet, dreht nicht), Audio-Thread 6,2 % (97
  Wechsel/s, zwei je AAC-Frame bei 48 kHz). `perf` darf gsr nicht ansehen
  (`yama ptrace_scope` 1, kein Kind). Die Schleife schläft bis zum nächsten
  Frame (`src/recorder/recorder.c` `recorder_sleep_until_next_frame`).
  Hebel nur über andere Formate (`-fm content`: Clips mit wechselnder
  Bildrate, `opus` statt AAC: MP4-Kompatibilität); nicht vorgesehen.
- **Cursor:** `record_cursor = "system"`, Plugin läuft nicht. Der Tracker
  (`linux/cursor_track.rs` `run`) wartet mit `poll_event(50 ms)` auf
  Events, keine Schleife; das Plugin läuft nur bei Smooth-Cursor-Aufnahmen
  (nicht messbar ohne Aufnahme).
- **Settings-Fenster, Fehler:** die ganze CPU liegt im Mess-Thread
  (`linux/app/save.rs` `poll_replay`, 46 Ticks in 30 s, 3 Wechsel/s), der
  Hauptthread zeichnet nichts nach. `load.rs` `replay_pids` liest dafür
  **jede Sekunde `/proc/*/stat` aller Prozesse** (480 hier), um die Kinder
  des Daemons zu finden; läuft, solange das Fenster offen ist, auf jeder
  Seite.

### Verhalten

- Die Kinder des Daemons kommen aus `/proc/<daemon>/task/*/children`
  (jeder Thread listet die Prozesse, die er gestartet hat; stirbt der
  startende Thread, gehen sie an einen anderen Thread derselben Gruppe),
  ihre Kinder ebenso. Gelesen werden nur noch die Dateien dieser paar
  Prozesse. Erwartet: Mess-Thread unter 0,2 %.
- Kernel ohne `CONFIG_PROC_CHILDREN`: keine Liste, die Zeile sagt
  „Running, not measured“ (ehrlich statt falsch). Gentoo, Arch, Debian,
  Fedora haben es an.
- Auswahl wie heute: `boltsnap-replay…` und der gsr, der nach
  `/dev/stdout` schreibt, plus deren Kinder.

### Code

1. `src/platform/linux/app/load.rs`: `fn children(proc, pid)`;
   `replay_pids` nutzt sie statt `processes`, das wegfällt. Test: die
   Fixture bekommt `task/<tid>/children` (zwei Threads beim Daemon, einer
   mit dem gsr, einer mit dem Worker), Erwartung wie heute.
2. Dieser Plan; `docs/replay.md` braucht nichts (Text bleibt).

### Abnahme

fmt, clippy, `cargo test`; `replay_load_now` gleiche Zahl wie vorher;
Settings-Fenster neu geöffnet, Mess-Thread über 30 s.

**Macht nicht:** kein anderer Takt, kein Messen nur auf der Replay-Seite,
nichts an gsr, Worker, Cursor.

### Nachtrag 29.10.: der Balken zuckt

Im selben Fenster lief der Hauptthread nach dem Start weiter. `perf`
(`cpu-clock`, ungestrippter Build in `target/prof`, Fenster als Kind
gestartet, 20 s nach 5 s Anlauf): jede Sekunde ~3 ms (ein Bild für die neue
Load-Zahl), dazu **alle ~6,5 s ~110 ms über 300 ms**, die Gleitbewegung des
Balkens, bei 240 Hz Bild für Bild neu gezeichnet (`tiny_skia`
`fill_path_impl`, `Pixmap::fill`, `Fonts::draw_clipped` vorn). Im Mittel
~1,8 % eines Kerns, der größte Teil davon dieser Burst.

Ursache: der volle Ring wirft ganze GOPs von einer Sekunde heraus, die
gehaltene Dauer pendelt in der letzten Sekunde (29,0–30,0 s bei 30 s,
`duration_us` im Status). `set_replay_held` (`app/preview/replay.rs`)
gleitet jedes Mal dorthin: die älteste Zelle wird dunkler und wieder hell,
„Holds 29 s“ und „Holds 30 s“ wechseln.

**Verhalten:** eine Dauer innerhalb der letzten Sekunde vor voll
(`min(length, holds)`) zeigt voll. Füllen beim Start gleitet wie bisher
Zelle um Zelle; ein Puffer, der mehr hält als die Seite sagt (alte
Einstellungen), bleibt unberührt.

**Code:** `set_replay_held`; Test in `app/tests/preview.rs`: 59,03 und
59,53 s bei 60 s zeigen voll, ohne zu gleiten.

**Gebaut 29.10.**, Test grün und ohne den Fix rot (Gegenprobe per
Stash); fmt, clippy `--all-targets -D warnings`, `cargo test` 165 + 416.
Gleiches Profil danach, 35 s: die ersten 20 s (Fenster offen, unbedient)
2–4 Samples je Sekunde, also ~0,3 % eines Kerns, kein Burst mehr. Danach
Bedienung mit der Maus: bis zu einem ganzen Kern bei 240 Hz, je Bild
Formen (`tiny_skia`) 50 %, Glyphen-Umrisse 20 %, libc-Kopien 17 %, libm
10 %. Das ist der nächste Hebel, siehe Tempo-Durchsicht.

**Macht nicht:** kein Teil-Neuzeichnen des Fensters (jedes Bild malt alles;
eigener Umbau, nicht hier).

### Tempo-Durchsicht 29.10.

Nutzer 29.10.: „lass nochmal schauen wie wir alles schneller machen
koennen“. Nur gemessen, nichts gebaut.

- **Clip-Export** (Vollbild aus beiden Monitoren, NVDEC `-crop` plus NVENC
  `-qp 18`, `worker/src/crop.rs` `encode`): live 4,4 s für 30 s (~400 fps).
  Synthetisch (`probes/p6`, 600 Frames 3840 × 1080 nach 1920 × 1080, je
  zweimal): p4 wie heute 2,4–3,0 s, `-tune ll` 2,6 s, p1 2,4–2,5 s bei
  0,6 dB weniger PSNR. Kein verlässlicher Gewinn, bleibt.
- **Shelf** (`shelf/cards/actions.rs`): malt je Bild die ganze Fläche neu
  und meldet sie ganz als Schaden; die Fläche ist nur die Kartenspalte.
  Nicht gemessen, kein Befund.
- **Selector**: seit 21.09. regional (nur geänderte Bereiche,
  `docs/screenshot-performance.md`), hier nicht neu gemessen.
- **Settings-Fenster** beim Bedienen (Profil oben): bis zu einem ganzen Kern
  bei 240 Hz; jedes Bild malt das ganze Fenster. Hebel, nach Wirkung:
  1. nur geänderte Bereiche neu malen (wie der Selector; der Vergleich mit
     dem letzten Bild in `window.rs` `changed` gibt es schon, er spart nur
     das Senden, nicht das Malen);
  2. Glyphen einmal rastern und wiederverwenden (20 %: `ttf_parser`
     liest Umrisse und Variationen jedes Bild neu);
  3. Squircle-Pfade je Größe merken (`ui::push_squircle`, `hypotf`,
     `roundf`: ein Teil der 50 % Formen und 10 % libm).
  Nutzer 29.10.: alle drei, solange nichts hässlicher wird. Plan und Stand:
  [settings paint speed](../settings/2026-10-29-settings-paint-speed.md).
