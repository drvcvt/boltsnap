# Settings-UI (Plan 2026-10-23)

Status: **Plan, nichts gebaut.** Richtung vom Nutzer am 23.10. bestätigt
(„passt an sich so“), Phasen 0 bis 3 danach ausgeplant. Freigabe am 24.10.
samt der drei offenen Entscheidungen (Abschnitt 9). Am selben Tag erweitert:
das Fenster wird das **Boltsnap-Fenster** mit Library und Settings, die
Library ist Phase 6 ([Library-Plan](2026-10-24-library.md)).
Zeilenangaben sind vom 23.10.; S1 (`21c8268`) hat `RecordingThumb` und
`load_legacy` entfernt, vor jeder Phase werden die Belege neu geprüft.
Grundsatzentscheidung des Nutzers (23.10.): eigenes Fenster, selbst gezeichnet
mit tiny-skia, „wenn es sich gut und schön umsetzen lässt und kaum
Dependencies mitbringt“. Ergebnis der Prüfung unten: **null neue Crates.**

Grundlage: Config-Review CF1 bis CF8 in
[comfort-and-config](2026-09-25-comfort-and-config.md#4-config-review-2026-10-23)
und die Bestandsaufnahme aller Settings dort. Ohne Phase 1 (Schema und ein
einziger Schreiber) baut das UI auf einer Config, die Fehler still verschluckt
und parallele Schreiber verliert.

## 1. Ziel und Nicht-Ziele

Ziel: alle heutigen Einstellungen und ein paar neue (Kartengröße, sichtbare
Karten, Shelf-Ecke) in einem Fenster einstellen, Änderungen wirken sofort, wo
das technisch geht, sonst sagt die Zeile, wann. Die Config-Datei bleibt die
Wahrheit und bleibt von Hand editierbar, mit Kommentaren.

Was es **nicht** macht:

- Keine Hotkeys. Die Binds leben in der Hyprland-Config, nicht in boltsnap.
- Kein Windows. `boltsnap settings` meldet dort klar „not available on
  Windows“ (Backend eingefroren, AGENTS.md).
- Kein Light-Theme, keine Themes, keine Sprachen (UI englisch wie der Rest).
- Keine Editor-Settings (eddy), keine gsr-Expertenparameter (`-bm`, `-keyint`,
  Qualitätsstufe), keine Recording-Interna (Quota, Stall-Sekunden).
- Kein Screenreader (kein AT-SPI). Tastaturbedienung ja, vollständig.
- Kein exaktes Fractional Scaling: ganzzahliger Buffer-Scale, bei 1,5 rendert
  das Fenster mit 2 und der Compositor skaliert. Beide Monitore laufen heute
  mit Scale 1 (`hyprctl monitors`, 23.10.).
- Kein Import/Export, keine Profile, kein „Reset all“ in v1 (pro Zeile ja).

## 2. Verhalten

### Öffnen und Fenster

- Wege: Tray-Einträge **Open Boltsnap…** (Library) und **Settings…** (ganz
  unten, über Quit, falls vorhanden), `boltsnap open` und `boltsnap settings`.
  Der Tray startet denselben Befehl als eigenen Prozess.
- Eigener Prozess mit eigenem Wayland-Fenster (`xdg_toplevel`, App-ID
  `boltsnap`), ein Fenster für Library und Settings. Ein Absturz im Settings-UI trifft weder den Daemon
  noch eine laufende Aufnahme.
- Startgröße 760×520 logisch, Minimum 560×400, frei skalierbar (Tiling).
  Server-Side-Decorations über `xdg-decoration`, wo angeboten; sonst
  SCTK-`fallback_frame`. Hyprland zeichnet keine Deko, das Fenster braucht
  keine eigene Titelzeile.
- Nur eine Instanz: `flock` auf `<runtime>/app.lock`. Eine zweite
  Instanz schickt über `<runtime>/app.sock` „raise“ mit der Zielseite und
  beendet sich;
  die erste fordert per `xdg_activation_v1` Fokus an. Ob Hyprland ohne
  Input-Serial aktiviert, ist **nicht belegt** und wird vorher isoliert
  geprobt (Phase 4, Schritt 1). Fällt die Probe durch: kein zweites
  Fenster, Notification „Settings already open“.
- Schließen: Fenster-X, `Esc` (wenn kein Dropdown offen ist), `Ctrl+W`,
  `Ctrl+Q`. Es gibt nichts zu speichern, also keine Rückfrage.

### Aufbau

Links eine schmale Navigation mit Icon und Name je Seite (oben die
Library-Seiten All, Screenshots, Videos, darunter die Gruppe Settings),
rechts die Seite. Settings-Seiten bestehen aus Zeilen. Jede Zeile: Icon, Name, optional ein kurzer Hinweis in `sub`
darunter (nur wo nötig, z. B. „Applies to the next recording“), Control
rechtsbündig. Keine Rahmen um Zeilen, Gruppen nur über Abstand und eine
Gruppenüberschrift. Genau eine Seite sichtbar, Wechsel mit gleitender
Auswahlfläche in der Navigation.

Seiten und Zeilen (Control; wann es wirkt):

| Seite | Zeile | Key | Control | Wirkt |
| --- | --- | --- | --- | --- |
| Shelf | Card size | `[shelf] card_width` (neu) | Slider 140 bis 380 px, Schritt 10, Vorschau-Zahl | sofort, Re-Render (Abschnitt 3) |
| Shelf | Cards shown | `[shelf] visible_cards` (neu) | Stepper 1 bis 8 | sofort |
| Shelf | Position | `[shelf] corner` (neu) | Segment: 4 Ecken als Icons | sofort, beim nächsten Neuaufbau des Layers |
| Screenshots | Save folder | `save_dir` | Pfadfeld + Ordner-Dialog | sofort |
| Recording | Display | `record_default_target` | Dropdown: Focused display, je Monitor, Both | nächste Aufnahme |
| Recording | Both displays | `record_both_mode` | Segment Separate/Combined | nächste Aufnahme |
| Recording | Frame rate | `record_fps` / `record_profile` | Dropdown 30, 60, 120, 144, 240, Custom (Stepper) | nächste Aufnahme |
| Recording | Codec | `record_codec` | Dropdown Auto + gefundene Encoder (`gsr --info`) | nächste Aufnahme |
| Recording | Cursor | `record_cursor` | Segment System/Mellow/Quick | nächste Aufnahme |
| Recording | Audio | `record_audio_enabled` | Schalter | nächste Aufnahme |
| Recording | Audio source | `record_audio_source` | Segment System+Mic/Mic/System | nächste Aufnahme |
| Recording | Show frame | `record_show_frame` | Schalter | sofort |
| Recording | Video folder | `record_dir` | Pfadfeld + Ordner-Dialog, leer = Save folder | sofort |
| Recording | Disk save also to shelf | `record_disk_add_to_shelf` | Schalter | sofort |
| Recording | Shelf to disk after | `record_shelf_to_disk_after_seconds` | Dropdown Off, 30 s, 1 min, 5 min, 15 min | nächste Aufnahme |
| Replay | Start with daemon | `[replay] autostart` | Schalter | nächster Daemon-Start |
| Replay | Length | `[replay] duration_seconds` | Slider 5 bis 600 s | Replay-Neustart |
| Replay | Memory | `[replay] memory_mib` | Dropdown 256 MiB bis 4 GiB | Replay-Neustart |
| Replay | Frame rate | `[replay] fps` | Dropdown 30, 60, 120, 144, 240 | Replay-Neustart |
| Replay | Bitrate | `[replay] bitrate_kbps` | Auto (berechnet, Wert angezeigt) oder Stepper | Replay-Neustart |
| Replay | Display | `[replay] output` | Dropdown wie Recording | Replay-Neustart |
| General | Font | `ui_font` | Dropdown der installierten Familien (fontconfig), „Desktop font“ zuerst | sofort |

`[replay] encoder` bleibt Datei-only (Expertenwert, Auto wählt über
`gsr --info`). Die Replay-Seite zeigt bei laufendem Replay oben eine Zeile
„Replay restarts to apply“ mit Button **Restart replay**, sobald ein Wert
geändert wurde, der erst nach Neustart wirkt. Ein Neustart verwirft den
Puffer, deshalb nie automatisch.

Längen und Bitrate hängen zusammen: die Zeile Memory zeigt live, wie viele
Sekunden bei der aktuellen Bitrate wirklich hineinpassen
(`replay::settings`-Rechnung), statt still zu kappen.

### Live anwenden

- Jede Änderung geht sofort als Patch an den Daemon (`SetSettings`, Phase 2);
  der validiert mit demselben Schema, schreibt die Datei, wendet an und
  antwortet mit dem neuen Stand. Kein Speichern-Button.
- Slider schicken beim Ziehen gedrosselt (höchstens alle 100 ms, letzter
  Wert immer beim Loslassen), Textfelder beim Verlassen oder `Enter`.
- Lehnt der Daemon ab, springt das Control auf den gültigen Wert zurück
  und die Zeile zeigt den Grund in einer Zeile.
- Jede Zeile mit Nicht-Default-Wert bekommt rechts neben dem Control ein
  kleines Zurücksetzen-Icon (entfernt den Key aus der Datei).

### Zustände und Randfälle

| Fall | Verhalten |
| --- | --- |
| Daemon läuft nicht | `boltsnap settings` startet ihn wie jeder Befehl (`ensure_daemon`). Scheitert das: Fenster zeigt oben „Daemon not running“, alle Controls sind deaktiviert, kein Schreiben an der Datei vorbei. |
| Daemon stirbt, während das Fenster offen ist | Banner wie oben, Controls aus; das Fenster verbindet sich alle 2 s neu und lädt dann den Stand frisch. |
| Config-Datei hat Syntaxfehler | Daemon arbeitet mit dem letzten gültigen Stand weiter (CF1). Fenster zeigt oben „config.toml has an error on line N“ mit Pfad. Alle Controls deaktiviert, weil ein Patch die kaputte Datei nicht sicher umschreiben kann. Sobald die Datei repariert ist (Watcher, CF6), lädt das Fenster neu. |
| Einzelner Key ungültig (`record_cursor = "Quick"`) | Die Zeile zeigt „Invalid in config.toml: …“ und den wirksamen Default. Das Control bleibt bedienbar; eine Änderung überschreibt den ungültigen Wert. Ungültige, unveränderte Keys schreibt der Daemon nie zurück (CF3). |
| Unbekannte Keys | bleiben unangetastet in der Datei; das Fenster zeigt sie nicht. `boltsnap doctor` meldet sie (V4, separat). |
| Handedit, während das Fenster offen ist | Watcher im Daemon (CF6) lädt neu, `SettingsWatch` schickt den Stand, Controls folgen ohne Flackern. Ein Control, das gerade gezogen wird, behält den Wert des Nutzers. |
| Config ist ein Symlink | Daemon schreibt in das Ziel, nicht über den Link (CF2). |
| Aufnahme läuft | Recording-Zeilen zeigen „Applies to the next recording“. Nichts an der laufenden Aufnahme ändert sich. |
| Monitor aus der Auswahl ist abgesteckt | Dropdown zeigt ihn als „DP-1 (disconnected)“, solange er konfiguriert ist; Start fällt wie heute auf den fokussierten zurück. |
| Ordner existiert nicht | Pfad wird angenommen und beim nächsten Speichern angelegt wie heute; die Zeile sagt „Will be created“. Nicht beschreibbarer Pfad: Ablehnung mit Grund. |
| Ordner-Dialog (Portal) fehlt | Button deaktiviert mit Tooltip-Hinweis, Pfadfeld bleibt. |

### Tastatur

`Tab`/`Shift+Tab` durch Navigation und Controls, `↑/↓` in Navigation und
Dropdown-Listen, `←/→` für Slider, Stepper und Segmente, `Space` schaltet,
`Enter` öffnet Dropdowns und bestätigt, `Esc` schließt das Dropdown, sonst das
Fenster. Fokus ist sichtbar über eine hellere Fläche, kein Glow-Ring.
`Ctrl+1` bis `Ctrl+5` springen zu den Seiten.

## 3. Kartengröße und Re-Rendering

### Befund (Code, 23.10.)

- Die Größe ist fest 190×132 (`src/shelf/thumbnail.rs:6-7`). Layout und
  Painter nehmen die Maße vom Bild selbst: `Layout::compute` bekommt
  `(id, thumb.width(), thumb.height())` (`src/shelf/layout.rs:45`),
  `blit_thumb_card_anim` zentriert in `img.dimensions()`
  (`src/shelf/paint.rs:37`). Eine Karte ist so groß wie ihr Thumbnail.
- Thumbnails entstehen an vier Stellen, alle mit der Konstante:
  1. CLI rendert vor und schickt `add_thumb` (`src/main.rs:580`); der Daemon
     nimmt es nur bei exakt `CARD_W×CARD_H` und dekodiert sonst die ganze PNG
     neu (`shelf/mod.rs:796`). Das ist der schnelle Screenshot-Pfad.
  2. Raw-Pixel-Pfad `prepare_pixels_with` (`shelf/mod.rs:869`).
  3. Video-Thumbnail per ffmpeg im `thumbnail::Worker`
     (`linux/shelf/thumbnail.rs:41`, `scale=…:crop=…`), ein Thread mit
     begrenzter Queue (`MAX_CACHED_CARDS`, `try_send`).
  4. Fertige Aufnahme: Platzhalter (`shelf/mod.rs:3291`, `3299`).
  (`RecordingThumb` und `load_legacy` gibt es seit S1 nicht mehr.)
- Die sichtbare Höhe hängt an der Kartenhöhe:
  `MAX_VISIBLE_CARDS` (5, `shelf/mod.rs:330`) begrenzt den Viewport in
  `animated_layout` (`shelf/mod.rs:483-490`).
- Windows nutzt dieselben Konstanten (`windows/shelf.rs:434`, `455`); das
  Backend ist eingefroren und behält sie.

### Verhalten

- Neuer Wert kommt an (Slider, Datei, Watcher): Der Daemon setzt die
  **aktuelle Kartengröße** `CardSize { w, h }`, `h = round(w × 132 / 190)`,
  damit das Seitenverhältnis bleibt. Bereich 140 bis 380 px.
- **Stufe 1, sofort, im Eventloop:** jedes Thumbnail wird aus dem
  vorhandenen kleinen Bild auf die neue Größe skaliert (Triangle, 20 Karten
  à höchstens 380×264: unter 1 ms). Layout und Animation sehen sofort die
  neue Größe, nichts springt; beim Vergrößern ist die Karte kurz weicher.
- **Stufe 2, im Hintergrund:** jede Karte wird aus ihrer Quelle scharf neu
  gerendert:
  - Bild-Karten aus `png_path` (temporär oder gespeichert) mit
    `make_image_card_thumbnail`.
  - Video-Karten per ffmpeg wie heute, mit der neuen Größe im Job.
  - Platzhalter einer laufenden Finalisierung: nur Stufe 1; das echte
    Thumbnail kommt ohnehin.
- Übernahme nur, wenn das Ergebnis exakt die **aktuelle** Größe hat. Damit
  sind veraltete Ergebnisse nach weiterem Ziehen automatisch verworfen,
  ohne Generationszähler. Der Worker liest vor jedem Job die aktuelle Größe
  aus einem `AtomicU64` und überspringt Jobs für eine alte Größe.
- Quelle fehlt (gespeicherte Datei extern gelöscht), Dekodieren scheitert:
  Karte behält das Stufe-1-Bild, kein Fehler-Toast.
- Neue Karten nach dem Wechsel entstehen direkt in der neuen Größe:
  - Die CLI liest `[shelf] card_width` aus derselben Config (schnelle
    Validierung, ein Key) und rendert in der Größe vor. Weicht sie vom Daemon
    ab (gerade geändert), greift der bestehende Fallback: der Daemon dekodiert
    die PNG selbst. Langsamer, aber korrekt.
  - Raw-Pfad, ffmpeg-Worker und der Aufnahme-Platzhalter nehmen die
    aktuelle Größe statt der Konstante.
- Drag läuft gerade: die Größe der Karte im Drag-Icon bleibt; die Karten im
  Shelf wechseln trotzdem (Drag-Icon ist ein eigener Buffer).
- Speicher: 20 Karten × 380×264×4 B = 8 MB im Maximum, heute 2 MB.

### Code

- `src/shelf/thumbnail.rs`: `CardSize { w, h }` mit `DEFAULT` (190×132),
  `from_width(px) -> Result<CardSize, String>` (Bereich, Seitenverhältnis)
  und `rescale(&RgbaImage, CardSize)`. Die Konstanten bleiben als
  `CardSize::DEFAULT.w/h` für Windows. Reine Funktionen, Tests.
- `linux/shelf/thumbnail.rs`: Job wird `(id, Source, CardSize)` mit
  `Source::Video(PathBuf) | Source::Image(PathBuf)`; `current: Arc<AtomicU64>`
  für den Skip. `ThumbnailReady` bringt die Größe mit.
- `linux/shelf/mod.rs`: `Daemon::card_size`, `set_card_size(size)` (Stufe 1
  plus Jobs), alle vier Erzeuger lesen `self.card_size`. **`mod.rs` darf
  nicht wachsen** (4031 Zeilen, Limit 1000): neue Logik in
  `linux/shelf/cards.rs`; im Gegenzug wandern die Thumbnail-Helfer
  `prepare_add`, `prepare_pixels*`, `write_temp_png` (heute
  `mod.rs:784-900`) mit dorthin. Netto wird `mod.rs` kleiner.
- `MAX_VISIBLE_CARDS` wird `self.visible_cards` (gleicher Weg).
- Ecke: `place_on_output` setzt Anker und Ränder aus `self.corner`; eine
  Änderung baut den Layer neu auf (gleicher Weg wie ein Monitorwechsel, die
  Karten bleiben im Modell). Animationen und Hit-Test sind bereits relativ
  zur Surface, nur der Stapel kehrt sich um, wenn die Ecke oben liegt
  (neueste Karte an der Ecke). `Layout::compute` bekommt dafür
  `grow_down: bool`.
- `main.rs` (CLI-Vorrendern): Größe über `config::shelf_card_size()`.

### Tests

- `CardSize::from_width`: Grenzen, Seitenverhältnis, Fehler außerhalb.
- `rescale` liefert exakt die Zielgröße, auch für 1×1 und Upscaling.
- Übernahme verwirft ein Ergebnis mit alter Größe (reine Funktion
  `accept_thumbnail(current, result)`).
- Layout mit 380er Karten und `visible_cards` 1 und 8: Viewport-Höhe,
  keine Überlappung; `grow_down` spiegelt die Reihenfolge.
- Worker-Skip: Job mit alter Größe wird übersprungen (Kanal-Test ohne
  ffmpeg, Bildquelle als kleine PNG im Temp-Verzeichnis).

## 4. Config-Fundament (Phase 1 und 2)

Behebt CF1 bis CF4 und CF6 aus der Config-Review; ohne das wäre das UI ein
weiterer Schreiber, der Werte verliert.

### Wo es liegt

Reine Logik kommt in die Lib-Crate (`src/lib.rs`, dort liegen heute schon
`protocol` und `replay`), damit sie ohne Daemon und Fenster testbar ist:
`src/settings/` (Schema, Laden, Patch-Writer). Die Binär-Crate behält
`src/config.rs` als **Fassade**: dieselben Funktionen und Typen wie heute
(`Config::load`, `load_valid`, `replay_settings`, `recording_prefs`,
`resolve_*`, `save_recording_prefs`), intern über `settings`. Grund: die
eingefrorene Windows-Shell ruft `Config` an 7 Stellen direkt auf
(`windows/recording.rs:369-370`, `windows/shelf.rs:109`, `500`, `521`,
`623-624`); sie darf sich nicht ändern.

### Phase 1: Schema, Laden, Patch-Writer

**Typen** (`src/settings/schema.rs`, nur Daten):

```rust
pub enum Kind { Bool, Int { min: i64, max: i64 }, Choice(&'static [&'static str]),
                Path, Text, Target /* focused | both | output:<name> */ }
pub enum Applies { Live, NextRecording, ReplayRestart, DaemonStart }
pub struct Key { pub path: &'static str /* "record_fps", "replay.fps" */,
                 pub kind: Kind, pub default: Value, pub applies: Applies }
pub const KEYS: &[Key] = &[ /* alle 21 heutigen + 3 neue Shelf-Keys */ ];
```

`record_profile` und `record_fps` bleiben zwei Keys wie heute (Datei bleibt
kompatibel); das UI zeigt sie als eine Zeile „Frame rate“.

**Laden** (`src/settings/load.rs`):

```rust
pub enum Loaded {
    Ok { values: Values, issues: Vec<Issue> },
    Broken { line: usize, message: String },
}
pub struct Issue { pub key: String, pub line: usize, pub message: String }
pub fn load(text: &str) -> Loaded
```

- Jeder Key im Schema: Wert gültig → `values`; falscher Typ, außerhalb des
  Bereichs, unbekannter Choice → Default in `values` **und** ein `Issue`.
- Keys in der Datei, die das Schema nicht kennt (auch Top-Level-Keys, die
  unter `[replay]` gelandet sind), → `Issue` „unknown key“, Wert bleibt in
  der Datei.
- `Values` ist eine typisierte Struktur (kein `HashMap<String, Value>`), damit
  Aufrufer keine Strings parsen.

**Patch-Writer** (`src/settings/write.rs`):

```rust
pub enum Change { Set(Value), Remove }
pub struct Patch(pub Vec<(&'static str, Change)>);
pub fn apply(text: &str, patch: &Patch) -> Result<String, String> // rein, testbar
pub fn write(path: &Path, patch: &Patch) -> io::Result<()>       // Datei
```

- `apply`: nur die Keys im Patch ändern, über `toml_edit` mit Erhalt von
  Kommentar und Position (wie K2 heute); `[replay]`/`[shelf]`-Tabellen bei
  Bedarf anlegen, ans Ende. Ein Wert wird vor dem Schreiben gegen das Schema
  geprüft; ungültig → `Err`, Datei unverändert.
- `write`: exklusiver Lock über `std::fs::File::lock` (stabil seit Rust
  1.89, hier 1.95) auf `config.toml.lock` neben dem übergebenen Pfad, nicht
  neben dem Symlink-Ziel, damit kein Lock in einem Dotfiles-Repo landet;
  `libc::flock` ginge nicht, `libc` hängt nur am Linux-Target und die
  Lib-Crate ist plattformneutral. Datei frisch lesen, `apply`, Ziel auflösen (`canonicalize`;
  kaputter Symlink → Fehler), Temp-Datei neben dem Ziel
  `.<name>.tmp-<pid>-<zähler>` mit `O_EXCL`, Modus der alten Datei, `fsync`,
  `rename`, `fsync` des Verzeichnisses, Lock lösen.
- Ein ungültiger Wert in der Datei, den der Patch nicht anfasst, bleibt
  unverändert stehen (CF3).

**Aufrufer umstellen**, je ein Commit, jeder für sich lauffähig:

1. `settings` plus Tests, `config.rs` unverändert.
2. `Config::parse`/`load` intern über `settings::load`; Verhalten bei
   `Broken` bleibt vorerst Default (damit Schritt 2 nur umbaut).
3. `save_recording_prefs` wird ein Patch der 7 Keys über `settings::write`
   (behebt CF2, CF4-Temp-Name und das Mitschreiben nie gesetzter Keys, CF3).
4. CF1: der Daemon hält den letzten gültigen Stand (`Daemon::settings`) und
   liest beim Aufnahme-Start, beim Disk-Save, in `watch.rs:142`,
   `recover.rs:99` und für die Popup-Schrift diesen Stand statt
   `Config::load()` (Aufrufer: `shelf/mod.rs:1202-1203`, `1993`, `2003-2005`,
   `2400-2403`, `2428`, `2441`, `2482-2485`, `2517`, `2582`, `2908-2909`).
   Ist die Datei `Broken`: eine Notification mit Zeile, Arbeit mit dem alten
   Stand. Replay (`replay/mod.rs:279`, `401-402`, `521`, `688`, `799`)
   bekommt die Settings vom Daemon übergeben; der Watchdog meldet einen
   Ladefehler einmal statt `continue`.
5. Selector (`main.rs:354`, `370`): liest über `load_valid`; das Speichern der
   zwei Selector-Prefs (`show_frame`, `audio_enabled`) ist ein Patch genau
   dieser zwei Keys und bricht die Aufnahme nicht mehr ab, wenn es scheitert
   (Meldung statt Abbruch).

**Tests:** Typfehler, Bereich, unbekannter Choice, unbekannter Key, Key in
falscher Tabelle, Syntaxfehler mit Zeile; `apply` erhält Kommentare und
Reihenfolge, legt Tabellen an, lässt fremde ungültige Werte stehen, lehnt
ungültige Werte ab; `write` folgt einem Symlink, behält den Modus, zwei
Threads schreiben verschiedene Keys gleichzeitig und beide Änderungen
landen; Fassade: `Config::load` liefert für jede bisherige Testdatei dasselbe
wie vorher (die bestehenden `config.rs`-Tests laufen unverändert).

#### Phase 1 im Detail (Gegenprüfung am Code, 24.10.)

**Heutiges Verhalten je Key** (`config.rs:94-300`, `parse_replay_settings`
`:136-187`). Es ist nicht einheitlich, und das bleibt in Phase 1 so:

| Keys | ungültiger Wert heute | Phase 1 |
| --- | --- | --- |
| `record_profile`, `record_fps` | `record_profile()` gibt `Err`, die Aufnahme startet nicht (`shelf/mod.rs:2392`, `2480`) | **strikt** wie heute: `Issue`, und `Config::record_profile()` gibt weiter denselben `Err`. Ein stiller Fallback auf den Default (240 fps) nähme mit einer anderen Bildrate auf als eingestellt; ein klarer Fehler ist besser. |
| alle `[replay]`-Keys, `replay` kein Table | `replay_settings()` gibt `Err`, Replay startet nicht | **strikt** wie heute, gleiche Meldungen |
| `record_default_target`, `record_both_mode`, `record_cursor`, `record_audio_source`, die drei Bools | still Default (`recording_prefs`) | Default **und** `Issue` |
| `save_dir`, `record_dir`, `record_codec`, `ui_font` (kein String) | still `None`, also Default | `None` **und** `Issue` |
| `record_shelf_to_disk_after_seconds` (keine Zahl) | still 60 s; `<= 0` heißt aus | gleich, plus `Issue` bei Nicht-Zahl |
| Syntaxfehler | `parse`: alles Default plus `eprintln`; `load_valid`: `None`; `replay_settings`: `Err("invalid config: …")` | `Loaded::Broken { line, message }`; die Fassaden verhalten sich wie heute (CF1 erst in Schritt 4) |
| unbekannter Key | ignoriert | ignoriert, plus `Issue` „unknown key“ (nur für die Anzeige, nie strikt) |

Ein strikter Key ist also nur für seinen Verbraucher strikt; `Issue`s sind
reine Information. Damit ändert Phase 1 bis Schritt 3 kein Verhalten, sie
macht Fehler nur sichtbar.

**Bausteine und Belege:**

- `toml::de::Error::span()` liefert den Byte-Bereich eines Syntaxfehlers
  (`toml-0.8.2/src/de.rs:70`), daraus die Zeile. `toml_edit` 0.20.2 hat
  keine öffentlichen Spans (`despan` ist `pub(crate)`), also findet ein
  Zeilen-Scan die Zeile eines Keys: aktuelle Tabelle aus `[name]`-Köpfen,
  Key am Zeilenanfang vor `=`. Gepunktete Keys und Inline-Tabellen gibt die
  Config nicht her; dort ist die Zeile `None`.
- `toml_edit::Document` (0.20.2) wie im heutigen Writer
  (`save_recording_prefs_at`, `config.rs:302-370`), Decor-Übernahme wie dort.

**Typen** (Lib-Crate, `src/settings/`):

- Die Enums (`RecordDefaultTarget` usw.) bleiben in `config.rs`; `Config`
  bleibt die typisierte Struktur und wandelt geprüfte Rohwerte wie heute um.
  Die UI braucht später nur die Choice-Listen aus dem Schema. (Ein Umzug in
  die Lib-Crate war erwogen und ist unnötig.)
- `schema.rs`: `Key { path, kind, applies, strict }` und `KEYS` mit den 21
  Keys (14 oben, 7 unter `[replay]`). `Kind`: `Bool`, `Int { min, max }`,
  `Choice(&[&str])`, `Text { empty_ok }`, `Path`, `Target`
  (`focused | both | output:<name>`). Die drei Shelf-Keys kommen erst in
  Phase 3 dazu.
- `load.rs`: `load(text) -> Loaded`, `Loaded::Ok { doc: Doc, issues }` oder
  `Broken { line, message }`. `Doc` hält je Key den geprüften Rohwert
  (`Value::Bool | Int | Text`) oder nichts, kein Default darin; Defaults und
  die Umwandlung in Enums bleiben in den Fassaden. `Issue { key, line: Option<usize>, message }` mit den heutigen
  Meldungen für strikte Keys.
- `write.rs`: `apply(text, patch) -> Result<String, String>` und
  `write(path, patch) -> io::Result<()>` wie oben beschrieben.

**Schritte** (je ein Commit, `cargo fmt --check`, `cargo clippy --all-targets`,
`cargo test`):

1. `src/settings/` mit `schema.rs`, `load.rs`, `write.rs` und Tests.
   `config.rs` parst noch selbst. Kein Verhalten ändert sich.
2. `Config::parse`, `load`, `load_valid` und `parse_replay_settings` lesen
   über `settings::load`. Abnahme: alle bestehenden `config.rs`-Tests
   unverändert grün, dazu ein Test, der für eine Liste kaputter und
   gültiger Dateien alte und neue Ergebnisse vergleicht (die alte
   Implementierung bleibt dafür bis zum Ende des Schritts im Test).
3. `save_recording_prefs` schreibt per Patch nur die Keys, deren Wert sich
   gegenüber dem wirksamen Wert der Datei ändert; ein ungültiger Wert, den
   der Aufruf nicht ändert, bleibt stehen (CF3). Behebt CF2 (Symlink, Modus,
   `fsync` des Ordners) und den festen Temp-Namen (CF4). Die bestehenden
   Writer-Tests laufen unverändert.
4. CF1 im Daemon und 5. Selector wie oben.

**Schritt 4 im Detail** (24.10., Gegenprüfung am Code). Abweichung vom
Entwurf oben: Den letzten gültigen Stand hält nicht `Daemon::settings`,
sondern die Fassade selbst. Der Entwurf hätte die Settings an Replay
(`replay/mod.rs:344`, `457`, `625`, `742`), den Watchdog
(`watchdog.rs:91`), `watch.rs:142` und `recover.rs:99` durchreichen müssen,
die alle in eigenen Threads lesen, und `shelf/mod.rs` (über 4000 Zeilen) wäre
gewachsen. In der Fassade wirkt der Fix für jeden Leser, auch Windows, ohne
einen Aufrufer anzufassen.

- `config.rs`: ein prozessweiter `Mutex` merkt sich den Text der letzten
  `config.toml`, die sich parsen ließ, und den zuletzt gemeldeten kaputten
  Text. Die Logik sitzt in einer kleinen Struktur mit reinen Methoden, der
  `static` hüllt sie nur ein, damit Tests nicht über globalen Zustand laufen.
- Verhalten je Datei:

  | Datei | `load()` | `load_valid()` | `replay_settings()` |
  | --- | --- | --- | --- |
  | fehlt | Default (wie heute) | `None` (wie heute) | Default (wie heute) |
  | Lesefehler | Default (wie heute) | `None` (wie heute) | `Err` (wie heute) |
  | gültig | gelesen, gemerkt | gelesen, gemerkt | gelesen, gemerkt |
  | kaputt, ein gültiger Stand bekannt | **letzter gültiger Stand** | `None` (der Aufrufer behält seinen Stand) | **letzter gültiger Stand** |
  | kaputt, keiner bekannt | Default | `None` | `Err("invalid config: …")` (wie heute) |

  Kaputt heißt immer: einmal melden je kaputtem Text, nach einer gültigen
  Fassung wieder neu. Die Meldung geht an `eprintln` und an einen Hook
  (`config::on_broken(fn(&str))`), den nur der Linux-Daemon setzt, mit
  `notify`: „config.toml line N is invalid: <Grund>; keeping the last valid
  settings“ bzw. „…; using the defaults“. Ohne Hook (CLI, Windows) bleibt
  es beim `eprintln`.
- `settings::Loaded::Broken` bekommt `reason`, die erste Zeile von
  `toml::de::Error::message()` (`toml-0.8.2/src/de.rs:64`), ohne den
  Quelltext-Ausschnitt der `Display`-Form. `message` bleibt die volle Form,
  `replay_settings` meldet weiter wörtlich `invalid config: <message>`.
- Hook setzen: `run_daemon` (`shelf/mod.rs:1189-1190`) vor dem ersten
  `Config::load()`; die beiden Zeilen werden zu zwei Zeilen, die Datei
  wächst nicht.
- Watchdog (`watchdog.rs:91`): ein `Err` von `replay_settings` läuft über
  denselben Fehlerweg wie ein gescheiterter Neustart (`eprintln`,
  `Backoff::failed`, eine Notification je Serie) statt still `continue`.

Was Schritt 4 nicht tut: keinen Datei-Watcher (Phase 2, CF6); eine CLI teilt
den Speicher des Daemons nicht (der Selector ist Schritt 5); einzelne
ungültige Werte behandeln die Fassaden wie bisher; einen kaputten Stand beim
Daemon-Start kann nichts retten außer Defaults und der Meldung.

Abnahme: Tests der Merk-Struktur (gültig, kaputt mit und ohne bekannten
Stand, gleiche Kaputt-Fassung nur einmal gemeldet, nach Reparatur wieder),
`Broken.reason` im Settings-Test, alle bisherigen Tests grün. Live: Daemon
neu starten mit gültiger Config, Replay startet wie vorher; der kaputte Weg
braucht eine Aktion im Daemon (Tray, Aufnahme, Speichern) und wird nur mit
Freigabe live geprüft, weil er die echte `config.toml` kurz kaputt macht.

**Aufrufer** (Stand 24.10., die Zeilen oben vom 23.10. sind verschoben):
`main.rs:354`, `370`, `425`, `430`; `linux/shelf/mod.rs:266`, `1189`,
`1516`, `1984`, `1994-1996`, `2386-2392`, `2413`, `2468-2480`, `2502`,
`2567`, `2893-2894`; `linux/shelf/watch.rs:142`; `record/recover.rs:99`;
`linux/select_skia/mod.rs:53`; `linux/replay/mod.rs:344`, `457`, `625`,
`742`; `linux/replay/watchdog.rs:91`; Windows (eingefroren):
`windows/recording.rs:369-370`, `windows/shelf.rs:109`, `500`, `521`, `623`.

### Phase 2: der Daemon als einziger Schreiber

**Protokoll** (`src/protocol.rs`, JSON wie die übrigen Requests):

| Request | Antwort |
| --- | --- |
| `{"cmd":"get_settings"}` | `{"ok":true,"settings":{…},"issues":[{key,line,message}],"broken":null}` |
| `{"cmd":"set_settings","patch":[["shelf.card_width",300],["record_dir",null]]}` | gleicher Stand wie oben nach dem Schreiben, oder `{"ok":false,"error":…,"key":…}` |
| `{"cmd":"settings_watch"}` | Stream: eine Zeile pro Änderung mit dem vollen Stand (wie `recording_watch`) |

`null` im Patch heißt Key entfernen (Default). Der Daemon prüft jeden Patch
gegen das Schema, bevor er schreibt; ein ungültiger Key lehnt den ganzen
Patch ab.

**Daemon:**

- `Daemon::settings` (der letzte gültige Stand) und `apply_live(&old, &new)`:
  für jeden geänderten `Live`-Key die Wirkung. `save_dir`/`record_dir` werden
  ohnehin beim Speichern gelesen, `record_show_frame` über
  `apply_show_frame_pref`, `ui_font` leert den Popup-Font-Cache, die
  Shelf-Keys gehen an Phase 3. `NextRecording`/`ReplayRestart`-Keys brauchen
  nichts, sie werden beim nächsten Start gelesen.
- Tray: `persist_tray_prefs` (heute über `PrefsWriter`-Thread, CF4) schreibt
  über denselben Patch-Pfad; der Tray-Snapshot kommt aus `Daemon::settings`.
  CF5: Radio-Eintrag „Focused display“.
- **Watcher:** `inotify_init1(IN_NONBLOCK | IN_CLOEXEC)` auf das
  Config-Verzeichnis (`IN_CLOSE_WRITE | IN_MOVED_TO | IN_DELETE`, nur Name
  `config.toml`), als calloop-`Generic`-Quelle. Entprellt 150 ms, dann
  neu laden. Eigene Schreibvorgänge: der Daemon merkt sich den Text, den er
  zuletzt geschrieben hat, und ignoriert ein Ereignis mit identischem Inhalt.
  Ist das Verzeichnis nicht da, legt der Daemon es an.
- CLI ohne Daemon (`ensure_daemon` scheitert): schreibt selbst über
  `settings::write`, gleiche Prüfung, gleicher Lock.
- CF7: `config_dir`/`cache_dir` ignorieren leere und relative XDG-Werte.

**Tests:** Protokoll-Roundtrips; `apply_live` meldet genau die geänderten
Keys; Watcher-Entscheidung „eigener Write“ als reine Funktion; Tray-Modell
mit „Focused display“.

**Live** (mit Zustimmung, Daemon-Neustart): Handedit von
`record_show_frame` ändert den Tray-Haken ohne Aktion; ein Syntaxfehler gibt
genau eine Notification, die nächste Aufnahme läuft weiter mit
`record_fps = 120`; nach dem Reparieren ist alles wie vorher.

CF8 (Daemon-Umgebung, `--save-dir`) ist nicht Teil dieses Plans.

#### Phase 2 im Detail (Gegenprüfung am Code, 25.10.)

Befund am Code (Stand `2c70573`):

- Der Tray schreibt über `PrefsWriter` (`shelf/mod.rs:251-310`): ein Thread,
  ein Briefkasten mit nur dem neuesten Wert (`tray::LatestValue`),
  Generationen und `prefs_completion` für Rollback. Geschrieben wird
  `save_recording_prefs`, also alle 7 Prefs, deren Wert vom Wert in der Datei
  abweicht. Ist der Snapshot des Daemons älter als ein Handedit, setzt ein
  Tray-Klick den Handedit zurück (Rest von CF4).
- `refresh_recording_prefs` (`mod.rs:1512`) liest über `load_valid` und
  wartet, bis kein eigener Write mehr unterwegs ist
  (`settled_prefs_generation`). Aufrufer: Tray-Aktion, `StartRecording` und
  die drei Aufnahme-Starts (`mod.rs:2385`, `2426`, `2467`).
- `ui_font` wirkt schon live: das Popup lädt die Schrift neu, wenn die Familie
  von `popup_font_family` abweicht (`mod.rs:2568`). `save_dir`, `record_dir`
  und alle `NextRecording`-Keys werden beim Gebrauch gelesen. Der einzige
  `Live`-Key mit Wirkung ist `record_show_frame` (`apply_show_frame_pref`).
- Watch-Streams: `recording_watch` schreibt JSON-Zeilen nicht blockierend in
  `Daemon::watchers`, höchstens 16, `watcher_connected` räumt auf
  (`mod.rs:331-350`, `1745-1759`, `3343-3356`).
- `handle_request` der Windows-Shell (`windows/shelf.rs:477-553`) matcht
  `Request` vollständig; neue Varianten brauchen dort einen Arm.
- `config.rs` hat 940 Zeilen, davon etwa 420 Tests; `protocol.rs` 710;
  `shelf/mod.rs` 4016 (darf nicht wachsen).
- `paths.rs:92-113`: `XDG_CACHE_HOME`/`XDG_CONFIG_HOME` werden ungeprüft
  übernommen (CF7).
- Tray `menu_model` (`tray.rs:165-205`): kein Eintrag für `Focused`, ein
  abgesteckter `output:X` zeigt den fokussierten Monitor als gewählt (CF5).

Abweichungen vom Entwurf oben, mit Grund:

- **Kein `Daemon::settings`, kein `apply_live`.** Den letzten gültigen Stand
  hält seit Phase 1 die Fassade (`Remembered`). `apply_live` hätte genau
  einen Fall (`record_show_frame`), den `refresh_recording_prefs` schon
  abdeckt; Shelf-Keys kommen in Phase 3 dazu, dort wird es nötig.
- **Watcher als eigener Thread** mit blockierendem `read` auf dem
  inotify-fd und `poll` für die 150 ms Entprellung, statt calloop-`Generic`
  plus Timer. Er schickt nur `DaemonEvent::ConfigChanged`, geladen wird auf
  dem Event-Loop. Weniger Code, kein Timer-Zustand im Daemon.
- **Eigene Writes nicht erkennen, sondern unschädlich machen:** Ein Reload
  nach dem eigenen Write liest denselben Stand, ändert nichts und sendet
  nichts (Vergleich mit dem zuletzt gesendeten Stand). Kein gemerkter Text.
- **Selector schreibt weiter selbst** (Schritt 5 von Phase 1: nur die
  geänderten zwei Keys, unter demselben Lock). Der Watcher sieht diesen
  Write wie jeden anderen. „Einziger Schreiber“ heißt damit: jeder Schreiber
  schreibt einzelne Keys unter einem Lock, der Daemon sieht jede Änderung.
  Damit entfällt auch der Fall „CLI ohne Daemon“.
- **Fehlerantwort ohne `key`:** Das UI schickt eine Zeile pro Patch und weiß,
  welche es war; die Meldung nennt den Key ohnehin.
- **Stand enthält nur gesetzte gültige Werte**, keine Defaults; die
  Defaults kommen mit dem Seitenmodell in Phase 5 ins Schema.

Schritte, je ein Commit:

1. **Aufräumen und CF7.** Die Tests aus `config.rs` ziehen nach
   `src/config/tests.rs` um (`#[cfg(test)] mod tests;`, nur Verschieben).
   `paths.rs`: `xdg_base(value) -> Option<PathBuf>` nimmt nur absolute
   Pfade; `config_dir` und `cache_dir` nutzen es. Test an der reinen
   Funktion (leer, relativ, absolut), ohne `set_var`.
2. **Ein geordneter Schreiber, Tray schreibt einzelne Keys (CF4).** Neue
   Datei `shelf/settings.rs` (shelf hat dann 10 Dateien): `Writer` mit
   Thread und `mpsc`-Warteschlange (geordnet, nicht verdichtet), jeder Job
   ist eine Liste `(key, Change)`, geschrieben mit `settings::write`; das
   Ergebnis kommt als `DaemonEvent::SettingsWritten { error }`. Der Daemon
   zählt offene Jobs (`pending_writes`) statt Generationen.
   - `persist_tray_prefs(prefs)`: Änderungen sind
     `config::pref_changes(&alt, &neu)` (die Keys, deren Wert sich ändert,
     aus `pref_values`), angezeigt wird sofort, geschrieben wird nur das.
   - `refresh_recording_prefs`: übersprungen, solange Jobs offen sind; liest
     über `Config::load()` (letzter gültiger Stand bei kaputter Datei,
     Defaults bei fehlender, wie beim Start). `load_valid` hat danach keinen
     Aufrufer mehr und fällt weg.
   - Abschluss eines Jobs: Fehler ohne Client → Notification „Recording
     settings could not be saved: …“; sind keine Jobs mehr offen,
     `refresh_recording_prefs`, das stellt bei einem Fehler den Stand der
     Datei wieder her (ersetzt `prefs_completion`/Rollback).
   - `StartRecording` setzt `show_frame`/`audio_enabled` wie bisher über
     `persist_tray_prefs`, ohne die `persisted_*`-Felder.
   - Raus aus `mod.rs`: `PrefsWrite`, `PrefsWriter`, `PrefsCompletion`,
     `prefs_completion`, die vier Generationsfelder,
     `persisted_recording_prefs`, der `PrefsPersisted`-Arm; in
     `focus_tests.rs` der Generationstest (der `LatestValue`-Test bleibt mit
     Zahlen statt `PrefsWrite`). `save_recording_prefs(_at)` braucht nur
     noch Windows und die Tests: `#[cfg(any(windows, test))]`.
   - Tests: `pref_changes` liefert genau den geänderten Key; bestehende
     Prefs-Tests bleiben.
3. **IPC `get_settings`, `set_settings`, `settings_watch`.**
   - Lib: `src/settings/state.rs` mit
     `State { path, doc, issues, broken: Option<Broken { line, reason }> }`,
     `State::read(path, text, good)` (kaputt: Werte und Befunde aus dem
     letzten gültigen Text) und JSON hin und zurück:
     `{"path":…,"values":{"replay.fps":60,…},"issues":[{"key","line","message"}],"broken":null|{"line":2,"reason":"…"}}`.
     Patch-JSON `[["record_fps",120],["record_dir",null]]` ↔
     `Vec<(String, Change)>`; nur Form prüfen, Inhalt prüft `apply`.
     `Doc` bekommt dafür ein `pub(super)`-Feld.
   - `protocol.rs`: `Request::GetSettings`, `SetSettings { patch }`,
     `SettingsWatch`; `Response` bekommt `settings: Option<State>` und
     `Response::ok_settings`.
   - `config::settings_state()`: Datei lesen, `remember`, `State::read`.
   - Daemon (`shelf/settings.rs`): `get_settings` antwortet mit
     `settings_state()`. `set_settings` geht als Job mit dem Client-Stream an
     den Writer (höchstens 32 offene Jobs, sonst „busy“); danach Antwort mit
     dem frisch gelesenen Stand oder dem Fehler aus `apply`.
     `settings_watch` schreibt den Stand als erste Zeile und hängt den Stream
     an `settings_watchers` (höchstens 16, gleiche Regeln wie
     `recording_watch`). `publish_settings()` liest den Stand, vergleicht mit
     dem zuletzt gesendeten und schreibt nur bei Änderung eine Zeile an alle;
     aufgerufen nach jedem Job.
   - Windows-Shell: ein Arm für die drei Requests mit
     „settings are not available on Windows“.
   - Tests: JSON-Roundtrip von `State` und Patch, `State::read` für kaputt
     mit und ohne letzten gültigen Text, Protokoll-Roundtrip der drei
     Requests und einer `Response` mit Stand, Patch mit Kommazahl oder
     falscher Form wird abgelehnt.
4. **Watcher (CF6).** In `shelf/settings.rs`:
   `watch_config(path, changed)` startet einen Thread:
   `inotify_init1(IN_CLOEXEC)`, Watch auf das Config-Verzeichnis
   (`IN_CLOSE_WRITE | IN_MOVED_TO | IN_MOVED_FROM | IN_DELETE`), Name muss
   `config.toml` sein. Ist `config.toml` ein Symlink, zusätzlich ein Watch
   auf den Ordner des Ziels mit dessen Namen (nach jedem Ereignis neu
   aufgelöst, so wird ein umgebogener Link auch gesehen). Nach einem
   passenden Ereignis 150 ms lang weitere Ereignisse schlucken (`poll`),
   dann `changed()`. Fehlt das Verzeichnis, wird es angelegt. Scheitert
   inotify, eine Zeile im Log, der Daemon läuft ohne Watcher weiter (die
   bisherigen Refresh-Punkte bleiben).
   - Daemon: `DaemonEvent::ConfigChanged` → `refresh_recording_prefs` und
     `publish_settings`. Eine kaputte Datei meldet `Config::load` dabei einmal
     über den Hook aus Phase 1.
   - Tests: Parser der inotify-Ereignisse an einem gebauten Puffer; echter
     Watcher an einem Temp-Ordner: Schreiben von `config.toml` ruft
     `changed` innerhalb von 2 s, ein anderer Dateiname und die
     `.lock`-Datei nicht (300 ms Wartezeit).
5. **CF5, Tray.** `default_targets(snapshot)`: `Focused display`, die
   Monitore, ein abgesteckter `output:X` als „X (disconnected)“, `Both
   displays`; Beschriftung, Auswahl und der Klick nutzen dieselbe Liste.
   Tests: Fokus gewählt, abgesteckter Monitor gewählt, Klick-Index ergibt
   das richtige Ziel.

Was Phase 2 nicht macht: kein CLI-Befehl `boltsnap settings get/set` (das
Fenster in Phase 5 ist der Client), keine Defaults im Stand, keine
Shelf-Keys, kein `doctor`, keine Pfadprüfung für `save_dir`/`record_dir`
beim Setzen (kommt mit der Zeile in Phase 5).

Was kaputtgehen kann: ein Tray-Klick während eines offenen Jobs zeigt kurz
den eigenen Wert und springt nicht zurück (gewollt, Refresh wartet);
ein Watcher-Thread, der hängen bleibt, blockiert nichts außer sich selbst;
der Windows-Arm ist nicht kompiliert (kein Target).

Abnahme: je Schritt `cargo fmt --check`, `cargo clippy --all-targets` ohne
Warnung, `cargo test`. Live nach Schritt 5 (installieren, Daemon neu): mit
Sicherung der echten `config.toml` `record_show_frame` von Hand umschalten,
`settings_watch` über den Socket zeigt die neue Zeile und der Tray-Haken
folgt (über `busctl` das Menü lesen); ein Syntaxfehler gibt genau eine
Notification, `settings_watch` zeigt `broken` mit Zeile; Datei zurück, alles
wie vorher.

## 5. Settings-Fenster und Widgets (Phase 4)

### Technik, ohne neue Crates

| Bedarf | Lösung | Beleg |
| --- | --- | --- |
| Fenster | SCTK `shell::xdg::window` | `smithay-client-toolkit-0.19.2/src/shell/xdg/window` |
| Deko-Fallback | SCTK `fallback_frame` | `src/shell/xdg/fallback_frame.rs` |
| Fokus anfordern | SCTK `activation` | `src/activation.rs` |
| Tastatur, Text | SCTK `xkbcommon` (Default-Feature), `KeyEvent.utf8` | `src/seat/keyboard/mod.rs:257` |
| Zeichnen | tiny-skia, ab_glyph (schon da) | `Cargo.toml` |
| Schrift | `font::load_ui_font_weights` (500/600) wie die Controls | `linux/shelf/font.rs` |
| Einfügen in Textfelder | `wl-clipboard-rs` (schon da) | `Cargo.toml` |
| Ordner-Dialog | xdg-desktop-portal `FileChooser` über `zbus` (schon da) | `linux/capture/portal.rs` nutzt zbus |
| Scale | `CompositorHandler::scale_factor_changed`, `set_buffer_scale` | `src/compositor.rs:26` |
| Icons | Lucide-SVGs (ISC) als Assets, eigener kleiner Pfad-Parser (M L H V C S Q T A Z, Bogen zu Kubik) auf tiny-skia `PathBuilder` | statt `resvg`/`usvg` (etwa 10 Crates) |

### Gestaltung

Richtung ist die der Recording-Controls (V9, vom Nutzer abgenommen):
mt-ui-style, dunkel, opak. Das Fenster ist dieselbe Familie, kein neuer Look.

- Flächen: Fenster `#121212`, Navigation-Auswahl und Controls `#222222`,
  Hover einen Schritt heller (`#2A2A2A`), Text `#ECECEC`, Neben-Text
  `#969696`, gedimmt `#6A6A6A`. Eine Akzentfläche nur für den Schalter
  „an“ und den Slider-Wert: invertiert (`#ECECEC` auf `#121212`), wie der
  Hauptbutton der Controls. Keine Farbe sonst; Fehlertext `#E5484D`.
- Schriftskala (Outfit über `ui_font`, sonst Desktop-Font): `title` 15/600,
  `label` 13/500, `hint` 12/500, `key` 11/500. Keine weiteren Größen.
- Abstandsskala: 4, 8, 12, 16, 24. Fensterrand 16, Zeilen-Innenabstand 12,
  Gruppenabstand 24.
- Controls 30 px hoch (≈ 2× 13 px), Radius 9; Zeilen 44 px (30 + 2×7).
  Icons 13 px neben `label`, 16 px in der Navigation; Strich 1,5.
- Keine Rahmen um Zeilen oder Karten, keine Trennlinien; Gruppen über
  Überschrift (`label` 600, `sub`) und 24 px Abstand.
- Motion: Auswahlfläche in der Navigation gleitet (Minimum-Jerk, 180 ms),
  Schalterknopf gleitet (140 ms), Dropdown blendet 90 ms ein. Aus absoluter
  Zeit (`Instant`), Frame-Callbacks nur, solange etwas läuft.
  Mausrad-Scroll interpoliert mit Retarget, Touchpad pixelgenau.

Widget-Satz, jedes mit Hover, Fokus, deaktiviert:
Schalter, Segment (2 bis 4 Teile, Icon oder Text), Dropdown (Liste im
eigenen Fenster gezeichnet, nicht als `xdg_popup`; Listen sind kurz, bei
Platzmangel öffnet sie nach oben), Slider mit Wert, Stepper (− Wert +),
Textfeld einzeilig (Cursor, Auswahl, Pfeile, Home/End, `Ctrl+A/C/V/X`,
Backspace/Delete wortweise mit `Ctrl`), Icon-Button, Banner.

### Code und Ordner

Ordnerlimits: `src/platform/linux/` hat schon 13 Dateien (Limit 12),
`docs/plans/` 18. Neues kommt nur in neue Unterordner; die Aufteilung der
beiden bestehenden Ordner ist ein eigener Schritt (unten, Phase 0).

Alles Plattformneutrale liegt in der Lib-Crate (`src/lib.rs`), die
Plattform-Teile in der Binär-Crate:

- `src/settings/` (neu, Lib): `mod.rs`, `schema.rs`, `load.rs`, `write.rs`,
  `tests.rs`.
- `src/ui/` (neu, Lib, nur Pixel): `mod.rs`, `theme.rs`
  (Skalen, Farben), `text.rs` (Messen, Zeichnen, Kürzen mit …),
  `icons.rs` (Pfad-Parser, Icon-Registry), `widgets/` (je Widget eine Datei,
  höchstens 8), `layout.rs` (Zeilen, Navigation, Scroll). Gezeichnet wird in
  einen `tiny_skia::Pixmap`; Eingaben als eigene Event-Typen, keine
  Wayland-Typen (AGENTS.md: geteilte Typen ohne Wayland).
- `src/app/` (neu, Lib): Navigation über Library und Settings,
  `settings/` (Seitenmodell aus dem Schema, Zustand, Reaktionen auf Events,
  Patch-Erzeugung). Testbar ohne Fenster. Die Library-Logik liegt in
  `src/library/` (Library-Plan, Abschnitt 4).
- `src/platform/linux/app/` (neu): `window.rs` (SCTK-Fenster, Buffer,
  Scale, Frame-Callbacks), `input.rs` (Pointer, Tastatur, Wheel →
  UI-Events), `portal.rs` (FileChooser), `single.rs` (Lock, Raise),
  `mod.rs`; Phase 6 ergänzt `library/`.
- `src/platform/windows/`: `settings` gibt einen klaren Fehler zurück.
- `assets/icons/ui/`: die genutzten Lucide-SVGs, Lizenzdatei.

### Offscreen-Render für Prüfung und Tests

`boltsnap settings --render <page> <out.png> [--state <json>]` zeichnet eine
Seite ohne Fenster in eine PNG (wie das bestehende `debug_render` des
Shelf). Damit gibt es pro Phase einen echten Screenshot und Tests messen
Geometrie (gleicher Rand, gleiche Höhen, Icon ≤ Text, keine Überlappung)
statt nach Augenmaß.

## 6. Reihenfolge

Jede Phase ist ein eigener Commit (oder wenige), baut, testet und lässt den
Rest unverändert lauffähig.

Vorher laufen die Review-Fixes in der vom Nutzer am 23.10. bestätigten
Reihenfolge: R1 (Recording-Review, Abschnitt 5), S6/S5 und S1 (Shelf-Plan),
RP2/RP3 (Recording-Review, Abschnitt 5). Sie berühren `config.rs` nicht, also
kollidieren sie nicht mit Phase 1.

0. **Ordner aufräumen** (nur Verschieben, ein Commit je Ordner, `git mv`
   damit die History folgt):
   - `docs/plans/` (18 Dateien) nach Thema:
     `recording/` (recording-control, recording-hardening-240fps,
     recording-audio-popup-font, recording-cache-quota, live-cursor-plugin),
     `replay/` (replay-implementation, replay-freeze-repro.py),
     `shelf/` (shelf-hardening, output-replace-ipc, tray-icon),
     `platform/` (x11-screenshot-support, decouple-eddy),
     `quality/` (performance-functionality, performance-save-and-screenshot,
     review-fixes, deep-review-repros.patch, recording-reliability-review),
     `settings/` (comfort-and-config, settings-ui, library). Größter Ordner danach: 5.
     Auf 10 der 18 Dateien verweist irgendeine andere Datei (`git grep`
     nach Dateinamen, 23.10.); ein Skript schreibt jeden Markdown-Link und jeden
     Pfad im Fließtext relativ zum neuen Ort um, `git grep` danach muss für
     jeden alten Pfad leer sein, und ein Link-Check (jeder relative Link
     zeigt auf eine existierende Datei) läuft über alle `.md`.
   - `src/platform/linux/` (13 Dateien): `capture.rs`, `portal.rs`,
     `image_transfer.rs` nach `capture/` (`capture/mod.rs`,
     `capture/portal.rs`, `capture/image_transfer.rs`); danach 10 Dateien.
     `linux/mod.rs` exportiert `portal` und `image_transfer` per `pub use`
     wieder, damit alle Aufrufer (`main.rs`, `shelf/mod.rs`, `shelf/tests.rs`)
     unverändert bleiben. In den drei Dateien werden die etwa 12
     `super::`-Pfade (`timing`, `ipc`, `select_skia`, `hypr`, `portal`)
     angepasst. Keine Logik ändert sich; Abnahme ist `cargo test` und
     `cargo clippy` ohne neue Warnung.
1. **Schema, Laden mit Befunden, Patch-Writer** (`src/settings/`), Fassaden
   für die alten Aufrufer, Tests. Danach Aufrufer umstellen, CF1 im Daemon.
2. **Daemon als einziger Schreiber:** IPC `GetSettings`, `SetSettings`,
   `SettingsWatch`, Watcher, Tray und CLI-Prefs darüber, CF5, CF7.
3. **Shelf-Keys:** `[shelf] card_width`, `visible_cards`, `corner`,
   Re-Rendering (Abschnitt 3), `linux/shelf/cards.rs`. Schon ohne UI per
   Datei bedienbar und live testbar.
4. **UI-Grundlage:** Aktivierungs-Probe, Fenster, Theme, Text, Icons,
   Widgets, Offscreen-Render; eine Testseite.
5. **Settings-Seiten, Tray-Einträge, `boltsnap settings`**, README
   (Support-Matrix, Konfiguration), `docs/codebase.md`. Die Library-Seiten
   sind bis Phase 6 nicht in der Navigation.
6. **Library** nach dem [Library-Plan](2026-10-24-library.md): `boltsnap open`,
   Raster, Vorschaubilder, Aktionen, Papierkorb. Wird nach Phase 5 auf
   Funktionsebene ausgeplant.

Abhängigkeiten: 2 braucht 1; 3 braucht 2 nur für Live-Anwenden über den
Watcher; 5 braucht 2, 3 und 4; 6 braucht 3 (`CardSize`) und 4. Phase 4 kann parallel zu 2 und 3 entstehen.

## 7. Was kaputtgehen kann

| Risiko | Gegenmaßnahme |
| --- | --- |
| Aufrufer-Umbau in Phase 1 ändert stilles Verhalten (heute Default, künftig letzter gültiger Stand) | Fassaden zuerst, Aufrufer einzeln, Test je Aufrufer für den kaputten Fall |
| Schreiber-Umbau verliert Kommentare oder Reihenfolge | K2-Tests bleiben, neue Tests mit kommentierter Beispieldatei und Symlink |
| Watcher-Schleife (eigener Write löst Reload aus) | Inhaltsvergleich, Test |
| Re-Render belastet die CPU beim Ziehen | Drossel 100 ms im UI, Skip veralteter Jobs, Worker mit nice 19 wie die Video-Thumbnails |
| CLI-Vorrendern nutzt alte Größe | bestehender Fallback (PNG dekodieren), Test |
| `mod.rs` wächst weiter | Netto-Abnahme ist Abnahmekriterium von Phase 3 |
| Fokus-Anforderung wirkt auf Hyprland nicht | isolierte Probe zuerst; Fallback ohne zweites Fenster |
| Eigene Widgets fühlen sich schlechter an als GTK | Offscreen-Render und Messung je Widget, Live-Test durch den Nutzer nach Phase 4 an der Testseite, bevor die Seiten gebaut werden |
| Windows-Build bricht durch geteilte Änderungen | `src/settings/` und `src/ui/` sind reine Daten/Pixel; Windows ruft nichts davon außer der Fehlermeldung. Kein Windows-Target lokal installiert: im Plan als offen markieren |

## 8. Abnahme

- Je Phase: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung,
  `cargo test` grün, `git diff --check`.
- Phase 1: Tests für Syntaxfehler, falschen Typ, unbekannten Key, Symlink,
  Modus, parallelen Schreiber (zwei Threads, Lock), Kommentare erhalten.
- Phase 2: IPC-Roundtrip-Tests; live: Handedit in `config.toml` ändert den
  Tray-Haken ohne Aktion; kaputte Datei führt zu genau einer Notification
  und die nächste Aufnahme nutzt weiter `record_fps = 120`.
- Phase 3: live: `card_width = 300` in der Datei, drei Karten (Bild, Video,
  gespeichertes Bild) werden ohne Springen größer und danach scharf;
  `hyprctl layers` zeigt die neue Breite; Ecke oben rechts wächst nach unten.
- Phase 4: Offscreen-Render der Testseite im Dark Theme, angesehen;
  Geometrie-Tests grün; live durch den Nutzer: Hover flackert nicht,
  Tastatur erreicht alles, Scroll gleitet.
- Phase 5: Offscreen-Render jeder Seite; live: jede Zeile einmal geändert
  und in `config.toml` geprüft (Kommentare erhalten), Tray öffnet das
  Fenster, zweiter Aufruf öffnet kein zweites Fenster.

## 9. Entscheidungen

Vom Nutzer am 24.10. so bestätigt:

- Kartengröße 140 bis 380 px, die Höhe skaliert mit (festes
  Seitenverhältnis 190:132).
- „Position“ (Ecke) gehört in v1.
- Phase 0 (Ordner aufräumen) läuft vorab als eigene Commits.
- Ein Boltsnap-Fenster für Library und Settings; Library als Phase 6, nach
  den Settings; Löschen in der Library nur in den Papierkorb.

## 10. Stand

| Phase | Stand |
| --- | --- |
| 0, `docs/plans/` | umgesetzt 24.10.: 18 Dateien per `git mv` in `recording/` (5), `replay/` (2), `shelf/` (3), `platform/` (2), `quality/` (5), `settings/` (3 mit den beiden neuen); 14 Dateien mit Verweisen umgeschrieben (Skript: Markdown-Links relativ zum neuen Ort, Pfade `docs/plans/<name>` im Text). Link-Check vorher und nachher: kein toter Link; `git grep` nach jedem alten Pfad leer (außer `vendor/libway`, das auf eigene Pläne zeigt). |
| 0, `src/platform/linux/` | umgesetzt 24.10.: `capture.rs`, `portal.rs`, `image_transfer.rs` per `git mv` nach `capture/` (`mod.rs`, `portal.rs`, `image_transfer.rs`); `linux/mod.rs` exportiert `image_transfer` und `portal` wieder, alle Aufrufer unverändert; `linux/` hat jetzt 10 Dateien. Abweichungen: nur drei Pfade in `image_transfer.rs` mussten sich ändern (statt etwa 12), und zwar auf `crate::platform::{timing, ipc}`, weil `examples/image_transfer_bench.rs` die Datei per `#[path]` einbindet (das Example bekam dafür ein `mod platform` und den neuen Pfad). `docs/codebase.md` zeigt auf die neuen Orte; Zeilenangaben in älteren Plänen bleiben als historischer Stand. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 25 + 350 grün, Link-Check ohne toten Link. |
| 1, Schritt 1 | umgesetzt 24.10.: `src/settings/` (`schema.rs` 21 Keys mit `Kind`, `Applies`, `strict`; `load.rs` mit `Doc`, `Issue`, `Loaded`; `write.rs` mit `apply`/`write`; 14 Tests). Noch kein Aufrufer. Die Meldungen strikter Keys sind die alten; ein negativer Replay-Wert hieß hier noch „must be 1..600“ statt „invalid“, Schritt 2 stellt das zurück. Gegenprobe: ohne `File::lock` scheitert der Parallel-Test 3 von 3 Mal. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 350 grün. |
| 1, Schritt 2 | umgesetzt 24.10.: `Config::parse`, `load_valid` und `parse_replay_settings` lesen über `settings::load`, `Config` hält nur gültige Werte plus die `Issue`s; `record_profile()` holt den Fehler aus den `Issue`s (`record_fps` vor `record_profile`), Replay nimmt das erste strikte `[replay]`-`Issue`. `check` meldet eine negative Zahl für einen Key mit `min >= 0` als „invalid“, wie der alte Replay-Parser. Vergleich alt gegen neu: ein temporärer Test mit der alten Implementierung prüfte 66 gültige und kaputte Dateien (Profil, Prefs, Pfade, `shelf_to_disk_after`, Replay-Ergebnis samt Meldung, Syntaxfehler ja/nein), alle gleich; ohne die Negativ-Regel schlug er bei `duration_seconds = -1` fehl. Der Test ist wie geplant wieder raus, die Negativ-Meldung steht dauerhaft in `settings/tests.rs`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 350 grün. Kein `cargo check` für Windows (kein Target installiert); die öffentliche API von `Config` ist unverändert. |
| 1, Schritt 3 | umgesetzt 24.10.: `save_recording_prefs_at` schreibt über `settings::update` (neu: Änderungen werden unter dem Lock aus dem frisch gelesenen Text bestimmt) nur die Prefs, deren Wert vom wirksamen Wert der Datei abweicht. Ein ungültiger, unveränderter Wert bleibt samt Kommentar stehen, nie gesetzte Keys kommen nicht dazu (CF3); Symlink, Modus, `fsync` des Ordners (CF2) und eindeutiger Temp-Name (CF4) kommen aus `settings::write`. Abweichung vom Plan: Eine Assertion im bestehenden Test `audio_preferences_parse_and_round_trip` prüfte, dass der unveränderte Default `record_audio_enabled = true` geschrieben wird, also genau das CF3-Verhalten; sie prüft jetzt das Gegenteil. Neuer Test `recording_prefs_write_only_changed_keys_and_keeps_invalid_ones`; Gegenprobe: mit einem Filter, der alles schreibt, schlägt er fehl. Die Tests räumen die `.lock`-Datei mit weg. Neben der echten `config.toml` liegt nach dem ersten Schreiben eine `config.toml.lock`. CF4 selbst (ganzer Snapshot über die Auswahl hinweg) bleibt bis Phase 2 offen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 351 grün. Live nicht geprüft; das folgt mit Schritt 4, der den Daemon anfasst. |
| 1, Schritt 4 | umgesetzt 24.10. wie in „Schritt 4 im Detail“: `Remembered` in `config.rs` merkt sich den letzten gültigen Text; `load` und `replay_settings` lesen ihn, solange die Datei kaputt ist, `load_valid` gibt dann weiter `None`. Meldung einmal je kaputtem Text über `eprintln` und den Hook `config::on_broken`, den `run_daemon` mit `notify` setzt (`shelf/mod.rs` bleibt bei 4016 Zeilen). `Loaded::Broken` hat jetzt `reason`. `Config::parse` loggt nicht mehr selbst; das macht die Meldung, einmal statt bei jedem Lesen. Watchdog: ein Ladefehler läuft über `restart_failed` wie ein gescheiterter Neustart. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 352 grün. Live: installiert, Daemon neu gestartet, er antwortet, Replay `stopped` wie vorher (kein `autostart`), `daemon.log` ohne Config-Meldung. Der kaputte Weg ist live noch nicht geprüft: Er braucht eine kurz kaputte echte `config.toml` und eine Aktion im Daemon (Tray, Aufnahme, Speichern). |
| 1, Schritt 5 | umgesetzt 24.10.: Der Linux-Selector (`start_recording_flow`) speichert über `config::save_selector_prefs` nur die der zwei Keys `record_show_frame`/`record_audio_enabled`, die sich in der Auswahl geändert haben, als Patch unter dem Lock; ein Tray-Wert, der während der Auswahl geschrieben wurde, bleibt (der Rest von CF4 für den Selector). Ein Fehler beim Speichern bricht die Aufnahme nicht mehr ab: Startet eine Aufnahme, geht er nach stderr, weil der Daemon dieselben zwei Werte mit `StartRecording` speichert und ein Scheitern selbst meldet (`Recording settings could not be saved`); bei abgebrochener Auswahl speichert sonst niemand, dann meldet `record_flow` ihn als Notification. `StartRecording` nimmt `show_frame` jetzt direkt aus der Auswahl (vorher über die geänderten `prefs`, gleicher Wert). Abweichung vom Plan: Gelesen wird weiter über `Config::load()`, nicht `load_valid`; im frischen CLI-Prozess ergibt beides bei kaputter Datei dieselben Defaults samt Meldung. Windows (`main.rs:425-430`, eingefroren) unverändert, dort ist `save_selector_prefs` ungenutzt. Neuer Test `selector_prefs_write_only_the_keys_that_changed`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 353 grün; installiert. Der Selector ist interaktiv und wurde nicht live bedient. |
| 2, Schritt 1 | umgesetzt 25.10.: die Tests von `config.rs` liegen in `src/config/tests.rs` (nur verschoben, `config.rs` jetzt 523 Zeilen); `paths::xdg_base` verwirft leere und relative `XDG_CONFIG_HOME`/`XDG_CACHE_HOME` (CF7), Test an der reinen Funktion. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 354 grün. |
| 2, Schritte 2 bis 5 | im Detail geplant 25.10. („Phase 2 im Detail“) |
| 3 bis 5 | geplant, freigegeben 24.10. |
| 6 (Library) | Skizze im Library-Plan |
