# Settings-UI (Plan 2026-10-23)

Status: **Phasen 0 bis 2 umgesetzt (25.10.), 3 und 4 umgesetzt (26.10.), 5 und 6 offen**, Stand in
Abschnitt 10. Richtung vom Nutzer am 23.10. bestätigt
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
- Launcher (Wunsch des Nutzers, 26.10.): ein sichtbarer Eintrag
  `boltsnap.desktop` mit `Name=Boltsnap`, `Exec=boltsnap settings` (ab Phase 6
  `boltsnap open`), `Icon=boltsnap`, `StartupWMClass=boltsnap` (gleich der
  App-ID, damit Launcher und Docks das Fenster dem Eintrag zuordnen),
  `Categories=Graphics;Utility;`. Das Logo als `hicolor`-Icon in 64, 128 und
  256 px; 128 und 256 kommen unverändert aus `assets/windows/boltsnap.ico`
  (enthält 16 bis 256 px), 64 ist `assets/icons/boltsnap-app-64.png`.
  Installiert über `.deb`-Assets und `postInstall` im Flake; für
  `cargo install` stehen die zwei `install`-Befehle im README. Das Binary
  schreibt keine Dateien nach `~/.local/share` von selbst.
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

### Phase 3 im Detail (Gegenprüfung am Code, 26.10.)

Stand der Fundstellen (Zeilen von heute, `shelf/mod.rs` hat 3890 Zeilen):

- Konstanten `CARD_W`/`CARD_H` (`src/shelf/thumbnail.rs:6-7`) nutzen:
  CLI-Vorrendern `main.rs:585-590`; `prepare_add` `shelf/mod.rs:708-740`
  (Client-Thumb nur bei exakt `CARD_W×CARD_H`, sonst PNG dekodieren);
  `prepare_pixels_with` `mod.rs:778-827`; Video-Worker
  `linux/shelf/thumbnail.rs:40,65,71`; Aufnahme-Platzhalter `mod.rs:3148-3161`;
  `debug_render` `src/shelf/mod.rs:15`; Windows `windows/shelf.rs:434,455`.
- `prepare_add` und `prepare_pixels` laufen **im Client-Thread**
  (`mod.rs:640`, `667`), nicht im Eventloop; das Ergebnis kommt als
  `DaemonEvent::AddPrepared` an und wird in `add_prepared` (`mod.rs:1767`)
  übernommen. Die aktuelle Größe muss also auch außerhalb des Eventloops
  lesbar sein.
- `MAX_VISIBLE_CARDS` (`mod.rs:266`) steckt in den freien Funktionen
  `visible_cards` (`:291`), `overflow_card_opacity` (`:357`),
  `animated_item_limit` (`:375`), `animated_card_layout` (`:379-436`) und in
  `card_anim` (`:2050`).
- Ecke: `place_on_output` (`mod.rs:1577`) setzt fest
  `Anchor::BOTTOM | Anchor::LEFT` und `set_margin(0, 0, 24, 24)`; es baut nur
  neu, wenn der Monitor wechselt (`:1580`).
- Layout und Painter: `ThumbRect` hat `u32`-Koordinaten, `blit_thumb_card_anim`
  nimmt `u32` (`paint.rs:37-45`), Windows nutzt beide mit (`windows/shelf.rs:326`).
- Video-Worker: ein Thread, `sync_channel(MAX_CACHED_CARDS)`, `try_send`
  verwirft bei voller Queue (`thumbnail.rs:9-35`). `ThumbnailReady` ersetzt das
  Bild ohne Größenprüfung (`mod.rs:2862`).
- `refresh_recording_prefs` (`shelf/settings.rs:61`) liest `Config::load()` und
  läuft bei `ConfigChanged`, nach jedem `SettingsWritten` und an vier
  Aktionspunkten (`mod.rs:1452`, `2284`, `2327`, `2368`, `3072`).
- `src/shelf/paint.rs` hat 1197 Zeilen und darf nicht wachsen; Phase 3 fasst
  ihn nicht an.

#### Abweichungen vom Plan oben (begründet)

1. **Kein `grow_down`, kein Spiegeln.** Gespiegelte Koordinaten bräuchten für
   die Karte, die bei vollem Shelf oben hinausscrollt, negative `y`; `ThumbRect`
   und der Painter sind `u32` und mit Windows geteilt (eingefroren, kein Target
   zum Prüfen). Stattdessen steht die neueste Karte in jeder Ecke oben im
   Stapel, so wie heute. Unten verankert ändert sich nichts. Oben verankert
   wächst der Layer nach unten, die älteren Karten müssen dabei eine Stufe
   nach unten gleiten: `animated_card_layout` nutzt die vorhandene
   Scroll-Bewegung dann auch bei nicht vollem Shelf (heute nur ab Karte 6).
   Die überzählige Karte verlässt den Viewport unten, wie heute; alle
   Koordinaten bleiben ≥ 0. `Layout::compute` bleibt unverändert, Windows auch.
   Lesart für den Nutzer: überall neueste oben.
2. **Ecke wechselt per Neuaufbau,** wie im Plan, aber ein laufender Drag wird
   vorher mit `clear_drag()` beendet (wie `add_prepared` es schon tut), weil das
   Zerstören der Ursprungs-Surface den Drag sonst ohne Endereignis abreißt.
3. **Re-Render-Jobs als Liste statt Queue-Einträge.** Mit `try_send` in die
   20er-Queue würden beim Ziehen des Sliders (alle 100 ms 20 Jobs) die Jobs
   der *letzten* Größe verworfen und Karten blieben weich. Stattdessen hält
   der Worker eine geteilte Liste `Mutex<Vec<(id, Source)>>`; der Daemon
   ergänzt sie (gleiche `id` wird ersetzt) und weckt den Worker mit einem
   `try_send(Job::Rerender)`. Der Worker arbeitet nach **jeder** Nachricht die
   Liste ab, darum reicht ein verlorenes Wecken: dann liegt schon eine andere
   Nachricht in der Queue. Größe liest der Worker beim Rendern, nicht beim
   Einreihen; veraltete Jobs gibt es dadurch nicht, nur veraltete Ergebnisse,
   und die verwirft der Daemon an der Größe.
4. **Kein `AtomicU64` im Worker, ein `AtomicU32` für alle:**
   `cards::CARD_WIDTH` (Breite, Höhe folgt) ist die eine Quelle für
   Client-Threads, Worker und Daemon. Nur `apply_shelf_prefs` schreibt ihn.
5. **Kein `accept_thumbnail`:** die Prüfung ist
   `image.dimensions() == card_size().dims()` an zwei Stellen
   (`ThumbnailReady`, `add_prepared`); getestet wird sie über `CardSize`.
6. **Shelf-Werte laufen über denselben Refresh wie die Recording-Prefs**
   (eine `Config::load()`), damit sie auch ohne Watcher bei der nächsten Aktion
   ankommen. `refresh_recording_prefs` heißt danach `refresh_prefs`.

#### Verhalten

- Keys, alle `Applies::Live`, nicht strikt (ungültig: Default plus Befund wie
  jeder andere Key), Tabelle `[shelf]`:
  - `card_width`: ganze Zahl 140 bis 380, Default 190. Höhe
    `round(w × 132 / 190)`: 140 → 97, 190 → 132, 300 → 208, 380 → 264.
  - `visible_cards`: 1 bis 8, Default 5.
  - `corner`: `bottom-left` (Default), `bottom-right`, `top-left`, `top-right`.
    Rand 24 px zu beiden Bildschirmkanten der Ecke, wie heute.
- Größe ändert sich (Datei, `set_settings`, Watcher):
  1. `CARD_WIDTH` setzen, jede Karte im Eventloop aus ihrem jetzigen Bild auf
     die neue Größe skalieren (Triangle), `relayout`, `draw`. Ein laufender
     Drag blockiert das Zeichnen schon heute; `clear_drag` zeichnet danach.
  2. Alle Karten (`id`, `Source::Image(png_path)` oder
     `Source::Video(png_path)`) in die Re-Render-Liste, Worker wecken.
  3. Worker: Bild-Karte `fs::read` → `decode_shelf_image` (gleiche Grenzen
     wie beim Hinzufügen) → `make_image_card_thumbnail`; Video wie heute per
     ffmpeg, Filter mit der gelesenen Größe. Ergebnis als `ThumbnailReady`.
  4. Daemon übernimmt nur bei exakt aktueller Größe und nur, wenn die Karte
     noch existiert. Fehler (Datei weg, Dekodieren scheitert): Karte behält das
     skalierte Bild, kein Toast.
- Neue Karten nach dem Wechsel:
  - CLI liest `Config::load().shelf_prefs().card` und rendert in der Größe
    vor. Kaputte Datei: letzter gültiger Stand gibt es im CLI-Prozess nicht,
    also Default 190 plus die übliche `eprintln`-Meldung; der Daemon dekodiert
    dann selbst (bestehender Fallback).
  - `prepare_add` und `prepare_pixels` bekommen die Größe als Parameter, der
    Client-Thread liest `card_size()`.
  - Kommt ein `AddPrepared` mit alter Größe an (Wechsel während der
    Vorbereitung): Stufe-1-Skalierung und ein Re-Render-Eintrag für die Karte.
  - Aufnahme-Platzhalter: `RgbaImage::from_pixel` in aktueller Größe (der
    Umweg über `make_card_thumbnail` entfällt, gleiches Ergebnis).
- `visible_cards` ändert sich: `relayout`, `draw`. Karten jenseits der Zahl
  bleiben im Modell (Cache 20), wie Karte 6 heute.
- Ecke ändert sich: Shelf leer: nur merken, der nächste Aufbau nimmt sie.
  Sonst `clear_drag()` falls aktiv, Layer verwerfen, `place_on_output` baut
  ihn mit neuem Anker auf dem gleichen Monitor neu, wartet auf `configure`,
  zeichnet.
- Unverändert: Drag-Icon behält die Größe beim Drag-Start, Windows nutzt
  weiter 190×132, 5 Karten, unten links.

#### Umsetzung in Schritten (je ein Commit)

1. **Keys und reine Typen.**
   - `settings/schema.rs`: `TABLES = ["replay", "shelf"]`, drei Keys.
   - `shelf/thumbnail.rs`: `CardSize { w, h }`, `DEFAULT`,
     `from_width(i64) -> Option<CardSize>` (Bereich, Höhe gerundet),
     `rescale(&RgbaImage, CardSize) -> RgbaImage`;
     `CARD_W`/`CARD_H` bleiben als `CardSize::DEFAULT.w/h`.
   - `shelf/layout.rs`: `Corner` mit `parse`, `is_top`, `is_right`, `DEFAULT`.
   - `config.rs`: Felder für die drei Keys, `ShelfPrefs { card, visible,
     corner }`, `Config::shelf_prefs()`.
   - Tests: Grenzen und Höhen von `from_width`, `rescale` liefert exakt die
     Zielgröße auch für 1×1 und beim Vergrößern, `Corner::parse`, Config mit
     gültigen, ungültigen und fehlenden `[shelf]`-Keys (ungültig: Default plus
     `Issue`), `"shelf.card_width" = 300` auf oberster Ebene bleibt unbekannt.
   - Noch kein Aufrufer im Daemon.
2. **`linux/shelf/cards.rs` anlegen, nur verschieben:** `prepare_add`,
   `write_temp_png`, `prepare_pixels`, `prepare_pixels_with`,
   `decode_shelf_image`, `shelf_png_decoder` (`mod.rs:706-852`) und
   `visible_cards`, `overflow_card_opacity`, `animated_item_limit`,
   `animated_card_layout` (`mod.rs:291-436` ohne die Cache-/Drag-Helfer
   dazwischen). `mod.rs` holt sie per `use cards::…`, `tests.rs` bleibt
   unverändert. Abnahme: `cargo test` gleich viele Tests grün, `mod.rs` etwa
   230 Zeilen kürzer. Danach 11 Dateien in `linux/shelf/`.
3. **`visible_cards` und Ecke live.**
   - `Daemon::shelf_prefs`, beim Start aus `Config::load()`.
   - Die vier Funktionen nehmen `visible: usize`, `animated_card_layout`
     zusätzlich `top: bool` (Scroll auch bei nicht vollem Shelf); `card_anim`
     und `animated_layout` reichen `self.shelf_prefs` durch.
   - `cards.rs`: `anchor(Corner) -> (Anchor, [i32; 4])`, `place_on_output`
     nutzt es; `apply_shelf_prefs(new)` mit den Fällen oben.
   - `refresh_recording_prefs` → `refresh_prefs`, wendet beides an.
   - Tests: `animated_card_layout` mit `visible` 1 und 8 (Viewport-Höhe,
     keine Überlappung, alle `y` ≥ 0); oben verankert bei Fortschritt 0 liegt
     Karte 1 auf dem alten Platz von Karte 0, bei 1 auf ihrem neuen;
     `overflow_card_opacity` und `animated_item_limit` mit `visible` 1;
     `anchor` für alle vier Ecken.
4. **Kartengröße live.**
   - `cards.rs`: `CARD_WIDTH: AtomicU32`, `card_size()`, `rescale_cards`.
   - `prepare_add(…, size)`, `prepare_pixels*(…, size)`; Client-Threads
     übergeben `card_size()`.
   - `thumbnail.rs`: `Job::Video(id, path) | Job::Rerender`,
     `Source::Image | Source::Video`, geteilte Liste, `extract(path, size)`,
     `render(source, size)`; `submit` bleibt für neue Videos, neu
     `rerender(cards)`.
   - Daemon: Größenprüfung in `ThumbnailReady` und `add_prepared`,
     Platzhalter in `add_finalized_cards`.
   - `main.rs`: CLI rendert mit `Config::load().shelf_prefs().card`.
   - Tests: `prepare_add` nimmt einen Client-Thumb nur in der übergebenen
     Größe; Worker mit Bildquelle (kleine PNG im Temp-Ordner, ohne ffmpeg):
     `rerender` liefert die gelesene Größe, eine Liste mit doppelter `id`
     rendert einmal, fehlende Datei gibt `None`; der bestehende ignorierte
     ffmpeg-Test prüft eine Nicht-Default-Größe.
5. **Doku und live.** README (Konfiguration `[shelf]`), `docs/codebase.md`
   (`cards.rs`), Status hier. Install, Daemon-Neustart (keine Aufnahme läuft),
   echte `config.toml` vorher sichern.

#### Abnahme Phase 3

- Je Schritt `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung,
  `cargo test` grün, `git diff --check`.
- `shelf/mod.rs` nach Schritt 4 kürzer als 3890 Zeilen.
- Live (Schritt 5), über `set_settings` mit dem Scratch-Helfer:
  - Drei Karten (Screenshot, kurzes Testvideo per `add_video` aus einer
    ffmpeg-`lavfi`-Datei im Temp-Ordner, ein gespeicherter Screenshot,
    danach per `safe-rm` wieder weg). `shelf.card_width = 300`:
    `hyprctl layers` zeigt Breite 344 (300 + 2 × 22), ein Screenshot des
    Layers per `grim` zeigt die Karten scharf.
  - `shelf.visible_cards = 2` mit drei Karten: Layer-Höhe für zwei Karten.
  - `shelf.corner = "top-right"`: `hyprctl layers` zeigt den Layer oben
    rechts, ein neuer Screenshot gleitet oben ein, die alten rutschen nach
    unten (Augenschein durch den Nutzer, Bewegung ist per Screenshot nicht
    prüfbar).
  - Datei am Ende byte-gleich mit der Sicherung, `daemon.log` ohne neue
    Fehler.

#### Was Phase 3 nicht macht

- Keine UI, keine Tray-Einträge für die drei Keys (Phase 5).
- Keine Animation des Größenwechsels: die Karten springen in einem Frame auf
  die neue Größe (Stufe 1); Phase 5 drosselt den Slider.
- Windows bleibt bei 190×132, 5 Karten, unten links; keine Änderung an
  `windows/`, `Layout::compute`, `ThumbRect` oder `paint.rs`.
- Kein Re-Render für das Drag-Icon eines laufenden Drags.
- Keine Prüfung, ob die Ecke mit Waybar o. Ä. kollidiert; die exklusive Zone
  bleibt `-1` wie heute.

#### Was kaputtgehen kann

| Risiko | Gegenmaßnahme |
| --- | --- |
| Stufe 1 ruckelt bei 20 großen Karten | Zeit im Schritt-4-Test messen (Release, 20 Karten 190 → 380); über 8 ms: Stufe 1 nur für sichtbare Karten, Rest per Re-Render |
| Neuaufbau des Layers mitten in einer Animation | `anims` laufen nach Zeit weiter, `shelf_configured` wartet auf `configure`; live geprüft |
| Hyprland mappt den Layer oben anders (Waybar-Zone) | exklusive Zone wie heute; live per `hyprctl layers` |
| Re-Render dekodiert 20 große PNGs hintereinander | ein Thread, nacheinander, gleiche Grenzen wie beim Hinzufügen; nur nach einem Größenwechsel |
| Tests teilen den statischen `CARD_WIDTH` | reine Funktionen bekommen die Größe als Parameter, kein Test schreibt den Static |

### Nebenbei: ruckelndes Entfernen (26.10., Meldung des Nutzers)

Befund am Code: Beim Entfernen schrumpft `animated_layout()` die Höhe in jedem
Frame, und `draw()` ruft dafür jedes Mal `layer.set_size` (`shelf/mod.rs`,
`if size_changed`). Hyprland beantwortet jede neue Größe mit einem `configure`,
und der configure-Handler zeichnet sofort (`should_draw`, Karten vorhanden).
Diese Zeichnung hat wieder eine neue Höhe, setzt wieder `set_size` und fordert
einen weiteren Frame-Callback an. So entstehen mehrere Commits und Callbacks
pro Frame, außerhalb des Monitortakts, und der Compositor ordnet den Layer in
jedem davon neu an. Beim Erscheinen tritt das nicht auf, weil die Größe dabei
von Anfang an feststeht. Live nicht reproduziert, weil hier kein Klick-Tool für
den Schließen-Knopf installiert ist.

Verhalten danach: Solange eine Karte verschwindet, behält der Layer seine
Größe. Unten verankert rutschen die Karten im Buffer nach unten (Versatz =
gehaltene Höhe − animierte Höhe), oben verankert rücken sie nach oben, der Rest
ist transparent. Erst wenn die Karte weg ist, schrumpft der Layer einmal auf
die Endgröße (`relayout` in `tick_animations`, wie heute). Außerdem ist nur
noch ein Frame-Callback gleichzeitig angefordert; ein neuer Layer setzt das
zurück.

Code:
- `cards.rs`: `hold_size(layout, (w, h), top) -> Layout`, rein, mit Test.
- `animated_layout()`: bei laufendem Verschwinden `hold_size` mit der
  aktuellen Layer-Größe. `draw()` und `hit_test()` nutzen beide diese Funktion
  und bleiben deckungsgleich.
- `draw()`: `surface.frame` nur, wenn `frame_requested` leer ist;
  `place_on_output` setzt `frame_requested = None`.

Tests: unten verankert behält das gehaltene Layout die Höhe, und die Karten
liegen um die Differenz tiefer; oben verankert bleiben die y-Werte gleich; ohne
Differenz ist das Layout unverändert.

Abnahme: `cargo fmt --check`, clippy, `cargo test`; live schließt der Nutzer
eine Karte, und `hyprctl layers` zeigt danach die Endgröße.

Was es nicht macht: keine andere Kurve oder Dauer (240 ms, `ease_out_cubic`
bleiben). Für die 240 ms fängt der transparente Rest oberhalb Klicks ab, die
der Hit-Test ignoriert. Am Erscheinen ändert sich nichts.

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

### Review Phase 1 und 2 (25.10.)

Zwei unabhängige Reviews über `3d600b0^..fdfa08e` (Lib/Fassade/Protokoll und
Daemon/Tray/Selector), jeder Fund danach am Code nachgeprüft. Kein
Datenverlust, keine Korruption im Writer.

| ID | Prio | Auslöser → Wirkung | Beleg | Reproduziert |
| --- | --- | --- | --- | --- |
| SU1 | P2 | `record_show_frame`/`record_audio_enabled` ändert sich während der Bereichswahl (Tray, Handedit, `set_settings`); `StartRecording` trägt immer beide Werte des Selectors, der Daemon schreibt sie über `persist_tray_prefs` zurück → die Änderung geht auf der Platte verloren. Widerspricht dem Status von Phase 1 Schritt 5 („ein Tray-Wert … bleibt“), das gilt nur bei Abbruch. | `shelf/mod.rs:1710-1717`, `main.rs:367-394` | Code, nicht live (Selector interaktiv) |
| SU2 | P3 | Config-Verzeichnis wird gelöscht (`rm -r ~/.config/boltsnap`) → inotify schickt `IN_IGNORED`, `add_watches` scheitert still, danach kommt nie wieder ein passendes Ereignis: Handedits und `settings_watch` tot bis zum Neustart, ohne Logzeile. | `shelf/settings.rs:205-253` | ja, Python-Probe des Reviewers (nach `rm` `IN_DELETE` + `IN_IGNORED`, nach Neuanlage nichts) |
| SU3 | P3 | `config.toml` nicht lesbar (kein UTF-8, Modus 000, Verzeichnis) → `settings_state` liest `""`, `remember("")` gilt als gültig, setzt `good` auf leer: der Fehler ist unsichtbar und der nächste Syntaxfehler fällt auf Defaults statt auf den letzten gültigen Stand (CF1). | `config.rs:350` (`unwrap_or_default`) | Code |
| SU4 | P3 | Datei wird außerhalb des Locks von `REMEMBERED` gelesen; Watchdog-Thread und Event-Loop können sich über einen Edit hinweg überholen → ein älterer gültiger Text wird zuletzt als `good` gemerkt, oder ein schon reparierter Fehler wird noch einmal gemeldet. | `config.rs:137`, `217`, `350` gegen `336` | Code |
| SU5 | P3 | Gültige Datei, dann gelöscht, dann kaputt neu angelegt → `Config::load` ruft bei `NotFound` `remember` nicht auf, die Werte der gelöschten Datei kommen als „letzter gültiger Stand“ zurück; ob das passiert, hängt davon ab, ob `settings_state`/`replay_settings` dazwischen lief. | `config.rs:217-225` | Code |
| SU6 | P3 | `Remove` eines Keys in einer fehlenden Tabelle (`[["replay.fps", null]]` ohne `[replay]`) legt eine leere, nicht implizite `[replay]` an → Datei wird geschrieben (oder angelegt), obwohl sich nichts ändert, und der Watcher feuert. | `write.rs:36-37` | Code (toml_edit 0.20.2 `Table::new` nicht implizit) |
| SU7 | P3 | `"replay.fps" = 30` auf oberster Ebene: `schema::find` trifft, also kein „unknown setting“, gelesen wird es aber nie → still ignoriert. | `load.rs:128` | Code |
| SU8 | P3 | Entprellung zählt jedes Ereignis im Ordner und hat keine Obergrenze → bei einem Symlink in einen dauernd beschäftigten Ordner kommt `changed()` erst, wenn der Ordner ruhig ist. | `shelf/settings.rs:221` | Code |
| SU9 | P4 | `IN_Q_OVERFLOW` (wd −1) passt nie → eine Änderung kann verloren gehen; praktisch kaum erreichbar. | `shelf/settings.rs:215-217` | Code |
| SU10 | P4 | `create_new` scheitert mit `AlreadyExists` (alte Temp-Datei mit gleicher PID) → der Fehlerpfad löscht eine Datei, die er nicht angelegt hat. | `write.rs:127-150` | Code |
| SU11 | P4 | Kommentar „kein Lock im Dotfiles-Checkout“ stimmt nur für eine verlinkte Datei, nicht für ein verlinktes Verzeichnis. | `write.rs:89` | Code |

Nicht übernommen: Der Daemon-Reviewer schlug für SU1 ein Protokoll mit
`Option<bool>` vor. Einfacher und ohne Protokolländerung: Der Daemon schreibt
bei `StartRecording` gar nichts mehr, der Selector hat die geänderten Keys
schon geschrieben. Dafür schluckte der Daemon bisher einen Fehler des
Selectors; der Selector meldet ihn jetzt selbst als Notification. Bleibt
offen (bewusst): Ein Tray-Write, der beim Start der Auswahl noch in der
Warteschlange lag, kann einen im Selector geänderten Wert danach
überschreiben (Fenster von einem fsync).

Fixplan, je ein Commit:

1. **Config lesen (SU3, SU4, SU5)** in `config.rs`: `read_noted(path,
   remembered)` liest die Datei und ruft `see` unter demselben Lock;
   `NotFound` liest als `""` (wird gemerkt, setzt also zurück), jeder andere
   Lesefehler wird nicht gemerkt. `load`: bei Lesefehler der letzte gültige
   Stand, sonst Defaults; `replay_settings`: Fehler wie bisher
   `read replay settings: …`; `settings_state`: Werte des letzten gültigen
   Stands, `broken` mit `line: None` und dem Lesefehler. Tests an
   `read_noted` mit eigenem `Mutex<Remembered>` und Temp-Datei: gelöscht
   setzt zurück, ein Verzeichnis als Pfad gibt `Err` und behält `good`.
2. **Lib (SU6, SU7, SU10, SU11):** `apply` legt für `Remove` keine Tabelle
   an; oben gilt ein Schema-Key nur ohne Tabellenteil; `replace` räumt nur
   eine selbst angelegte Temp-Datei weg; Kommentar korrigiert. Tests:
   `Remove` ohne Tabelle lässt Text gleich, quotierter Punkt-Key wird
   gemeldet.
3. **Daemon (SU1):** `StartRecording` schreibt nichts; `start_recording`
   bekommt `audio_enabled` und nutzt ihn für diese Aufnahme; der Selector
   meldet einen Speicherfehler bei gestarteter Aufnahme als Notification.
4. **Watcher (SU2, SU8, SU9):** Ereignisse mit Maske parsen; `IN_IGNORED`
   entfernt den Watch, fällt der Ordner-Watch weg, versucht der Thread alle
   1 s neu und ruft danach `changed()`; `IN_Q_OVERFLOW` zählt als Änderung;
   Entprellung höchstens 1 s nach dem ersten passenden Ereignis. Test: der
   bestehende Watcher-Test löscht den Ordner, legt ihn neu an, schreibt, und
   erwartet einen Aufruf; Gegenprobe ohne Neuversuch.

Abnahme: je Commit `cargo fmt --check`, `cargo clippy --all-targets` ohne
Warnung, `cargo test`; danach installieren, Daemon neu, `get_settings`
antwortet. SU2 wird nur im Temp-Ordner geprüft, nie an der echten Config.

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

### Phase 4 im Detail (Gegenprüfung am Code, 26.10.)

#### Befund

| Was | Beleg |
| --- | --- |
| SCTK ist 0.19.2, `default = ["calloop", "xkbcommon"]` | `Cargo.lock`, `smithay-client-toolkit-0.19.2/Cargo.toml` |
| Fenster: `Window::set_app_id`, `set_title`, `set_min_size`, `request_decoration_mode`, `WindowHandler::{configure, request_close}`, `WindowConfigure::new_size` als `(Option<NonZeroU32>, …)` | `src/shell/xdg/window/mod.rs:71, 207–277` |
| Aktivierung: `ActivationState::request_token(qh, RequestData)`, Antwort in `ActivationHandler::new_token`, dann `activate(surface, token)` | `src/activation.rs:18, 64–111` |
| Tastatur mit Wiederholung über calloop: `SeatState::get_keyboard_with_repeat(qh, seat, None, loop_handle, callback)`; `KeyEvent { keysym, utf8 }`, `Modifiers { ctrl, shift, … }` | `src/seat/keyboard/repeat.rs:58`, `keyboard/mod.rs:242, 266` |
| Mausrad: `PointerEventKind::Axis { vertical: AxisScroll { absolute, discrete, stop }, source }`; Touchpad liefert `discrete = 0` | `src/seat/pointer/mod.rs:51, 109` |
| Scale: `CompositorHandler::scale_factor_changed(…, new_factor: i32)` | `src/compositor.rs:26` |
| Cursor: `ThemedPointer` + `CursorIcon`, so wie Shelf und Selector | `linux/shelf/mod.rs:35, 3358`, `linux/select_skia/mod.rs:22` |
| Text auf `tiny_skia::Pixmap`: ab_glyph-Outline, Deckung src-over; Breite über `h_advance` | `selector/render.rs:417, 446` |
| UI-Font: `load_ui_font_weights(family, (500, 600))` liegt in der Binär-Crate; Fallback `fallback_popup_font` (eingebettetes DejaVu-Subset) ebenso | `linux/shelf/font.rs:52`, `shelf/font.rs:3`, `assets/fonts/dejavu-badge.ttf` |
| Farben und Maße der Controls (V9): `#121212`, `#222222`, `#ECECEC`, `#969696`, `#6A6A6A`, Rand 16, Control 30, Titel 15, Label 13, Key 11 | `shelf/paint/controls.rs:7–22` |
| Lucide liegt lokal: `lucide-static` 1.54.0 (ISC), 2134 SVGs, nur `path`, `circle`, `rect`, `line`, `polyline`, `polygon`, `ellipse`, Bögen (`a`) häufig | `~/projects/stepi/node_modules/lucide-static/icons/` |
| tiny-skia 0.11 `PathBuilder` kennt keine Bögen | `move_to`, `line_to`, `quad_to`, `cubic_to`, `close`, `push_circle`, `push_oval` |
| Hyprland: `misc:focus_on_activate = false` (Default), also macht eine Aktivierung das Fenster vermutlich nur „urgent“ | `hyprctl getoption misc:focus_on_activate`, 26.10. |
| Hyprland-Lua fokussiert ein Fenster per `hyprctl eval 'hl.dispatch(hl.dsp.focus({ window = "address:…" }))'`; ein unbekanntes Fenster gibt eine Warnung, falsche Argumente einen Fehler | live geprüft am schon aktiven Fenster, 26.10. |
| `src/platform/linux/` hat 10 Dateien, `src/` 6 Ordner-Module plus `lib.rs`/`main.rs`; `src/shelf/paint.rs` hat 1197 Zeilen und wächst nicht | `ls`, `wc -l` |

#### Abweichungen vom Plan oben

1. **Raise-Weg erst nach der Probe.** Da `focus_on_activate` aus ist, prüft
   die Probe zwei Wege: (a) Aktivierung mit Token ohne Serial, (b)
   „Neu-Mappen“: das laufende Fenster baut sein `xdg_toplevel` neu auf,
   und Hyprland fokussiert neue Fenster. Gewinnt (a), bleibt es dabei. Sonst
   (b), weil das ohne Compositor-eigenen Code auskommt; das Fenster landet
   dann auf dem aktuellen Workspace. Die Notification „already open“ bleibt
   der letzte Ausweg. Hyprland-Dispatch per `hyprctl eval` nutzt die Probe nur
   als Messwerkzeug, nicht im Produkt.
2. **Kein `fallback_frame` in Phase 4.** Hyprland, Sway und KDE bieten
   `xdg-decoration`; hier gibt es keinen Compositor ohne, also wäre das
   ungetesteter Code. Das Fenster fordert Server-Deko an; ohne Server-Deko
   erscheint es ohne Rahmen. Bleibt offen für Phase 5.
3. **Single-Instance (`single.rs`) kommt in Phase 4**, nicht erst in 5, weil
   sie direkt an der Probe hängt.
4. **Die Lib braucht ihren eigenen Fallback-Font:** `src/ui/text.rs` bettet
   dasselbe `assets/fonts/dejavu-badge.ttf` per `include_bytes!` ein (die
   Binär-Crate-Funktion ist aus der Lib nicht erreichbar). Echte Fonts kommen
   über `Fonts::new(medium, demibold)` von der Plattform.
5. **Icons:** neben `path` liest der Parser die anderen sechs Lucide-Elemente
   (je eine Zeile auf `PathBuilder`). Bögen werden nach SVG-Anhang F.6 in
   höchstens 90°-Kubikstücke zerlegt.
6. **Navigation-Icons 13 statt 16 px:** neben 13-px-Text wären 16 px größer
   als der Text (harte UI-Regel). 16 px nur für freistehende Icons auf
   Icon-Buttons (`ICON_ALONE`).
7. **Eine Fläche mehr:** `SELECTED #383838` für das gewählte Segment und den
   Eintrag unter dem Zeiger in einer offenen Liste; `#2A2A2A` allein ist neben
   `#222222` als Auswahl zu schwach.
8. **Keine Seitentitel:** die Navigation nennt die Seite schon; die Seite
   beginnt mit der ersten Gruppenüberschrift.

#### Verhalten der Testseite

`boltsnap settings` öffnet in Phase 4 ein Fenster „Boltsnap“ (App-ID
`boltsnap`, 760×520, Minimum 560×400) mit Navigation links (zwei Einträge:
Widgets, Text) und rechts einer Galerie: je Widget eine Zeile mit Icon, Name,
Hinweis und dem Control, dazu je eine deaktivierte Variante. Werte ändern sich
nur im Fenster, nichts geht an den Daemon oder die Datei. `Esc`, `Ctrl+W`,
`Ctrl+Q` und das Fenster-X schließen.
`boltsnap settings --render <page> <out.png> [--scale N]` zeichnet dieselbe
Seite ohne Fenster. Unter Windows: Fehler „settings are not available on
Windows“.

Zustände je Widget: normal, Hover, Fokus, gedrückt/aktiv, deaktiviert.
Hover heißt eine Stufe heller (`#2A2A2A`) mit 90 ms Übergang; Fokus heißt
dieselbe Fläche plus hellerer Text, kein Ring. Deaktiviert ist Text
`#6A6A6A`, keine Hover-Reaktion. Was ausblendet (Dropdown-Liste), nimmt keine
Eingaben mehr an. Ein Zeiger-Verlassen (auch durch Fokusverlust des
Fensters) löscht den Hover.

| Widget | Maus | Tastatur | Animiert |
| --- | --- | --- | --- |
| Schalter | Klick schaltet | `Space`/`Enter` | Knopf gleitet 140 ms |
| Segment | Klick auf Teil | `←/→` | Auswahlfläche gleitet 140 ms |
| Dropdown | Klick öffnet, Klick auf Eintrag wählt, Klick daneben schließt | `Enter`/`Space` öffnet, `↑/↓`, `Enter` wählt, `Esc` schließt | Liste blendet 90 ms ein, öffnet nach oben, wenn unten kein Platz ist |
| Slider | Klick springt, Ziehen folgt, Wert rechts | `←/→` ein Schritt, `Home/End` | Knopf folgt direkt (Ziehen ist direkte Manipulation) |
| Stepper | `−`/`+`, Grenzen deaktivieren den Knopf | `←/→` | keine |
| Textfeld | Klick setzt Cursor, Ziehen wählt aus, Doppelklick ein Wort | Zeichen, `←/→` (`Shift` wählt, `Ctrl` wortweise), `Home/End`, `Backspace`/`Delete` (`Ctrl` wortweise), `Ctrl+A/C/X/V`, `Enter` bestätigt | Cursor blinkt nicht (kein Dauer-Redraw) |
| Icon-Button | Klick | `Space`/`Enter` | Hover |
| Banner | – | – | – |

Navigation: Auswahlfläche gleitet 180 ms (Minimum-Jerk) zum gewählten
Eintrag, `↑/↓` wenn sie Fokus hat, `Ctrl+1`/`Ctrl+2`. `Tab`/`Shift+Tab`
läuft durch Navigation und alle aktiven Controls der Seite; ein fokussiertes
Control unterhalb des sichtbaren Bereichs scrollt in Sicht.
Scroll: Mausrad-Rastung = 48 px Ziel, gleitet 160 ms aus absoluter Zeit,
weitere Rastungen verschieben das Ziel mit Übernahme der Geschwindigkeit;
Touchpad (`discrete == 0`) folgt pixelgenau. Reduced Motion: keins (kein
Signal unter Wayland); die Wege sind kurz.

#### Code

Lib (`src/lib.rs` bekommt `pub mod ui; pub mod app;`), keine Wayland-Typen:

- `src/ui/mod.rs`: `Rect`, `Input` (Zeiger, Rad, Taste, Text), `Key`,
  `Effect` (`Copy(String)`, `Paste`, `Close`, `Redraw`), `Canvas`
  (Pixmap + Scale).
- `src/ui/theme.rs`: Farben, Schriftskala (`Title 15/600`, `Label 13/500`,
  `Hint 12/500`, `Key 11/500`), Abstände (4, 8, 12, 16, 24), `CTL_H 30`,
  `RADIUS 9`, `ROW_H 44`, Icon 13/16, Strich 1,5, Dauern.
- `src/ui/text.rs`: `Fonts`, `measure`, `draw` (Baseline auf
  Versalien-Mitte einer Box, wie `cap_baseline` in `controls.rs:73`),
  `ellipsize`, Cursor-Positionen für das Textfeld.
- `src/ui/icons.rs`: SVG-Parser (Elemente, `d`-Befehle samt Bögen), Registry
  `Icon::Name` → eingebettete SVG aus `assets/icons/ui/`, `draw(icon, rect,
  color)` mit Strich 1,5 px in Zielgröße.
- `src/ui/motion.rs`: `Tween` (Minimum-Jerk `10u³ − 15u⁴ + 6u⁵`, Start,
  Ziel, Dauer, `retarget` mit Geschwindigkeitsübernahme), `ScrollState`.
- `src/ui/widgets/`: `mod.rs`, `switch.rs`, `segment.rs`, `dropdown.rs`,
  `slider.rs`, `stepper.rs`, `text_field.rs`, `button.rs` (Icon-Button und
  Banner). 8 Dateien.
- `src/ui/tests.rs`.
- `src/app/mod.rs`: `App` mit Navigation, Seiten, Fokus, Scroll;
  `handle(Input) -> Vec<Effect>`, `draw(&mut Canvas)`, `animating()`.
- `src/app/gallery.rs`: die zwei Testseiten.
- `assets/icons/ui/lucide.svg`: die genutzten Symbole aus
  `lucide-static/sprite.svg`, unverändert, plus `LICENSE` (ISC). Abweichung:
  ein Sprite statt Einzeldateien, weil Phase 5 sonst über 40 Dateien in einen
  Ordner legt (Limit 12).

Plattform:

- `src/platform/linux/app/mod.rs`: `run(page)` mit calloop-Loop,
  `render(page, out, scale)` offscreen.
- `src/platform/linux/app/window.rs`: SCTK-Registry, `Window`,
  `SlotPool`, Buffer-Scale, Frame-Callbacks nur, solange
  `App::animating()`.
- `src/platform/linux/app/input.rs`: Pointer, Tastatur mit Wiederholung,
  Rad → `Input`; Cursor über `ThemedPointer`; `Effect::Copy/Paste` über
  `wl-clipboard-rs`.
- `src/platform/linux/app/single.rs`: `flock` auf `<runtime>/app.lock`,
  `<runtime>/app.sock` nimmt „raise <page>“ an; Weg je nach Probe.
- `src/main.rs`: Befehl `settings`; `platform/mod.rs` und
  `platform/windows` liefern die Windows-Meldung.

#### Schritte

1. **Probe** (`examples/activation_probe.rs`, wird danach entfernt):
   öffnet ein kleines Fenster (App-ID `boltsnap-probe`), liest Befehle von
   stdin: `activate` (Token ohne Serial anfordern, eigenes Surface
   aktivieren), `remap` (Toplevel neu aufbauen), `quit`. Ein Skript merkt
   sich das aktive Fenster, fokussiert es vor jedem Befehl zurück
   (`hyprctl eval`), schickt den Befehl und liest danach
   `hyprctl activewindow -j` und `urgent` aus `hyprctl clients -j`. Am Ende
   fokussiert es das ursprüngliche Fenster wieder. Dauer etwa 3 s.
   Ergebnis und Entscheidung kommen in den Stand (Abschnitt 10).
2. **UI-Grundlage:** `theme`, `text`, `icons`, `motion`, Assets. Tests:
   jede Registry-Icon-Datei parst; Bogen-Endpunkt trifft das Ziel (±0,01);
   `measure` gleich Summe der Advances; `ellipsize` passt in die Breite und
   endet auf „…“; Minimum-Jerk bei 0/0,5/1 = 0/0,5/1, Retarget ohne Sprung
   in Position und Geschwindigkeit.
3. **Widgets, App, Galerie, Offscreen-Render:** Tests messen am gezeichneten
   Layout: jede Control-Höhe 30, Zeilen 44, Rand 16 an allen Seiten der
   Seite, Icon-Box ≤ Label-Größe und auf Versalien-Mitte, keine zwei
   Rechtecke überlappen, Tab-Reihenfolge = Lesereihenfolge; je Widget die
   Tastatur- und Maus-Abläufe aus der Tabelle als Eingabe-Folgen. Danach
   `boltsnap settings --render widgets` im Dark Theme ansehen.
4. **Fenster und Single-Instance:** `linux/app/`, Befehl `settings`.
   Live: Fenster öffnen, Screenshot per `boltsnap full --no-copy -o`
   (Zwischenablage bleibt), zweiter Aufruf öffnet kein zweites Fenster und
   holt das erste nach vorne (Weg aus Schritt 1), `Esc` schließt.
5. **Doku:** `docs/codebase.md` (neue Ordner), Stand im Plan. README erst in
   Phase 5 (der Befehl hat dann echte Seiten).

#### Abnahme

`cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung,
`cargo test`, `git diff --check`; Offscreen-Render angesehen; Probe-Ergebnis
im Plan; live die Punkte aus Schritt 4. Durch den Nutzer: Hover flackert
nicht, Tastatur erreicht alles, Scroll gleitet.

#### Was Phase 4 nicht macht

- Keine echten Settings-Seiten, kein IPC zum Daemon, kein Tray-Eintrag,
  kein Ordner-Dialog (alles Phase 5).
- Kein `fallback_frame` (siehe Abweichung 2), kein Fractional Scaling.
- Kein Cursor-Blinken, keine Tooltips, kein Rechtsklick-Menü im Textfeld,
  keine IME (`text-input`), keine Primärauswahl.
- Keine Library-Seiten (Phase 6).

#### Was kaputtgehen kann

| Risiko | Gegenmaßnahme |
| --- | --- |
| Probe stört den Nutzer (Fenster erscheint, Fokus wandert) | etwa 3 s, Fokus wird danach zurückgegeben |
| Keine der beiden Raise-Varianten fokussiert | Notification „Boltsnap is already open“, kein zweites Fenster |
| Eigene Font-Metriken setzen Text optisch zu tief | Baseline über Versalhöhe (`cap_baseline`), Test misst die Icon-Mitte gegen die Versalien-Mitte |
| Bogen-Umrechnung verzerrt Icons | Endpunkt-Test plus Render der Icons im Offscreen-Bild |
| `src/` bekommt zwei neue Ordner | `src/` hat 6, danach 8; jede neue Datei unter 500 Zeilen |
| Tastatur-Wiederholung mit calloop-Version | dieselbe calloop 0.13 wie SCTK (`Cargo.toml`-Kommentar), Build zeigt es |

### Phase 4b: Überarbeitung nach `/ui` (26.10., Meldung des Nutzers)

#### Befund

Der Nutzer, nach dem ersten Öffnen: wirkt „sehr low quality“, zeigt einen
Fehler, „nicht wie ein richtiges Settings-UI“, „nicht alles beim Resizen
responded“. Live-Screenshot bei 946×1032 (gekachelt) und Code gegengelesen:

| # | Befund | Beleg |
| --- | --- | --- |
| B1 | Der „Fehler“ ist das erfundene Demo-Banner der Galerie („config.toml has an error on line 3“), rot. Verstößt gegen „keine erfundenen Demo-Daten“. | `app/gallery.rs:64-68` |
| B2 | Die Seite ist eine Testgalerie: lose Zeilen ohne Fläche, eine ganze Gruppe „Disabled“ als Duplikat, Werte ohne Bezug zur echten Config. Struktur nur über Abstand trägt bei 700 px breiten Zeilen nicht, Label und Control stehen 500 px auseinander. | Screenshot, `app/mod.rs:205-212` |
| B3 | Inhalt läuft über die ganze Breite, ohne Höchstbreite; bei 1900 px Fenster noch weiter auseinander. | `block_rects`: `w = size.0 - NAV_W - 2·EDGE` |
| B4 | Controls haben feste Breiten (Textfeld 240, Slider 160), die Navigation ist immer 184 breit, nichts stapelt: unter etwa 600 px wird das Label zur Ellipse, bei sehr schmalen Kacheln (Hyprland hält `min_size` gekachelt nicht ein) schneidet es ab. | `TextField::new(_, 240.0)`, `Slider` `TRACK_W`, `NAV_W` |
| B5 | Motion ist Minimum-Jerk mit festen Dauern; `/ui` verlangt seit 25.10. Federn mit drei Tokens. Druck- und Fokuswechsel sind hart (kein Press-Zustand, Fokusfläche schaltet um), der Seitenwechsel tauscht den Inhalt hart. | `ui/motion.rs`, `widgets/mod.rs` `Look` |
| B6 | Kein Scroll-Hinweis: lange Seiten zeigen nicht, dass es weitergeht. | `app/draw.rs` |
| B7 | Die Gates aus `/ui` (Überlauf, Versalien-Zentrierung, Padding, Kontrast, Motion-Liste, Copy) gibt es nicht als Messung. | `app/tests.rs` |

#### Richtung (ersetzt „Gestaltung“ oben, wo es abweicht)

Dieselbe Familie wie die Recording-Controls, aber mit Flächen als Struktur:
Gruppen liegen auf einer eigenen Fläche, wie in den Systemeinstellungen
der großen Desktops, ohne Rahmen, Linien oder Schatten. Abweichung vom
Abschnitt „Aufbau“ („Gruppen nur über Abstand“), Grund: B2.

- **Flächen:** Inhalt `#121212` (`WINDOW`), Navigation `#171717`
  (`SIDEBAR`, ganze Höhe), Gruppe `#1C1C1C` (`PANEL`), Control auf der
  Gruppe `#262626` (`RAISE`), Hover `#2E2E2E`, gewählt/gedrückt `#3A3A3A`.
  Text `#ECECEC`, Neben-Text `#9A9A9A`, gedimmt `#6A6A6A` (nur deaktiviert).
  Akzent invertiert wie bisher (Schalter an, Slider-Füllung). Fehler
  `#E5484D` nur für echte Fehler.
- **Schrift:** `title` 18/600 (Seitentitel, neu sichtbar), `label` 13/500,
  `heading` 13/600, `hint` 12/500, `key` 11/500.
- **Abstände:** 4, 8, 12, 16, 24. Seitenrand 24, Titel zu erster Gruppe 16,
  Gruppenkopf zu Fläche 8, zwischen Gruppen 24.
- **Maße:** Controls 30 hoch, Radius 9; Gruppe Radius 12 (Padding 12 ≥
  Radius 9, also kleinere Stufe innen); Zeile 46 (30 + 2×8), Gruppe innen
  4 oben/unten und 12 links/rechts, damit Inhalt an allen vier Seiten 12
  vom Rand der Fläche sitzt. Inhaltsspalte höchstens 680 breit, mittig im
  Bereich rechts der Navigation. Navigation 200 breit, Einträge 30 hoch,
  Rand 8.
- **Responsiv**, in der Reihenfolge aus `/ui`: (1) Spalte schmaler bis
  zum Fensterrand; Textfeld und Slider wachsen und schrumpfen mit der
  Zeile (Textfeld 160 bis 360, Slider-Spur 120 bis 240). (2) Passt Label
  (mindestens 120) plus Control nicht in die Zeile, stapelt die Zeile:
  Label oben, Control darunter über die volle Zeilenbreite (Schalter und
  Icon-Buttons bleiben rechts neben dem Label). (3) Unter 600 px
  Fensterbreite zeigt die Navigation nur Icons (46 breit, Icon 16 frei
  stehend). Mindestgröße des Fensters 360×320.
- **Scroll-Hinweis:** dünner Daumen (4 breit, Radius 2) am rechten Rand
  des Inhalts, nur wenn die Seite länger ist als das Fenster.

#### Motion

Drei Spring-Tokens, kritisch gedämpft (`bounce 0`), analytisch aus
absoluter Zeit, Retarget übernimmt Wert und Geschwindigkeit
(`/ui` motion.md §4, §5):

| Token | Dauer | Wofür |
| --- | --- | --- |
| `QUICK` | 150 ms | Hover, Druck, Fokus, Liste ein/aus, Text heller |
| `SNAP` | 200 ms | Schalterknopf, Auswahl im Segment, Auswahl in der Navigation, Markierung in der offenen Liste |
| `GLIDE` | 300 ms | Mausrad-Scroll, Scroll zum fokussierten Control, Seitenwechsel (Inhalt blendet ein und steigt 6 px) |

Vorschlag für „Ohne Animation“ (entscheidet der Nutzer; bis dahin so
gebaut): Tippen, Caret und Textauswahl im Textfeld; die Zahl in Slider und
Stepper (der Knopf gleitet, die Zahl wechselt); Neu-Layout beim Resize
(folgt dem Compositor direkt). Alles andere animiert. Reduced Motion: der
Desktop meldet hier keine Einstellung; `App::reduced_motion` lässt den
6-px-Weg beim Seitenwechsel weg, die Blende bleibt (Test).

#### Inhalt

Die Galerie fliegt aus dem Fenster. `boltsnap settings` zeigt die Seiten
und Zeilen aus der Tabelle oben mit den echten Werten aus
`Config::load()` (über ein reines `app::pages::Prefs` in der Lib, damit
Phase 5 nur die Quelle tauscht):

- Shelf: Card size, Cards shown, Position.
- Screenshots: Save folder.
- Recording: Both displays, Frame rate, Cursor, Show frame; Audio, Audio
  source; Video folder, Disk save also to shelf, Shelf to disk after.
- Replay: Start with daemon, Length, Memory, Frame rate.

Was eine Liste vom System braucht (Display, Codec, Font, Replay Display,
Bitrate) und der Ordner-Dialog kommen mit Phase 5. Bis Phase 5 schreibt
das Fenster nichts; oben steht deshalb ein ruhiges Info-Banner „Preview:
changes here are not saved yet“ (Neben-Text-Farbe, kein Rot). Die Galerie
bleibt als Stress-Fixture für Tests und `--render --fixture` (lange
Labels, langer Pfad, deaktivierte Varianten).

#### Gates (als Tests, Zahlen in den Bericht)

1. **Überlauf:** jede Seite plus Fixture bei 360, 480, 760 und 1400 px
   Breite: jedes Teil einer Zeile liegt in seiner Gruppe (12 Rand),
   Label und Control überlappen nicht, die offene Liste liegt im Fenster.
2. **Versalien:** Pixelmessung im Render (Scale 2): Abstand Versalien-Oberkante
   zu Control-Oberkante gegen Grundlinie zu Unterkante, `|off| ≤ 0,5`
   logische px, je Control-Art mit Text. Einmal mit dem echten UI-Font
   (Outfit) über einen `#[ignore]`-Test im Bin-Crate.
3. **Padding und Achsen:** Gruppen-Innenabstand an allen vier Seiten 12
   (±0,5); Label-Mitte gleich Control-Mitte in einzeiligen Zeilen; Icon
   ≤ Textgröße.
4. **Kontrast:** alle genutzten Text/Fläche-Paare ≥ 4,5:1 (deaktiviert
   ausgenommen).
5. **Motion:** Feder-Tests (exaktes Ende, Retarget stetig in Wert und
   Geschwindigkeit, gleiche Bahn bei 60/120/144 Hz, Deckkraft in [0, 1]),
   Liste der Zustandswechsel oben.
6. **Texte:** Test über alle Strings der Seiten: keine Ausrufezeichen,
   keine Gedankenstriche, Satzanfang groß, Rest klein (Ausnahmen: Einheiten,
   „FPS“-freie Zahlen), Denylist aus `/ui`.
7. **Screenshot:** `--render` bei 360, 760 und 1400 Breite angesehen.

#### Code

- `ui/motion.rs`: `Spring` ersetzt `Tween` (gleiche Methoden, `retarget`
  nimmt ein Token), `Scroll` auf `GLIDE`.
- `ui/theme.rs`: Farben, Maße, Tokens wie oben; `Duration`-Konstanten
  entfallen.
- `ui/widgets/`: `Look` bekommt `press` und `focus` als Wert (0 bis 1),
  `focused` bleibt Bool für die Logik; `Widget::width(fonts, room)`;
  Textfeld und Slider flexibel; Dropdown-Markierung gleitet.
- `app/layout.rs` (neu): Spalte, Gruppen-Flächen, Zeilen samt Stapeln,
  kompakte Navigation; `app/mod.rs` gibt das ab und bleibt unter 500.
- `app/pages.rs` (neu): `Prefs` und die echten Seiten. `app/gallery.rs`
  wird zur Fixture.
- `app/draw.rs`: Navigation-Fläche, Gruppen, Titel, Scroll-Daumen,
  Seitenblende.
- `app/gates.rs` (neu, `cfg(test)`): Gates 1 bis 6.
- `linux/app/`: `Prefs` aus `Config`, `--render … [--size WxH]
  [--fixture]`, Mindestgröße 360×320.

#### Was Phase 4b nicht macht

- Kein Schreiben, kein IPC, keine System-Listen, kein Ordner-Dialog,
  kein Tray- oder Launcher-Eintrag (Phase 5).
- Kein Hell-Theme, kein Fractional Scaling, keine Tooltips.

### Phase 4c: Harte Wechsel an Schwellen (26.10., Meldung des Nutzers)

Befund: 4b hat „Neu-Layout beim Resize“ als Ganzes auf die Ohne-Animation-Liste
gesetzt. Das stimmt nur für stetiges Mitziehen (Spaltenbreite, Spurbreiten).
Wechsel an einer Schwelle sind Zustandswechsel und schalteten hart. Inventar
aller harten Wechsel im Fenster, mit Abhilfe:

| # | Wechsel | vorher | jetzt |
|---|---|---|---|
| M1 | Navigation breit ↔ schmal (600 px) | Breite 200 ↔ 46 und Namen springen | Feder `nav_open` mit `GLIDE`; Breite, Icon-Lage und -Größe (13 ↔ 16) folgen ihr, Namen blenden mit `2·open − 1` und werden am Rand abgeschnitten statt gekürzt |
| M2 | Zeile neben ↔ gestapelt | Höhe 46 ↔ 76, Control springt unter den Namen | Feder je Zeile mit `GLIDE`; Höhe, Namen- und Control-Rechteck interpoliert, Gruppen darunter wandern mit |
| M3 | Segment gleich breit ↔ nach Inhalt | Teilbreiten und Gesamtbreite springen | stetig: Breite `room` zwischen knapp und gleich; Teile gleich breit, wer seinen Inhalt nicht fasst, behält Inhalt + 2·`GAP_M`, der Rest teilt sich den Platz |
| M4 | Scroll-Daumen erscheint/verschwindet | hart | Feder `QUICK`, unter der Seitenblende |
| M5 | Name des gewählten Navigationseintrags | hart TEXT ↔ SUB | Kanal `Current` in `Fades`, `QUICK` |
| M6 | Text des gewählten Segment-Teils | hart TEXT, während die Fläche gleitet | Helligkeit aus dem Abstand zur gleitenden Fläche |
| M7 | Stepper-Knopf an der Grenze | Icon und Hover hart zu DIM | Feder `live` je Knopf, `QUICK` |
| M8 | Textfeld Fokus | gekürzter ↔ voller Pfad und Cursor hart | Überblendung über den Fokus-Fade |

Ohne Animation (Vorschlag, mt entscheidet): Tippen, Cursor-Bewegung und
Auswahl; Zahl in Slider und Stepper; der angezeigte Wert im Dropdown beim
Wählen (die Liste blendet darüber aus); stetiges Mitziehen mit der
Fenstergröße (Spalte, Spuren, Scroll-Grenze). Reduced Motion: M1 und M2
springen.

Dazu (Meldung des Nutzers, gleicher Tag): Schrift zu klein und unscharf.
Befund: ab_glyph skaliert `PxScale` nach Ascent − Descent, nicht nach dem Em;
„13 px“ waren bei Outfit etwa 10,3 px Em, Versalien 9,1 px. Und Grundlinie
und Textanfang lagen auf Bruchteilen eines Pixels, jede waagerechte Kante
verschmierte über zwei Reihen. Abhilfe: die Schriftskala als ganze
Versalhöhen (`TextStyle::cap`: Titel 14, Label und Heading 10, Hint 9, Key 8),
`Fonts::size` rechnet daraus die Größe für die geladene Schrift (Outfit etwa
20 / 14,3 / 12,9 / 11,4 px Em); Grundlinie und Textanfang auf ganze
Gerätepixel. `CTL_H` 30 minus Versalhöhe 10 ist gerade, die Zentrierung bleibt
exakt. Icons neben Text 13 → 14 px (= Nenngröße der Schrift).

Code: `app/layout.rs` (`reshape`, `nav_open`, Zeilen-Federn, Geometrie mit
`now`, `AT_REST` für Ruhe-Layouts), `app/mod.rs` (Felder, `resize`, `show`),
`app/draw.rs` (Navigation, Daumen), `app/fades.rs` (`Current`, `set`),
`ui/widgets/segment.rs`, `stepper.rs`, `text_field.rs`. Tests: Navigation und
Stapeln halb und am Ende, Reduced Motion springt, Seitenwechsel stapelt ohne
Bewegung, Segment-Breiten stetig (1-px-Schritte, Sprung ≤ 1 px), Stepper-Knopf
blendet. Gates unverändert im Ruhezustand.

### Phase 4g: Navigation gleitet wieder (26.10., Meldung des Nutzers)

Meldung: die Animation von Icons und Text in der Seitenleiste wirkt nicht wie
eine Feder. Befund: seit 4e rundet `draw_nav` die Icon-x auf ganze Pixel
(`draw.rs`); beim Einklappen legt das Icon nur 5 px zurück (x 20 → 15), also
5 Stufen in 300 ms statt einer Bewegung, und der Name hängt am Icon und
springt mit. Die Regel aus 4e gilt für Text und was ihn trägt, nicht für
Formen, die allein kurze Wege gehen.

Fix: Icon-x und -Größe folgen der Feder ohne Runden (Vektor, in Ruhe trotzdem
ganze Pixel: offen 20, zu 15); der Name bleibt an seiner offenen Stelle
(ganze Pixel), blendet aus und wird von der schmaler werdenden Fläche
abgeschnitten, statt mitzurutschen. Geometrie in `App::nav_icon` und
`App::nav_text_x` (layout.rs), damit Zeichnen und Test dieselbe nutzen. Test:
alle 5 ms über das Einklappen mindestens 20 verschiedene Icon-x, monoton,
Name-x konstant und ganzzahlig. `/ui`-Skill: Ergänzung „kurze Wege nicht auf
ganze Pixel runden, nur Text“.

### Phase 4h: Nichts wandert ein paar Pixel (26.10., Wunsch des Nutzers)

Meldung: auch gleitend wirken die rund 4 px, die die Navigations-Icons beim
Einklappen wandern, auf der Kurvenlänge unsauber („stottert“); sie sollen so
ausgerichtet sein, dass sie sich gar nicht bewegen müssen. Regel des Nutzers
für die Zukunft: für ein paar Pixel keine Bewegung, weder animiert noch
sprunghaft.

Lösung ohne Regelbruch: eingeklappt ist die Fläche 30 × 30 (`NAV_NARROW_W`
46 − 2 · `GAP_S`), quadratisch wie ein Icon-Knopf, das 14-px-Icon mittig, 8 px
vom Rand; ausgeklappt sitzt das Icon ebenfalls 8 px eingerückt, gleich seinem
Abstand oben und unten ((`CTL_H` − `ICON`) / 2). Das Icon steht in beiden
Zuständen auf x 16 in 14 px, der Name ab x 38 (`GAP_S` hinter dem Icon)
blendet nur aus. Die Größe 16 eingeklappt entfällt (sonst wandern die Kanten).

Gleiche Regel, angewandt: der Hub von 6 px beim Einblenden einer Seite
(`PAGE_RISE`) entfällt, die Seite blendet nur. Bleiben: Schalter-Knopf (16 px,
die Bedeutung des Controls), Auswahlflächen (ein Eintrag weit und mehr),
Stapeln (Dutzende Pixel), Scroll.

Tests: Icon-x und -Größe der Navigation sind über das ganze Einklappen gleich;
Seite ohne Hub (der bisherige Test „fades in and rises“ wird zu „fades in“).
Icon-Gate der Navigation: Abstand Icon → Name = `GAP_S`.

### Phase 4i: Segment ohne Aufleuchten (26.10., Meldung des Nutzers)

Meldung: das Klicken auf ein Segment wirkt komisch, vielleicht das
Aufleuchten. Befund: `Segment::draw` zeichnet seine Fläche mit dem ganzen
`Look` (`surface(canvas, rect, &look)`), also hellt Hover die ganze Spur auf
und ein Druck mischt sie Richtung `SELECTED`, während die Auswahlfläche zum
neuen Teil gleitet: zwei Rückmeldungen, die eine auch auf Teilen, die nicht
gemeint sind. Fix: die Spur bleibt ruhig (Hover und Druck 0, Fokus-Kontur
bleibt); Rückmeldung geben der Teil unter dem Zeiger (Text hellt auf, wie
bisher) und die gleitende Auswahl. Test: Spur-Pixel mit Druck = ohne Druck.

### Phase 4j: Hover über langen Listen (26.10., Meldung des Nutzers)

Meldung: schnelles Hovern über die Font-Liste laggt extrem. Messung (Release,
Sonde, 330 Einträge, Liste offen, 946 × 1032): je Zeigerbewegung 1,67 ms, je
Frame 2,09 ms. Ursachen: `Dropdown::width` und `overlay` rechnen bei jedem
Aufruf die Breite aller 330 Einträge (`content_w`), mehrmals je Ereignis
(Layout neben/gestapelt, Trefferprüfung, Liste); und jede Zeigerbewegung wird
sofort verarbeitet, bei einer 1000-Hz-Maus bis zu 1000 je Sekunde.

Fix: (1) `Dropdown` merkt sich die Inhaltsbreite je Schrift (`Fonts::id`, eine
Nummer je geladener Schrift; neue Schrift, neue Rechnung). (2) Das Fenster
sammelt Zeigerbewegungen und reicht je Frame nur die letzte weiter; jede
andere Eingabe (Druck, Loslassen, Rad, Taste, Verlassen) verarbeitet die
ausstehende Bewegung zuerst, damit Hover und Klick zusammenpassen. Abnahme:
dieselbe Sonde, je Bewegung deutlich unter 0,2 ms; Test, dass die Breite
gleich bleibt und nach Schriftwechsel neu gerechnet wird.

### Phase 4k: Logo in der Seitenleiste, eingeprägt (26.10., Wunsch des Nutzers)

Wunsch: das Boltsnap-Logo unten im freien Platz der Seitenleiste, abgedunkelt
in die Ecke, „wie reingedrückt“.

Beleg: das finale Logo liegt als Vektor in `.superpowers/…/boltsnap-logo-v10-final.svg`
(Potrace, zwei Pfade, `viewBox 0 0 1952 2304`, Gruppe `translate(0,2304)
scale(0.1,-0.1)`); `.superpowers/` ist gitignored, also kommt der Pfad als
`assets/logo/boltsnap-mark.svg` ins Repo. `icons::parse` liest die
Pfad-Befehle (relativ, `c`/`l`/`m`/`z`).

Verhalten:
- Unten links in der Seitenleiste, in die Ecke angeschnitten (ein Teil liegt
  jenseits der Fensterkante), Höhe 168 px. Eingeprägt: die Form dunkler als
  die Leiste (`PRESSED` 0x0F auf 0x17; zuerst `WINDOW` 0x12, nach dem Render
  mit 1,04:1 kaum zu sehen), darüber ein 1-px-Rand noch dunkler (`SHADE`
  0x0A, Schatten der oberen Kante), darunter ein 1-px-Rand heller (`LIP`
  0x22, Licht auf der unteren Kante); Licht von oben wie überall. Kein Glow,
  kein Verlauf. Jeder Pfad wird für sich gefüllt (ein gemeinsamer Pfad mit
  gerade-ungerade hätte die Überlappung von Punkt und Körper ausgeschnitten).
- Nur bei offener Navigation (blendet mit den Namen) und wenn die Leiste unter
  den Einträgen genug Platz hat (mindestens `GAP_XL` Luft); sonst blendet es
  mit `QUICK` aus. Es wandert nie.
- Einmal je Größe und Skala in eine Pixmap gerendert, danach nur kopiert.

Tests: das Logo liegt unter dem letzten Eintrag mit Luft, fehlt bei
geschlossener Navigation und bei zu kleinem Fenster; Kontrast-Gate bleibt
(Deko, kein Text). Render angesehen.

### Phase 5a: Öffnen ohne Terminal (vorgezogen 26.10., Wunsch des Nutzers)

Die zwei Wege aus „Öffnen und Fenster“ kommen vor dem Rest von Phase 5, damit
der Nutzer das Fenster ohne Terminal öffnen kann. Das Fenster bleibt die
Vorschau aus 4b.

- **Tray:** Eintrag **Settings…** ganz unten, hinter einem Trenner (einen
  Quit-Eintrag gibt es nicht). Er startet `current_exe() settings` direkt im
  Tray-Thread über `paths::spawn_reaped`, stdio auf null; der Daemon-Loop und
  `shelf/mod.rs` (3681 Zeilen, über dem Limit) bleiben unberührt. Ein schon
  offenes Fenster hebt sich über die Einzelinstanz aus 4.4. Fehler beim Start:
  `notify("Settings: …")`. Immer aktiv, auch während einer Aufnahme.
- **Launcher:** `assets/linux/boltsnap.desktop` (Werte aus „Öffnen und
  Fenster“, dazu `Terminal=false`, `Comment=Shelf, recording and replay
  settings`), Icons `assets/linux/boltsnap-64.png`, `-128.png` und `-256.png`
  byte-gleich aus den PNG-Einträgen von `boltsnap.ico`, damit die README-Befehle
  aus einem Ordner kopieren (`assets/icons/boltsnap-app-64.png` bleibt für
  Windows liegen). Abweichung: `Categories=Graphics;` statt
  `Graphics;Utility;`, weil `desktop-file-validate` bei zwei Hauptkategorien
  warnt, dass Menüs den Eintrag doppelt zeigen.
- Installation: `.deb`-Assets nach `usr/share/applications/` und
  `usr/share/icons/hicolor/NxN/apps/boltsnap.png`, dasselbe im Flake
  (`postInstall`), Release-Tarball legt die Dateien bei (kleine Erweiterung
  gegenüber „Öffnen und Fenster“). README: Befehle für `cargo install`, mit
  `sed` auf `~/.cargo/bin/boltsnap`, weil `~/.cargo/bin` meist nicht im PATH
  der Sitzung liegt (hier gemessen: PATH des Daemons ohne `~/.cargo/bin`).
- Abnahme: Test, dass der letzte Menüeintrag „Settings…“ ist und aktiv bleibt
  während einer Aufnahme; Test, dass die Icon-Dateien 64/128/256 px groß sind
  und die `.desktop`-Datei `Exec=boltsnap settings`, `Icon=boltsnap` und
  `StartupWMClass=boltsnap` trägt; live: Eintrag im Tray und im Launcher des
  Nutzers sichtbar, Start über `.desktop` per `gio launch`/`hyprctl` öffnet
  das Fenster.
- Macht nicht: kein Schreiben der Settings, kein „Open Boltsnap…“ (Library,
  Phase 6), keine Dateien, die das Binary selbst nach `~/.local/share` legt.


### Phase 5b: Zeilen speichern, Replay-Puffer schalten (26.10., Meldung des Nutzers)

Meldung: „man kann garnicht den replay state steuern … wenn ich was toggle und
dann settings erneut oeffne ist es wieder aus“. Befund: das Fenster schreibt
noch nichts (4b-Vorschau), und eine Zeile für den laufenden Puffer gibt es
nicht, nur „Start with Boltsnap“ (`replay.autostart`).

Verhalten:
- Jede Zeile mit Key schreibt bei Änderung sofort über `set_settings` an den
  Daemon (prüft, schreibt, wendet an). Schalter, Segment, Dropdown, Stepper:
  beim Ändern. Slider: beim Ziehen höchstens alle 100 ms, der letzte Wert beim
  Loslassen immer. Textfeld: beim Verlassen oder `Enter` (wie `value()` es
  schon hält). Ein unveränderter Wert schreibt nichts.
- Neue erste Zeile der Replay-Seite **Replay buffer** (Schalter): an = der
  Puffer läuft (Status `running`, `starting` oder `stalled`), Umschalten ruft
  `start`/`stop` der Replay-Steuerung auf, abseits des Fensters, Start dauert
  Sekunden. Der Daemon meldet danach `ReplayStateChanged`, damit der Tray-Haken
  folgt, auch für `boltsnap replay start|stop` (bisher nur bei Tray-Klicks).
- Lehnt der Daemon oder Replay ab: das Fenster lädt die Werte neu (das
  Control springt auf den gespeicherten Stand) und zeigt oben auf jeder Seite
  ein Banner mit dem Grund (`ERROR`, Warn-Icon). Der nächste Erfolg nimmt es
  wieder weg.
- Daemon nicht erreichbar (`ensure_daemon` scheitert) oder `config.toml`
  kaputt: Banner mit Grund, alle Zeilen aus, kein Schreiben.
- Das Preview-Banner entfällt.

Code:
- `app/writes.rs` (neu): `Bind` (Key mit Kodierung oder Replay-Puffer),
  `Write`, Erkennung geänderter Werte nach jeder Eingabe (Vergleich mit dem
  zuletzt gemeldeten Wert, Drossel während Capture), `App::write`,
  `App::reload` (neue Seiten, gleiche Seite und Scroll, Fokus weg).
- `ui::Effect::Changed(block)`; `Row::bind`; `pages.rs` bindet jede Zeile
  (Kodierung je Key, Dropdowns aus ihrer Werteliste), `Notice` für Banner und
  gesperrte Zeilen, `Prefs::replay_running`.
- `linux/app/save.rs` (neu): ein Thread schreibt der Reihe nach
  (`call_daemon(SetSettings)` bzw. `replay::call`), Ergebnis über einen
  calloop-Kanal zurück; `status()` aus `get_settings`.
- `linux/replay/mod.rs`: `ReplayStateChanged` nach `start` und `stop`.

Tests: Schalter schreibt seinen Key, Dropdown schreibt den gelisteten Wert,
Slider gedrosselt und letzter Wert beim Loslassen, Textfeld erst beim
Bestätigen, unveränderter Wert schreibt nichts, jede gebundene Zeile kodiert
ihren Wert so, dass `settings::load::check` ihn annimmt, Replay-Schalter
liefert `Write::Replay`, `reload` behält Seite, gesperrte Zeilen schreiben
nicht. Live: eine Zeile ändern, Fenster schließen und öffnen, Wert bleibt;
Datei zeigt den Key; Replay an und aus, Tray-Haken folgt.

Macht nicht (bleibt Phase 5): `SettingsWatch` (Tray-Änderung bei offenem
Fenster erscheint erst beim nächsten Öffnen), Zurücksetzen-Icon je Zeile,
Hinweise je ungültigem Key, System-Listen (Display, Codec, Font), Ordner-Dialog,
Bitrate, „Restart replay“-Knopf.

### Phase 4d: Ruckeln (26.10., Meldung des Nutzers)

Meldung: „die animationen sind gut aber sehr sehr laggy“. Messung (Release,
Fenster 946×1032 auf 200 Hz, Seitenwechsel per Tasten, Frame-Log im Fenster):
341 Frames, Zeichnen bis Commit Median 3,4 ms, Maximum 7,1 ms, Abstände
Median 5,0 ms mit wiederholten 10-ms-Lücken (ausgefallene Frames).
`App::draw` allein 1,7 ms (Sonde: Text 63 µs je 30 Zeichen, Icon 6 µs,
Gruppenfläche 45 µs, `clear` 104 µs).

Abhilfe, je Schritt gemessen:
- P1: Canvas bleibt zwischen Frames, neu nur bei anderer Größe oder Skala.
- P2: `wl_shm` `Abgr8888` (Byte-Folge RGBA wie tiny-skia), wenn der
  Compositor es anbietet: eine Kopie statt Byte-Tausch je Pixel; sonst wie
  bisher `Argb8888`.
- P3: Glyphen-Cache in `Fonts`: Abdeckung je (Schnitt, Glyphe, Größe,
  Viertelpixel-Versatz), gezeichnet wird nur noch gemischt.
- P4: Schaden nur, wo sich etwas geändert hat: Vergleich mit dem letzten
  Frame zeilenweise, gemeldet wird das umschließende Rechteck; nichts
  geändert, kein Commit.

Abnahme: dasselbe Frame-Log, Median ≤ 1,5 ms, keine 10-ms-Lücke während einer
Animation; Tests für den Cache (gleiche Pixel wie ohne) und das
Schadensrechteck.

### Review 26.10.: Ausrichtung und UI/UX (Wunsch des Nutzers)

Grundlage: Renders aller Seiten mit Outfit bei 360, 560, 946 und 1400 px,
Skala 2 vergrößert, Stress-Fixture, alle Gates (`cargo test gate`,
`boltsnap settings --measure`), Flächen-Kontraste nachgerechnet (WCAG-Formel).

Ausrichtung, gemessen, ohne Befund: Versalien je Control 0,00 px; Icons auf
der Versalien-Mitte höchstens 0,31 px, eine Achse (0,31), eine Textkante
(0,00), `GAP_S` + 0,37 vom breitesten Icon; Gruppen 12 px Innenabstand auf
allen Seiten; Titel und erster Navigationseintrag auf einer Mittellinie
(y 39); Controls bündig rechts auf `inner.right`; Spalte bei 946 px links und
rechts 33 px Rand.

| # | Prio | Befund (Auslöser, Wirkung) | Beleg | Lösung, Abwägung |
|---|---|---|---|---|
| U1 | P1 | Tastatur-Fokus praktisch unsichtbar: fokussiert `HOVER` gegen ruhend `RAISE` | 1,11:1 nachgerechnet; `widgets/mod.rs` `surface()` | Fokus als 2-px-Kontur in `SUB` innen auf der Fläche (5,38:1 gegen `RAISE`), konzentrischer Radius, `QUICK`; kein Glow. Alternative nur Fläche `SELECTED` (1,33:1) reicht nicht. |
| U2 | P1 | Fehler-Banner kürzt lange Meldungen mit „…“, der Grund ist nicht lesbar | Fixture: „config.toml cannot be read: /home/someone/Videos/boltsnap/recordin…“ | Banner bricht um (bis 4 Zeilen, Pfade an `/` und überall), Höhe wächst mit; Layout bekommt die Höhe aus dem Umbruch. |
| U3 | P1 | Bei 360 px werden Namen und Hinweise abgeschnitten (`/ui`: nichts horizontal abschneiden) | „Show saved videos on th…“, „Outline around a recorded r…“ in `recording-360` | Name und Hinweis brechen in zwei Zeilen um, Zeilenhöhe folgt; Gates prüfen „kein …“ bei 360. |
| U4 | P2 | Hinweise wiederholen sich: „Applies to the next recording“ 2×, „Applies when replay restarts“ 3× (`/ui`: keine Erklärsätze unter jeder Zeile) | `pages.rs` | Recording-Hinweis nur während einer Aufnahme (so stand es in „Zustände und Randfälle“); Replay-Hinweis einmal, und bei laufendem Puffer nach einer Änderung ein Knopf „Restart replay“ (Plan „Aufbau“). |
| U5 | P2 | Klick auf den Zeilennamen tut nichts, nur das Control ist Trefferfläche | `App::target_at` | Ganze Zeile ist Trefferfläche: Schalter schaltet, andere Controls bekommen Fokus; Hover hellt den Namen auf. |
| U6 | P2 | Length-Slider 5 bis 600 s linear, 119 Schritte auf 225 px; übliche 30 bis 120 s liegen im ersten Fünftel, „60 s“ statt „1 min“ | `replay-946` | Dropdown mit Stufen (15 s, 30 s, 1 min, 2 min, 5 min, 10 min) wie bei „Keep long shelf saves“; ein krummer Wert aus der Datei bleibt als eigener Eintrag (`choose`). |
| U7 | P2 | Ob die Länge in den Speicher passt, sieht man nicht | Plan „Aufbau“ verlangt es | Hinweis unter Memory „Holds about 75 s at 60 fps“, gerechnet wie `replay::settings`. |
| U8 | P2 | Tray kann „Default monitor“ setzen, das Fenster nicht; Codec und Font fehlen | Tray-Menü, Tabelle „Aufbau“ | Rest von Phase 5 (Display-Dropdown aus den Monitoren, Codec aus `gsr --info`, Font aus fontconfig). |
| U9 | P3 | Control-Spalte ausgefranst: Breiten 90 bis 371 px, linke Kanten springen von Zeile zu Zeile | `recording-946` | Dropdowns bekommen eine gemeinsame Mindestbreite (Token, z. B. 120), Segmente bleiben nach Inhalt; Abwägung: etwas mehr Leerraum in kurzen Dropdowns. |
| U10 | P3 | Bei 1400 px schwebt die Spalte mittig, 260 px Abstand zur Navigation | `recording-1400` | Spalte links an der Navigation (`GAP_XL`), höchstens 680 breit, Rest rechts frei; Abwägung: libadwaita zentriert, macOS nicht; Geschmacksfrage. |
| U11 | P3 | Corner-Segment mit Pfeilen liest sich als Richtung, nicht als Ecke | `shelf-946` | Vier kleine Positions-Piktogramme (Bildschirm-Rahmen mit Punkt in der Ecke), eigenes Sprite-Symbol. |
| U12 | P3 | Dünne Seiten: Screenshots 1 Zeile, Shelf 3 | Renders | Produktfrage: zusammenlegen oder um erwartete Einstellungen ergänzen (Format, Kopieren in die Zwischenablage, Dateiname). |
| U13 | P3 | Kein Accessibility-Baum (AT-SPI), Screenreader lesen das Fenster nicht; schmale Navigation ohne Namen | eigenes Toolkit | Groß, eigenes Vorhaben (AccessKit); bis dahin als Lücke im README. |

Kein Befund, bewusst so: ausgeschalteter Schalter (Spur 1,13:1 gegen die
Gruppe, Knopf 5,38:1 trägt die Form), Seitenleiste gegen Seite 1,04:1 (ruhig
gewollt), drei linke Achsen (Gruppenkante, Icon, Name) wie in Systemeinstellungen.

Umsetzung nach Wahl des Nutzers; je Punkt eigener Commit, Gates und Renders
danach.

### Phase 5c: Umsetzung des Reviews U1 bis U13 und der offenen Phase-5-Teile

Freigegeben zum Planen am 26.10. Belege für jede genutzte Schnittstelle stehen
beim Schritt. Jeder Schritt ist ein Commit mit `cargo fmt --check`, clippy,
`cargo test`, den Gates und einem Render; live wo angegeben (Tasten über
`hl.dsp.send_shortcut`, Config vorher sichern und byte-gleich zurück).

Entscheidungen des Nutzers vor dem jeweiligen Schritt: D1 Ohne-Animation-Liste
(4b-Vorschlag), D2 Spalte bei breitem Fenster (U10), D3 dünne Seiten (U12),
D4 AccessKit ja/nein (U13). Bis zur Antwort bleibt der heutige Stand.

#### S1 Puffer im Shelf (4f-Folge; der Selector macht es schon richtig)

- Erst messen wie in 4f: CPU des Daemons und Commit-Abstände beim Erscheinen
  und Entfernen einer Karte (Karte per `boltsnap full`, Entfernen per
  Socket-Befehl), Selector beim Ziehen eines Bereichs nicht skriptbar, dort nur
  Code-Beleg.
- Befund bestätigt: wie im Fenster bis zu drei `Buffer` je Größe behalten und
  reihum nutzen (`slot::Buffer::canvas` liefert `None`, solange der Compositor
  ihn hält, sctk 0.19.2 `shm/slot.rs:424`); Schaden nur im geänderten Bereich
  ist dort ein eigener Schritt, nicht Teil von S1.
- Dateien: `shelf/mod.rs` (über dem Limit, nur der Puffer-Teil wird in
  `shelf/surface.rs` ausgelagert statt die Datei wachsen zu lassen),
  `select_skia/mod.rs`.
- Macht nicht: Shelf-Zeichnen selbst.

##### S1 im Detail (Gegenprüfung am Code, 26.10.)

Befund:
- Shelf: `Daemon::draw` (`shelf/mod.rs:1986`) holt jeden Frame einen neuen
  Puffer (`self.pool.create_buffer`, `:2030`); während Erscheinen und
  Entfernen läuft das im Bildschirmtakt (Frame-Callback `:2060`).
- Selector: schon richtig, drei Puffer je Größe, freie werden genutzt
  (`select_skia/mod.rs:615-640`). Der Verdacht aus 4f-Folge war dort falsch.
- Popup und Rahmen der Aufnahme (`:2311`, `:2355`): zeichnen nur bei
  Ereignissen (Timer, Hover), nicht im Takt; bleiben, wie sie sind.
- `draw_shelf` leert den Puffer zuerst (`shelf/paint.rs:596`), ein
  wiederverwendeter Puffer zeigt also kein altes Bild.

Umsetzung:
- `linux/buffers.rs` (neu, linux/ hat dann 11 Dateien): `Buffers` hält bis zu
  drei `slot::Buffer` einer Größe; `next(pool, w, h, stride, format)` gibt
  einen freien zurück oder legt einen an (der älteste geht, sobald mehr als
  drei da sind); andere Größen fallen weg. Tests ohne Compositor sind nicht
  möglich (ein `SlotPool` braucht `wl_shm`); die Auswahl-Logik als reine
  Funktion über „frei/belegt, Größe“ wird getestet.
- Fenster (`app/window.rs`) und Shelf nutzen `Buffers`; im Shelf ersetzt eine
  Zeile den `create_buffer`-Aufruf, das Feld kommt dazu, beim Abbau der
  Layer-Fläche gehen die Puffer mit.

Abnahme: Code-Beleg (oben) plus die Messung aus 4f, die denselben
Mechanismus im Fenster belegt. Eine Live-Messung im Shelf braucht
Daemon-Neustarts, Screenshots und startet mit `replay.autostart` jedes Mal den
Replay-Puffer; der erste Versuch (26.10., 16:44) hat den Desktop des Nutzers
spürbar ausgebremst und wurde abgebrochen (`WAYLAND_DEBUG` kam nicht an, der
Daemon leitet stderr um). Live nur noch nach Absprache mit dem Nutzer, zu
einer Zeit, die er nennt: Karte erscheinen lassen, `hyprctl layers` und das
Gefühl beim Erscheinen.

Macht nicht: Schaden im Shelf nur im geänderten Bereich, Popup, Rahmen.

#### S2 Sichtbarer Fokus (U1)

- Fokussiertes Control: 2-px-Kontur in `SUB` innen auf seiner Fläche, Radius
  konzentrisch (`RADIUS − 1`), Stärke = Fokus-Fade (`QUICK`); Navigation
  genauso auf der Auswahlfläche. Hover und Druck bleiben Flächen-Helligkeit.
- Segment: Kontur um das ganze Segment; Slider: um den Knopf-Halo; Stepper:
  um das ganze Control.
- Gate: Kontrast Kontur gegen `RAISE` ≥ 3:1 (5,38:1) neu in `gate_contrast`.
- Dateien: `widgets/mod.rs` (`focus_ring`), alle Widgets, `draw.rs`.

##### S2 im Detail (Gegenprüfung am Code, 26.10.)

Befund: Fokus hebt heute nur die Fläche an, `RAISE` → `HOVER`
(`Look::surface`, `widgets/mod.rs:46`, wie Hover), 1,11:1; der Slider zeigt
Fokus über seinen Halo (`slider.rs:109`), die Navigation über die Fläche der
Auswahl (`draw.rs`, `fill = mix(RAISE, HOVER, focus)`). Es gibt keine
Kontur-Funktion im Canvas.

Abweichung von S2 oben: die Kontur liegt **außen** um die Fläche, nicht innen.
Innen läge sie bei einem eingeschalteten Schalter auf der hellen Spur (`SUB`
gegen `TEXT` nur 2,3:1); außen liegt sie überall auf der Gruppe (`SUB` gegen
`PANEL` 5,9:1) bzw. der Seitenleiste. Platz: Controls sind 30 hoch in 46er
Zeilen, die Navigationsflächen haben 2 px bis zum Nachbarn, die Gruppe 12 px
Innenabstand; die Kontur (2 px) passt überall, ohne etwas zu überdecken.

Umsetzung:
- `Canvas::ring(rect, radius, width, color, alpha)`: zwei abgerundete
  Rechtecke, gerade-ungerade gefüllt.
- Token `FOCUS_W = 2`. `widgets::focus_ring(canvas, rect, radius, look)`:
  Kontur um `rect.inset(-2)` mit Radius + 2 (konzentrisch), Farbe `SUB`,
  Deckkraft = Fokus-Fade (`QUICK`).
- `Look::surface` hebt nur noch für Hover; `surface()` zeichnet die Kontur mit.
  Schalter: um die Spur; Slider: um den Knopf, Halo nur noch Hover und Druck;
  Navigation: um die Auswahlfläche.
- Gate Kontrast: Paare „focus ring“ `SUB` auf `PANEL` und auf `SIDEBAR`.
- Test: fokussiertes Dropdown hat 1 px außerhalb seiner Kante Kontur-Pixel,
  ohne Fokus nicht; Fade-Mitte halb.

- Nur bei Tastatur-Fokus (wie `:focus-visible`): `App` merkt sich, ob der
  Fokus zuletzt per Taste kam (Tab, Pfeile in der Navigation) oder per Klick;
  `Look::ring` = Fokus-Fade nur im ersten Fall, sonst 0. Ein Klick nach
  Tastatur-Fokus blendet die Kontur mit `QUICK` aus. Test: Tab zeigt die
  Kontur, Klick auf dasselbe Control nicht.

#### S3 Umbruch statt Kürzen (U2, U3)

- `text.rs`: `wrap(style, text, width) -> Vec<String>` (Wortgrenzen, in Pfaden
  auch nach `/`, notfalls mitten im Wort), mit Tests.
- Banner: bis 4 Zeilen, Höhe = Zeilen × `LINE` + 2 · `GAP_M`, Icon auf der
  ersten Zeile; erst danach „…“.
- Zeilen: Name und Hinweis brechen um, wenn sie neben dem Control nicht
  passen und die Zeile schon gestapelt ist oder der Name unter `LABEL_MIN`
  fiele; Zeilenhöhe aus der Zeilenzahl, weiter Federn beim Stapeln (die Höhe
  geht in `lerp`, gerundet).
- Gates: bei 360 px kein „…“ in Namen, Hinweisen, Bannern der echten Seiten;
  Fixture-Pfad bleibt mittig gekürzt im Textfeld (das ist ein Feld, kein
  Text).
- Dateien: `text.rs`, `layout.rs` (Höhen je Block), `draw.rs`,
  `widgets/button.rs` (`draw_banner`), `tests/gates.rs`.
- Risiko: Höhen hängen nun am Text; `reshape` muss mit der Zielbreite
  rechnen (wie heute), Tests für Zeilenhöhen bei 360 und 1000.

##### S3 im Detail (Gegenprüfung am Code, 26.10.)

Befund: Banner zeichnet eine Zeile mit `draw_in` (kürzt mit „…“,
`widgets/button.rs:69`), Höhe fest `ROW_H` (`layout.rs:199`); Zeilen haben
feste Höhen `ROW_H`/`STACKED_H`, Name und Hinweis je eine Zeile `LINE` 16 /
`HINT_LINE` 14 (`layout.rs:268`), gekürzt über `draw_in` in `draw.rs`.

Verhalten:
- `Fonts::wrap(style, text, width, max_lines)`: gierig nach Wörtern; ein
  Wort, das allein nicht passt, bricht nach `/`, `-`, `_` oder notfalls
  zwischen Zeichen; mehr Zeilen als `max_lines`: die letzte endet mit „…“.
- Banner: bis 4 Zeilen, Zeilenabstand 20 (Versalhöhe 10, `(20 − 10) / 2`
  ganzzahlig), Höhe `2 · GAP_M + max(22, n · 20)` (eine Zeile wie heute 46),
  Icon auf der Mitte der ersten Zeile.
- Zeile: Name bis 3 Zeilen, Hinweis bis 3 Zeilen, in der Breite, die ihnen
  neben dem Control bleibt (gestapelt: volle Breite). Zeilenabstand eine
  Zeile 16 / 14 wie heute, ab zwei Zeilen 18 / 16 (Versalien 10 / 9, Abstand
  bis zur nächsten Versalie ≥ 4 px). Textblock `tb` = Summe der Zeilen.
  Höhe neben dem Control `max(ROW_H, tb + 2 · GAP_S)`, gestapelt
  `max(CTL_H, tb) + CTL_H + 3 · GAP_S`; die Feder beim Stapeln mischt jetzt
  zwischen diesen zwei Höhen je Zeile.
- `RowParts`: `text` (der ganze Textblock, bisher `label_block`), `label` und
  `hint` (die erste Zeile je Art, für Gates und Messung), `names`, `hints`
  (die umbrochenen Zeilen).
- Überschriften und Navigationsnamen bleiben einzeilig (kurz, eigene
  Strings); Textfeld kürzt weiter mittig (ein Feld, der volle Wert steht
  beim Bearbeiten da).

Gates und Tests: neues Gate „kein ‚…‘ in Namen, Hinweisen und Bannern“ bei
360/480/760/1400 über alle echten Seiten und den Fehlerzustand; Innenabstand-
und Achsen-Gate nutzen `text` statt `label` für die Oberkante gestapelter
Zeilen; `wrap`-Tests (Wörter, Pfad, Kompositum, `max_lines`); Zeilenhöhe mit
zwei Namenszeilen.

Risiko: alle Höhen hängen jetzt am Text, `layout_at` rechnet den Umbruch je
Frame (rund 20 Zeilen, Breiten aus dem Glyphen-Cache); Gates fangen Überlauf
und Achsen.

#### S4 Ganze Zeile klickbar (U5)

- `target_at`: die Zeile (ohne Hinweis-Unterkante) zählt als Ziel ihres
  Controls. Klick auf Name/Icon: Schalter schaltet, alle anderen bekommen
  Fokus (Dropdown öffnet nicht, Slider springt nicht). Hover über der Zeile
  hellt den Namen auf (`SUB` → `TEXT` beim Icon, `QUICK`).
- Test: Klick auf den Namen von „Show frame“ schreibt den Key, auf den Namen
  von „Frame rate“ fokussiert nur.

##### S4 im Detail (Gegenprüfung am Code, 26.10.)

Abweichung von S4 oben: nur Zeilen mit Schalter werden ganz klickbar (wie das
Label einer Checkbox). Ein Klick auf den Namen eines Dropdowns oder Sliders
gäbe nur Fokus, und der ist nach Mausklicks seit S2 unsichtbar: ein Klick ohne
sichtbare Wirkung wäre schlechter als keiner.

Umsetzung:
- `Widget::row_toggles(&self) -> bool` (Standard `false`, Schalter `true`) und
  `Widget::toggle(&mut self, now) -> Outcome` (Schalter: wie `Space`).
- `App::target_at`: liegt der Zeiger in der Zeile, aber nicht auf dem
  Control, ist die Zeile Ziel ihres Controls, wenn es `row_toggles`; sonst wie
  heute nichts. Hover hellt so auch den Schalter auf, wenn der Zeiger auf dem
  Namen ist (die Fläche zeigt, was ein Klick tut).
- `App::press`: Ziel-Control, Punkt außerhalb seines Rechtecks → `toggle`,
  Fokus wie bei einem Klick auf den Schalter.
- Tests: Klick auf den Namen „Show frame“ schaltet und schreibt den Key; Klick
  auf den Namen „Frame rate“ ändert nichts und schreibt nichts; Hover auf dem
  Namen „Show frame“ hellt den Schalter auf.

#### S5 Einheitliche Dropdown-Breite (U9)

- Token `MENU_MIN = 120` (Raster 4): Dropdowns mindestens so breit, Text
  links, Chevron rechts wie heute. Segmente und Felder unverändert.
- Gate: linke Kanten aller Dropdowns einer Seite gleich, wenn ihr Inhalt
  passt.

##### S5 im Detail (Gegenprüfung am Code, 26.10.)

Befund: `Dropdown::width` nimmt den breitesten Eintrag + Ränder + Haken
(`dropdown.rs:81`); die echten Listen ergeben 90 (Frame rate 60 fps), 93
(Keep long shelf saves), 102 bis 104 (Frame rate, Memory). Umsetzung: Token
`MENU_MIN = 120` in `theme.rs`, `width = max(Inhalt, MENU_MIN)`; Text links,
Haken rechts wie heute. Gate `gate_menus_share_a_left_edge`: auf jeder echten
Seite haben alle Dropdowns neben ihrem Namen, deren Inhalt in `MENU_MIN`
passt, dieselbe linke Kante (bei 760 und 1400 px).

#### S6 Live-Werte: `SettingsWatch` und `Widget::set` (Voraussetzung für S7, S8)

- `ipc.rs`: `watch_settings()` wie `watch_recording()` (`ipc.rs:187`); ein
  Thread liest Zeilen (`state_line`, `shelf/settings.rs:117`) und schickt den
  Stand über einen calloop-Kanal.
- `Widget::set(value, now)`: Schalter, Segment, Dropdown, Stepper gleiten mit
  ihren Federn zum neuen Wert, Slider und Feld springen (Ohne-Animation D1);
  ein Feld mit Fokus und ein gezogener Slider behalten den Wert des Nutzers.
- Fenster: neue Seiten aus dem Stand bauen; gleiche Struktur → Werte per
  `set`, sonst `reload`. Ersetzt das bisherige `prefs::load()` aus der Datei
  durch den Stand des Daemons.
- Tests: Tray-Änderung bei offenem Fenster kommt an; gezogener Slider wird
  nicht überschrieben.

##### S6 im Detail (Gegenprüfung am Code, 26.10.)

Befund: der Daemon schickt Beobachtern nach jedem eigenen Schreiben
(`shelf/settings.rs:175`) und jeder Dateiänderung (`shelf/mod.rs:2873`) eine
JSON-Zeile mit `State` (`shelf/settings.rs:179`), nur wenn sich etwas
geändert hat. `State` enthält nur die in der Datei gesetzten Werte, die
Defaults stecken in `Config`.

Abweichung von S6 oben: die Werte kommen weiter aus `prefs::load()`
(`Config::load()` liest dieselbe Datei, die der Daemon schreibt, mit allen
Defaults); die Watch-Zeile ist der Auslöser und liefert `broken`. Ein eigener
Umbau `State` → `Prefs` würde die Default-Logik von `Config` doppeln.

Verhalten:
- `ipc::watch_settings()` wie `watch_recording()`; ein Thread im Fenster liest
  Zeilen und schickt je Zeile ein Signal über einen calloop-Kanal; endet der
  Strom (Daemon weg), versucht er es alle 2 s neu und meldet „not running“
  über die Notice (Zustandstabelle „Daemon stirbt“).
- Auf ein Signal: Notice neu (`save::notice()`), Seiten neu bauen,
  `App::update(pages, now, busy)`. Gleiche Struktur (Blöcke gleicher Art,
  Zeilen mit gleichem Namen und gleichen Einträgen) → je Zeile mit anderem
  Wert `control.set(value, now)`, Hinweis und `enabled` übernommen, der
  gemeldete Wert (`sent`) gleich mitgesetzt, damit nichts zurückgeschrieben
  wird. Sonst `reload`.
- Übersprungen werden: die fokussierte und die gezogene Zeile, und Zeilen mit
  einem Schreibauftrag in der Schlange (`busy(block)`; das Fenster zählt
  Aufträge je Block bis zur Antwort). So springt ein Slider nach dem Ziehen
  nicht auf einen älteren Zwischenwert zurück.
- `Widget::set(&Value, now)`: Schalter (Knopf-Feder `SNAP`), Segment (Gleiten
  wie bei Wahl), Dropdown (Markierung gleitet), Stepper (Knöpfe blenden),
  Slider und Textfeld springen (Ohne-Animation: Zahl und Text, D1 vom Nutzer
  bestätigt 26.10.).
- Replay-Zustand (läuft/steht) kommt nicht über diesen Strom; er wird bei
  jedem Signal und beim Öffnen gelesen (`save::replay_state`), ein Umschalten
  im Tray erscheint daher beim nächsten Signal, nicht sofort (bleibt offen).

Tests: `update` mit geändertem Schalterwert gleitet und schreibt nichts
zurück; fokussiertes Feld und gezogener Slider bleiben; `busy`-Zeile bleibt;
andere Struktur → `reload`; je Widget `set`.

#### S7 Hinweise nur wenn sie stimmen, „Restart replay“ (U4)

- `watch_recording()` im Fenster; „Applies to the next recording“ nur bei
  laufender Aufnahme, an allen Recording-Zeilen mit `NextRecording`
  (Schema `Applies`).
- Replay: kein Hinweis je Zeile. Läuft der Puffer und wurde ein
  `ReplayRestart`-Key geändert, erscheint unter der Gruppe eine Zeile
  „Changes apply after a restart“ mit Knopf **Restart replay** (`Write::Restart`
  = `stop`, dann `start` im Schreib-Thread); verwirft den Puffer, deshalb nie
  automatisch.

##### S7 im Detail (Gegenprüfung am Code, 26.10.)

Befund: „Applies to the next recording“ steht fest an Frame rate und Audio,
„Applies when replay restarts“ an Length, Memory, Frame rate (`pages.rs`).
Der Recording-Watch schickt bei jeder Änderung eine Zeile mit `state`
(`shelf/mod.rs:1433`, `RecordingSnapshot`, `protocol.rs:101`), auch für die
Laufzeit; der Replay-Status liefert `state`, `session`, `seconds`,
`budget_bytes` (= `memory_mib · 2²⁰ / 3`, gemessen 178956970 bei 512 MiB).

Abweichung von S7 oben, Aufnahme: statt den Hinweis an sechs Zeilen zu hängen
(wieder Rauschen) steht während einer Aufnahme oben auf der Recording-Seite
ein Banner „A recording is running, changes here apply to the next one“ (Icon
`info`, `SUB`); die festen Zeilenhinweise entfallen. Show frame, Video folder
und Show saved videos wirken sofort und sind davon nicht betroffen.

Replay:
- Die drei festen Hinweise entfallen.
- Veraltet ist der laufende Puffer, wenn `seconds` oder `budget_bytes` nicht
  zur Datei passen, oder wenn in dieser Puffer-Sitzung (`session`) ein
  anderer `ReplayRestart`-Key (Frame rate, Encoder, Output, Bitrate)
  erfolgreich geschrieben wurde (im Fenster gemerkt je Sitzung; ein Neustart
  hat eine neue Sitzung und ist damit wieder aktuell).
- Dann steht in der Buffer-Gruppe unter „Replay buffer“ die Zeile **Restart
  replay** (Icon `rotate-ccw`, Hinweis „Applies your changes, the buffer
  starts empty“, Icon-Knopf). Klick: `Write::Restart` = `stop`, dann `start`
  im Schreib-Thread; nie automatisch.

Fenster: ein zweiter Watch-Thread auf `watch_recording()`, der nur bei
geändertem `state` signalisiert; `Prefs` bekommt `recording` (aus
`RecordingStatus`), `replay_stale`.

Tests: Banner nur mit `recording`; keine festen Hinweise mehr; Restart-Zeile
nur mit `replay_running && replay_stale`, ihr Knopf liefert
`Write::Restart`; Copy-Gate über die neuen Strings.

#### S8 Zurücksetzen je Zeile

- Steht der Key in der Datei (`State::doc`), zeigt die Zeile links vom
  Control einen Icon-Knopf `rotate-ccw` (quadratisch `CTL_H`); sein Platz
  öffnet und schließt mit `SNAP`. Klick: `Change::Remove`, Wert springt über
  S6 auf den Default.

##### S8 im Detail (Gegenprüfung am Code, 26.10.)

Abweichung von S8 oben (Regel „Keine Mini-Wege“, 4h): kein Platz, der sich
links vom Control öffnet (das Control spränge 38 px). Der Knopf sitzt im
Textbereich: neben dem Control rechts an dessen Ende (`GAP_S` vor dem
Control), gestapelt rechts auf der Namenszeile; die Textbreite ist um
`CTL_H + GAP_S` kleiner, solange er da ist. Er blendet mit `QUICK` ein und
aus (Kanal `Shown`), das Control bewegt sich nicht.

Belege: welche Keys in der Datei stehen, liefert `settings_state().doc`
(`config.rs`, auch der Daemon nutzt es); ein Reset ist `Change::Remove`
(`settings/write.rs:15`), der Daemon setzt den Default.

Umsetzung:
- `Prefs::set_keys`; `Row::reset` (der Key steht in der Datei, nur für
  `Bind::Key`).
- `Target::Reset(block)`: eigener Hover, Druck, Fokus; Tab-Reihenfolge Name
  → Reset → Control (Lesereihenfolge links nach rechts).
- `RowParts::reset: Option<Rect>`; gezeichnet als Icon-Knopf `rotate-ccw`
  (Fläche wie Controls, Fokus-Kontur aus S2), Deckkraft = `Shown`-Fade; Treffer
  nur, solange er da ist.
- Klick, `Space`, `Enter` → `Effect::Reset(block)` → `Write::Reset(key)` →
  `Change::Remove`; der neue Wert kommt über S6 zurück und gleitet.
- `update`: `reset` übernehmen, `Shown` blendet; `reload`/neu: sofort.

Tests: Reset nur für Keys in der Datei; Klick liefert `Write::Reset` mit dem
Key; Tab-Reihenfolge; Control-Rechteck mit und ohne Reset gleich; Gates im
Zustand „alle Keys gesetzt“ (Überlauf, Umbruch, Abstände, Achsen, Kontrast).

#### S9 Replay-Länge und Qualität (U6, U7)

- Length als Dropdown 15 s, 30 s, 1 min, 2 min, 5 min, 10 min (`choose`, ein
  anderer Wert aus der Datei bleibt als Eintrag).
- Unter Memory: „Video at about N Mbit/s“ aus `replay::settings::video_kbps()`
  (`src/replay/settings.rs:37`, füllt den Puffer mit der Länge); mit fester
  `bitrate_kbps` stattdessen „Holds about N s“.

##### S9 im Detail (Gegenprüfung am Code, 26.10.)

Befund: `replay.duration_seconds` 1 bis 600 (`schema.rs:134`), Length ist ein
Slider 5 bis 600 in 5er Schritten (`pages.rs`). `Settings::video_kbps`
(`replay/settings.rs:37`) füllt ohne feste Bitrate den Live-Drittel des
Speichers mit `seconds + 1` Sekunden, 90 %, minus Ton (128 kbit/s), geklemmt
auf 2 bis 80 Mbit/s. Abweichung vom Plan oben: wegen der unteren Klemme passt
die Länge nicht immer (10 min bei 256 MiB: 2 Mbit/s erzwungen, rund 336 s
passen); der Hinweis sagt deshalb normal die Qualität, im Engpass was passt.

Verhalten:
- Length: Dropdown 15 s, 30 s, 1 min, 2 min, 5 min, 10 min (`choose`, ein
  anderer Wert aus der Datei bleibt als eigener Eintrag, z. B. 45 s).
- Hinweis unter Memory, aus denselben Formeln wie der Recorder:
  `kbps = video_kbps()`, passt = `memory · 2²⁰ / 3 · 8 / ((kbps + 128) · 1000)`
  Sekunden minus die eine Reserve-Sekunde. Passt die Länge: „Video at about
  N Mbit/s“ (unter 10 mit einer Nachkommastelle, geschütztes Leerzeichen);
  sonst „Only about T fit, give it more memory“ (T wie die Dropdown-Einträge:
  unter 2 min in Sekunden, sonst ganze Minuten).
- `Prefs::replay_bitrate` (aus `replay.bitrate_kbps`).
- Der Hinweis folgt sofort: eine Änderung schreibt, der Watch (S6) baut die
  Seite neu, `update` übernimmt den Hinweis.

Tests: Stufen und eigener Wert aus der Datei; Hinweis 60 s/512 MiB ist
„Video at about 21 Mbit/s“ (gerechnet 20 994 kbit/s, derselbe Wert, mit
dem der laufende Replay-Recorder gestartet ist: `-q 20994`); 600 s/256 MiB nennt, was passt; feste Bitrate wird
genommen; Copy-Gate über die Hinweise.

#### S10 Fehlende Zeilen (U8)

- Recording: Display (Dropdown „Focused display“, je Monitor aus
  `OutputState::info` (`name`, `description`, sctk `output.rs:175`), „Both
  displays“, ein konfigurierter, abgesteckter als „… (disconnected)“, Logik
  wie `tray::default_targets`); Codec (Auto + `gsr::info().codecs`,
  `gsr.rs:74`, im Hintergrund, 0,5 s).
- Neue Seite General: Font (`fc-list : family`, gibt hier 1877 Familien,
  Dropdown mit Suche durch Tippen, „Desktop font“ zuerst).
- Dropdown mit vielen Einträgen: Liste scrollt in sich (Rad, Pfeile),
  höchstens 10 sichtbar.

##### S10 im Detail (Gegenprüfung am Code, 26.10.)

Aufgeteilt, weil der Font eine scrollende Liste mit Tipp-Suche braucht:
S10a Display und Codec, S10b Seite General mit Font.

S10a, Belege:
- Monitore: der Tray nimmt Waylands `OutputState` (`shelf/mod.rs:1144`,
  `name`, `description`); das Fenster hat einen eigenen `OutputState`
  (`app/window.rs`, `Parts::outputs`) und meldet Monitore beim Erscheinen
  und Gehen (`OutputHandler`), dann Neuaufbau. Beschriftung wie im Tray
  (`tray::monitor_label`, wird als reine Funktion über Name und Beschreibung
  geteilt), dazu „Focused display“, „Both displays“, ein konfigurierter, nicht
  angeschlossener als „… (disconnected)“ (Logik wie `tray::default_targets`).
  Werte: `focused`, `both`, `output:<name>` (`schema.rs`, `Kind::Target`).
- Codec: `record_codec` nimmt `auto`, `libx264`, `h264`, `hevc`, `av1`,
  `*_vulkan` (`gsr::choose`, `gsr.rs:112`); HDR- und 10-Bit-Einträge aus
  `gsr --info` lehnt `choose` ab und erscheinen nicht. Hier verfügbar
  (gemessen): h264, h264_software, hevc, av1, h264_vulkan, hevc_vulkan …
  Beschriftung: Auto, H.264, HEVC, AV1, „H.264 via Vulkan“, „H.264 on the CPU“
  (`libx264`); ein Wert aus der Datei, der nicht in der Liste ist, bleibt als
  eigener Eintrag. `gsr --info` dauert rund 0,5 s: einmal im Hintergrund beim
  Start, bis dahin nur Auto und der Wert aus der Datei, dann Neuaufbau.
- Icons: Display `monitor`, Both displays neu `columns-2`, Codec neu `film`
  (aus `lucide-static`, ins Sprite).
- Copy-Gate-Allowlist: „Vulkan“, „CPU“.

S10b (eigener Schritt): Seite General mit Font (`ui_font`, „Desktop font“
zuerst, Familien aus `fc-list : family`, 11 ms gemessen); Dropdown-Liste
höchstens 10 Einträge hoch, scrollt in sich (Rad, Pfeile, Markierung bleibt
sichtbar), Tippen springt zum ersten passenden Eintrag; das Fenster lädt
seine Schrift neu, wenn `ui_font` sich ändert.

S10b im Detail (Gegenprüfung am Code, 26.10.):
- Befund: die Liste zeichnet alle Einträge untereinander
  (`dropdown.rs:169`), so breit wie das Control; das Rad ignoriert die App
  bei offener Liste; Tippen geht nur an Textfelder (`Widget::text` ohne Zeit).
  `fc-list : family` liefert hier 1877 Zeilen, 330 Familien (erster Name vor
  dem Komma). Ein leerer `ui_font` wird `Some("")` (`config.rs:214`), also
  ist „Desktop font“ das Entfernen des Keys, nicht ein leerer Wert.
- Dropdown: höchstens 10 Einträge sichtbar, die Liste scrollt in sich
  (Feder wie die Seite, `Scroll`), Rad über der Liste scrollt sie, Pfeile
  ziehen die Markierung mit und halten sie sichtbar, Einträge am Rand werden
  abgeschnitten (Text mit Clip, Haken nur ganz sichtbar). Die Liste ist so
  breit wie ihr breitester Eintrag, mindestens wie das Control, höchstens wie
  der Platz. Tippen springt zum ersten Eintrag, der mit dem Getippten
  beginnt (Groß/klein egal, was innerhalb 1 s folgt, gehört dazu); bei
  geschlossener Liste öffnet Tippen sie.
- Closed-Control höchstens `MENU_MAX = 280` breit, längere Namen dort mit
  „…“ (der volle Name steht in der Liste).
- `Bind::Choices { key, values }`: je Eintrag ein Wert oder `None` für „zurück
  zum Default“ (`Write::Reset`).
- Seite General (Icon `settings`) mit Font (Icon `type`): „Desktop font“,
  dann die Familien; der Wert aus der Datei, wenn er keine Familie ist,
  bleibt als Eintrag. Familien einmal im Hintergrund (`fc-list`), dann
  Neuaufbau. Ändert sich `ui_font`, lädt das Fenster seine Schrift neu
  (`Fonts` neu, Layout neu).
- Copy-Gate: Font-Namen sind Namen, nicht Text, und zählen nicht.
- Fixture: ein Dropdown mit 40 Einträgen für Überlauf und Scrollen.
- Tests: höchstens 10 sichtbar; Rad scrollt, Ende hält; `End` zeigt den
  letzten; Tippen „no“ markiert den ersten „No…“-Eintrag, nach 1 s beginnt
  ein neues Wort; „Desktop font“ schreibt `Reset("ui_font")`, eine Familie
  ihren Namen; Liste breiter als das Control, aber im Fenster.

Tests S10a: Display-Einträge mit zwei Monitoren und einem abgesteckten,
Kodierung `output:DP-1`; Codec-Einträge aus einer Info-Liste (HDR fällt
weg, `h264_software` → `libx264`); eigener Wert bleibt; Gates und
Copy-Gate über die neuen Zeilen.

#### S11 Ordner-Dialog

- Icon-Knopf `folder-open` neben den Ordnerfeldern;
  `org.freedesktop.portal.FileChooser.OpenFile` mit `directory: true`
  (Portal-Version 4 hier, gtk-Backend), Ablauf wie `capture/portal.rs`
  (Request-Signal abonnieren, dann rufen); Ergebnis-URI → Pfad mit `~`.
  Kein Portal: Knopf aus, Feld bleibt.

##### S11 im Detail (Gegenprüfung am Code, 26.10.)

Belege: `org.freedesktop.portal.FileChooser` Version 4 auf dem Bus (gtk-
Backend), `OpenFile(s parent, s title, a{sv} options) → o`, Option
`directory` (ab Version 3), `current_folder` (Bytes mit Null am Ende),
`handle_token`; Antwort über `org.freedesktop.portal.Request.Response`
(`u` Code, `a{sv}` mit `uris`). Ablauf und `percent_decode` wie
`capture/portal.rs:22-97` (Signal abonnieren, dann rufen).

Verhalten:
- Ordnerfelder (Save folder, Video folder) haben rechts im Feld einen
  Icon-Knopf `folder-open` (quadratisch, eigene Hover-Fläche); die Textfläche
  endet `GAP_S` davor. Klick oder `Ctrl+O` im Feld öffnet den Dialog mit dem
  aktuellen Ordner; die Wahl landet im Feld mit `~` und wird geschrieben wie
  ein bestätigtes Feld. Abbruch ändert nichts.
- Der Dialog läuft in einem Thread (er wartet auf den Nutzer); die Antwort
  kommt mit (Seite, Block) zurück, eine andere Seite inzwischen verwirft sie.
- Kein FileChooser auf dem Bus: der Knopf fehlt, das Feld bleibt (geprüft
  beim Start im Hintergrund).
- `Effect::Browse` (vom Feld) wird in der App zu `Effect::PickFolder(block)`;
  `App::pick(block, path, now)` setzt den Text und meldet die Änderung.

Tests: Knopf nur mit `browse`; Klick auf den Knopf und `Ctrl+O` liefern
`PickFolder(block)`, Klick in den Text nicht; `pick` schreibt `save_dir`;
Überlauf- und Achsen-Gates mit Knopf (Fixture-Pfad).

#### S12 Ecken-Piktogramme (U11)

- Vier Sprite-Symbole: Bildschirmrahmen 16 × 12 mit gefülltem Punkt in der
  Ecke, Strich wie Lucide (`STROKE`), eigene `<symbol>` in `lucide.svg` mit
  Kommentar zur Herkunft. Test: rendert als Icon (Tinte im Rahmen).

##### S12 im Detail (26.10.)

Lucide hat kein Set „Karte in Ecke X“ (`picture-in-picture` nur für eine Ecke).
Vier eigene Symbole im Lucide-Raster (24, Strich 2, runde Enden wie Lucide):
Bildschirmrahmen `rect 20 × 16 rx 2` bei (2, 4), darin die Karte `rect 8 × 5
rx 1` (nach dem ersten Render größer als zuerst 6 × 4, bei 14 px sonst
3,5 px breit), 2 Einheiten von den Innenkanten, in der jeweiligen Ecke. IDs
`shelf-top-left` … `shelf-bottom-right`, im Sprite mit Kommentar „eigene,
Lucide-Stil“. Ersetzt die vier Pfeile im Corner-Segment. Test: je Symbol
liegt die Tinte der Karte im richtigen Viertel (Probe-Pixel in der Mitte
der Karte, gegenüberliegende Ecke leer).

#### S13 Spalte und dünne Seiten (U10, U12), nach D2 und D3

#### S14 AccessKit (U13), nach D4

- Eigene Phase mit Probe zuerst: `accesskit_unix` 0.22.1 (zbus 5.5 mit
  `async-io`, Boltsnap hat zbus 5.17) im Fenster, Baum aus Seiten/Zeilen,
  Aktionen Fokus/Klick; Probe mit `accerciser` oder Orca. Kosten: rund
  20 Crates mehr (atspi, async-executor …).

#### Reihenfolge (Vorschlag)

1. S1, weil es das ganze System betrifft und der Fix aus 4f fertig vorliegt.
2. S2, S3, S4, S5: Sichtbarkeit und Ausrichtung, unabhängig voneinander,
   S3 vor S5 (Breiten hängen an Umbruch).
3. S6, dann S7 und S8: brauchen beide die Live-Werte.
4. S9, S10, S11: neue Inhalte auf dem fertigen Fundament.
5. S12 Feinschliff, S13 und S14 nach den Entscheidungen.

Abhängigkeiten: S7, S8 brauchen S6; S5 nach S3; S10 braucht die scrollende
Liste (in S10 selbst); S11 braucht S3 nicht.

Was kaputtgehen kann: S3 ändert alle Höhen (Gates und Tests decken
Überlauf, Abstände, Achsen); S6 ersetzt die Wertquelle (Test: gleiche Werte
wie `prefs::load()` für eine Beispiel-Datei); S1 fasst den Daemon an (Shelf
zeigt, Karten erscheinen und verschwinden, live geprüft).

### Phase 5d: Live-Vorschauen (26.10., Wahl des Nutzers)

Der Nutzer will das Fenster „besonders“ machen und hat Live-Vorschauen
gewählt (Seiten zeigen, was eine Einstellung tut). Reihenfolge: P1 Shelf,
danach P2 Displays, P3 Replay-Puffer, je nach Rückmeldung zu P1.

P1, Shelf. Belege: Karten `CardSize` (190 × 132, Seitenverhältnis bleibt,
`shelf/thumbnail.rs:16`), Shelf 24 px von beiden Kanten seiner Ecke
(`EDGE_MARGIN`, `linux/shelf/cards.rs:29`), darin 22 px Rand und 10 px
zwischen den Karten (`LayoutConfig`, `shelf/layout.rs:9`), gestapelt von der
Ecke aus. Verhalten:
- Oben auf der Shelf-Seite eine eigene Fläche (`PANEL`, `PANEL_RADIUS`, kein
  Rahmen) mit einem Bildschirm im Seitenverhältnis 16:9 (`WINDOW`, Radius `RADIUS`,
  die Stufe unter `PANEL_RADIUS`, weil der Innenabstand gleich dem Radius ist),
  so breit wie die Fläche innen, `GAP_M` rundum (zuerst höchstens 360 px
  breit und mittig; der Render zeigte leere Ränder links und rechts, gegen
  „gleicher Innenabstand“ und „Füllgrad“).
- Darin die Karten des Shelfs maßstäblich (Bezug 1920 × 1080), in der
  gewählten Ecke, so viele wie „Cards shown“, so breit wie „Card size“; was
  nicht auf den Bildschirm passt, schneidet der Bildschirmrand ab.
- Folgt den Controls sofort, auch beim Ziehen des Sliders, nicht erst nach
  dem Speichern. Ecke wechseln: die Karten gleiten zur neuen Ecke (`GLIDE`);
  Breite: sie wachsen und schrumpfen (`SNAP`); Anzahl: neue blenden ein,
  überzählige aus (`QUICK`), keine wandert dafür ein paar Pixel (Regel 4h:
  Karten bleiben auf ihrem Platz, nur die neue kommt).
- Reduced Motion: springt.
- Kein Text, keine Deko-Bilder: die Karten sind Flächen (`SELECTED`, Radius
  maßstäblich), wie der Shelf ohne Inhalt.
Code: `Block::Preview(Preview::Shelf)`, `app/preview.rs` (Ziele aus den
Werten der Zeilen, Federn je Karte in Bildschirm-Koordinaten 0..1, damit ein
Resize nichts animiert), Layout-Höhe aus der Spaltenbreite, `draw.rs`.
Tests: Ziel-Rechtecke für Ecke, Breite, Anzahl gegen die Shelf-Formeln;
Ecke wechseln gleitet (halbwegs zwischen den Ecken), Ende exakt; Anzahl +1
blendet die neue ein, die anderen bleiben stehen; Gates (Überlauf, Abstände)
mit Vorschau; Render.

#### P2 Displays und P3 Replay-Leiste im Detail (26.10., freigegeben)

Gemeinsam: `app/preview.rs` wird ein Ordner `app/preview/` (`mod.rs` mit
Block-Art, Höhe und Bildschirm-Rechteck, `shelf.rs`, `displays.rs`,
`replay.rs`). Jede Vorschau ist eine eigene Fläche (`PANEL`, `GAP_M`
rundum, Abstand-Gate prüft das), folgt den Controls sofort und blendet bzw.
gleitet mit den Tokens; Reduced Motion springt; keine Mini-Wege (4h).

P2, Displays (oben auf der Recording-Seite). Belege: Lage und Größe jedes
Monitors aus `OutputInfo::logical_position`/`logical_size` (sctk
`output.rs:365`, wie der Shelf in `cached_monitors`), compositor-neutral.
Verhalten:
- Die Monitore maßstäblich in ihrer echten Anordnung, zusammen so groß wie
  die Fläche innen erlaubt (höchstens 200 px hoch), mittig darin; jeder mit
  2 px Abstand zum Nachbarn und seinem Namen (`Hint`, gekürzt auf seine
  Breite).
- Gewählt hell (`SELECTED`, Name `TEXT`), nicht gewählt dunkel (`RAISE`,
  Name `SUB`). „Focused display“: alle halb (heißt „der beim Start
  fokussierte“; der Monitor des Settings-Fensters wäre irreführend).
  „Both displays“: beide hell; mit „Both displays as: Combined“ schließt sich
  die Lücke zwischen ihnen zu einer Fläche (ein Clip), „Separate“ zeigt die
  Lücke (zwei Clips).
- Wechsel blenden (`QUICK`) je Monitor und für die Brücke; nichts wandert.
- Keine Monitore bekannt (Render ohne Wayland): keine Vorschau.
- `Prefs::displays` wird zu `Vec<Screen { name, label, x, y, w, h }>`.

P3, Replay-Leiste (oben auf der Replay-Seite). Belege: Status liefert
`state`, `duration_us` (gemessen 29 050 000 bei 30 s Länge), `seconds`;
`replay::preview_target` fragt den Steuer-Socket direkt mit 100 ms Timeout
ab, ohne den Daemon zu starten (`replay/mod.rs`). Verhalten:
- Eine Zeitleiste über die ganze Breite: rechts „now“, links die Länge
  („−30 s“, Minuszeichen U+2212), dazwischen Striche in einer runden
  Schrittweite (5 s, 10 s, 15 s, 30 s, 1 min, 2 min, 5 min), höchstens sechs.
- Läuft der Puffer: von rechts gefüllt, so weit er gerade hält
  (`duration_us`), wächst nach einem Start sichtbar mit (Abfrage 1 × je
  Sekunde über einen eigenen, leisen Status-Aufruf `replay::status_quiet`,
  der den Daemon nicht startet). Steht er: leer.
- Passt die Länge nicht in den Speicher (S9), ist der Teil, der nicht
  gehalten werden kann, dunkler (`WINDOW` statt `RAISE`); der Hinweis unter
  Memory sagt es in Worten.
- Neue Länge: Maßstab und Striche gleiten (`GLIDE`, die Feder läuft über die
  Sekunden), die Füllung gleitet mit.
- Texte: Striche mit `Key`-Stufe in `SUB`.

Tests: P2 Rechtecke für zwei Monitore nebeneinander (Reihenfolge, Verhältnis,
mittig), Helligkeit je Ziel, Brücke nur bei Both + Combined, ohne Monitore
kein Block; P3 Schrittweiten für 15 s, 30 s, 1 min, 10 min, Füllung aus
`duration_us`, dunkler Teil im Engpass, Länge wechseln gleitet; Gates.

### Phase 5e: Vorschauen und Ecken schöner (26.10., Meldung des Nutzers)

Meldung: die Shelf-Vorschau wirkt unspektakulär, die Ecken-Icons sind nicht
schön, die Replay-Leiste könnte schöner sein, die Monitor-Anzeige ist
„sehr hässlich“. Befunde am Screenshot des Nutzers und am Code:
- Monitore: Namen sind die Wayland-Beschreibung mit Seriennummer
  („BNQ ZOWIE XL LCD EB66M01776SL…“), gekürzt; zwei flache graue Kästen.
  `OutputInfo` hat `make` und `model` getrennt (sctk `output.rs:324`);
  hier `AOC`/`27G4HRE`, `BNQ`/`ZOWIE XL LCD`.
- Replay: Puffer läuft mit 29,85 von 30 s (gemessen), die Leiste war also
  fast voll; `SELECTED` auf `RAISE` (1,33:1) liest sich wie leer.
- Ecken-Icons: 1,5-px-Striche bei 14 px, die Karte 3,5 px breit.
- Shelf: dunkler, leerer Bildschirm mit grauen Kästen.

Gemeinsame Quelle für echte Inhalte: die Screenshots im Speicherordner
(`resolve_save_dir`, hier 13 Dateien, PNG und MP4); nur PNG, die neuesten
zuerst, einmal beim Öffnen im Hintergrund geladen und verkleinert, nie
eine neue Aufnahme. Keine Screenshots: die Vorschauen bleiben wie jetzt
(Flächen). Nichts verlässt den Rechner.

V1 Monitore:
- Jeder Monitor als Monitor: Gehäuse (`RAISE`, Radius `RADIUS − GAP_XS`),
  Bildschirm 3 px eingerückt, darunter ein kurzer Fuß (Hals und Standfläche,
  `RAISE`). Wohin aufgenommen wird, dessen Bildschirm ist an: er zeigt den
  passenden Ausschnitt des neuesten Screenshots, der so groß ist wie die
  ganze Anordnung (sonst `SELECTED`); die anderen sind aus (`WINDOW`).
  Focused: alle halb an.
- Name unter dem Fuß: Hersteller und Modell („BenQ ZOWIE XL LCD“,
  „AOC 27G4HRE“; Herstellercodes BNQ→BenQ, GSM→LG, DEL→Dell, SAM→Samsung,
  ACR→Acer, AUS→ASUS, HWP→HP, LEN→Lenovo, PHL→Philips, GBT→Gigabyte,
  ENC→EIZO, sonst der Code), darunter der Anschluss (`Key`, `SUB`). Gleiche
  Beschriftung im Display-Dropdown.
- Bei Both eine Zeile unter allen: „One clip“ (Combined) bzw. „Two clips“
  (Separate); bei Focused „The one with focus when you start“.
- Wechsel blenden (`QUICK`), nichts wandert.

V2 Ecken:
- Segment-Teile als gefüllte Mini-Bildschirme statt Strich-Icons: Fläche
  20 × 13 (`SUB` für nicht gewählt, `TEXT` für gewählt, Deckkraft gedämpft),
  darin die Karte 8 × 5 voll in der Ecke; eigener Teil-Typ im Segment
  (`Part::Corner`).
- Direkte Wahl in der Shelf-Vorschau: Klick in ein Viertel des Bildschirms
  setzt die Ecke (wie das Segment, schreibt `shelf.corner`); Hover zeigt
  die Karten blass in diesem Viertel. Tastatur bleibt das Segment.

V3 Replay:
- Kopfzeile in der Fläche: links „Holds 29 s“ (läuft) bzw. „Off“ (`Label`,
  `TEXT`), rechts die Bitrate wie im Hinweis („41 Mbit/s“, `SUB`).
- Die Leiste aus Zellen (eine je Sekunde bis 60, sonst je Länge/60 s),
  16 px hoch, 2 px Fuge: gehaltene Zellen hell (`TEXT` gedämpft auf 0,85),
  leere `RAISE`, nicht haltbare `WINDOW`; eine Zelle blendet beim Füllen
  stetig ein (Anteil aus der gleitenden Füllung).
- Zeiten darunter wie jetzt, „now“ in `TEXT`.

V4 Shelf:
- Hintergrund: der neueste Screenshot, auf 16:9 zugeschnitten (bei einem
  Bild über beide Monitore der Teil des ersten), Richtung `WINDOW`
  abgedunkelt (60 %), damit die Karten vorne stehen.
- Karten: die nächsten Screenshots als Vorschaubilder (füllend zugeschnitten
  auf das Kartenformat), Radius maßstäblich; mehr Karten als Bilder: die
  übrigen als Fläche.
- Maße (Wunsch des Nutzers, 26.10.): an der vordersten Karte eine Maßlinie
  für Breite und Höhe mit „190 px“ und „132 px“ (`Key`, `SUB`, dünne Linie
  mit Endstrichen, nicht auf der Karte). Interaktiv: die Kante der vordersten
  Karte lässt sich ziehen und ändert „Card size“ wie der Slider (in 10er
  Schritten, gedrosselt geschrieben wie der Slider); Hover über der Kante
  hellt die Maßlinie auf. Tastatur bleibt der Slider.

Code: `linux/app/shots.rs` (neu: neueste PNG laden, verkleinern, über einen
Kanal an das Fenster), `app/preview/` (Bilder als Pixmaps, Zeichnen mit
`tiny_skia::Pattern` in abgerundete Pfade), `widgets/segment.rs`
(`Part::Corner`), `pages/` (Beschriftung), Klick und Hover auf der
Shelf-Vorschau in `App` (`Target::Preview`).

Tests: Herstellercodes; Monitor-Teile und Beschriftung; Ecke per Klick in
ein Viertel schreibt `shelf.corner`, Hover blendet die Geisterkarten;
Zellenzahl je Länge, Kopfzeile; Bild-Zuschnitt (16:9, Kartenformat) als
reine Funktion; Gates (Überlauf, Abstände, Kontrast der neuen Texte).

Macht nicht: Live-Bild des Bildschirms (das zeigte das Settings-Fenster
selbst), Videos als Karten, Schatten.

#### V1 und V4 im Detail (Gegenprüfung am Code, 26.10.)

Befund am Code und an den Daten:
- Der Speicherordner hier (`~/Bilder/boltsnap`, `paths.rs:139`) hat 3 PNG,
  alle Bereichsaufnahmen (415 × 154 fast schwarz, 254 × 158, 588 × 375),
  dazu MP4 und `.cursor.json`. Ein Bild „über die ganze Anordnung“ gibt es
  hier nicht; hochgezogen auf 656 px wirkt eine Bereichsaufnahme verwaschen.
- Der echte Shelf stapelt die neueste Karte oben (`visible_cards` ist
  `newest_first`, `cards.rs:125`; `Layout::compute` von oben,
  `shelf/layout.rs:76`), also in unteren Ecken am weitesten von der Ecke.
- `OutputInfo::make`/`model` (sctk 0.19.2 `output.rs:324`, `:327`).
- tiny-skia 0.11.4: `Pattern::new(pixmap, SpreadMode, FilterQuality, opacity,
  transform)` (`shaders/pattern.rs:81`); die Pattern-Transformation wird mit
  der des Pfads verkettet (`painter.rs:308`), sie bildet Bildpixel auf
  logische Koordinaten ab; `Pixmap::from_vec` will vormultipliziertes RGBA
  (`pixmap.rs:62`). Dekodieren mit Grenzen wie der Shelf
  (`ImageReader::with_format`, `Limits::max_alloc`, Header-Maße vor dem
  Dekodieren, `linux/shelf/cards.rs:363`); Zuschnitt plus Verkleinern mit
  `shelf::thumbnail::make_image_card_thumbnail`.
- `--render` hat keine Monitore (kein Wayland); die Prüf-Renders der Displays
  laufen über einen Wegwerf-Test im Binary mit der echten Schrift.

Abweichungen vom Plan oben (begründet):
- Bildschirm-Inhalt (V1) und Hintergrund (V4) nur aus dem neuesten PNG, das
  ein ganzer Bildschirm ist (mindestens 1280 × 720); für V1 muss sein
  Seitenverhältnis dem der Anordnung entsprechen (± 2 %). Bereichsaufnahmen
  werden nur Karten. Hier also: Flächen statt Bild auf den Monitoren und ein
  dunkler Bildschirm hinter den Karten, die Karten zeigen die drei Bilder.
- Karten tragen die Bilder von der Ecke aus, das neueste vorn an der Ecke,
  auch unten (der Shelf hat es dort oben): so behält jede Karte ihr Bild, wenn
  „Cards shown“ wechselt (Regel 4h), statt dass alle eins weiterrücken.
- Hintergrund zu 65 % statt 60 % Richtung `WINDOW`: `TEXT` auf einem weißen
  Bildschirm braucht ≥ 62,5 % für 4,5:1. Die Maßzahlen stehen deshalb in
  `TEXT`, die Linien in `SUB`, Hover hellt die Linien auf `TEXT`.
- Unter den Monitoren steht immer eine Zeile, damit die Seite beim Wechsel
  nicht springt: „One clip“ auch für einen einzelnen Monitor, „Two clips“
  (drei: „Three clips“), Focused „The one with focus when you start“, ein
  abgesteckter Monitor „Not connected, the focused one records“ (das tut
  `resolve_record_outputs`, `record.rs:123`; die Monitore dann halb an wie
  bei Focused). Platz für die längste Zeile bei der Breite (bis 2 Zeilen).
- Dropdown: „BenQ ZOWIE XL LCD (DP-3)“ (Name wie in der Vorschau, Anschluss
  dahinter, damit zwei gleiche Modelle unterscheidbar bleiben). Tray bleibt.

V1 Verhalten und Maße (von oben, `GAP_M` rundum):
- Anordnung maßstäblich, höchstens 160 px hoch (vorher 200; darunter kommen
  jetzt Fuß, Namen und Zeile). Jeder Monitor: Gehäuse `RAISE`, Radius 5
  (`RADIUS − GAP_XS`), 4 px Fuge zum Nachbarn (`GAP_XS`), Bildschirm 3 px
  eingerückt, Radius 2 (konzentrisch), aus `WINDOW`, an: Bildausschnitt oder
  `SELECTED`, mit der Feder je Monitor gemischt (`QUICK`). Das Bild ist ein
  Pattern über das ganze Rechteck der Anordnung, jeder Bildschirm-Pfad zeigt
  so von selbst seinen Teil. Combined: eine Fläche über alle Bildschirme samt
  Fugen (Brücke, `QUICK`).
- Fuß 8 px: Hals 5 px hoch, 8 % der Gehäusebreite (mind. 4), Standfläche 3 px,
  28 % (mind. 12), Radius 1,5, `RAISE`; nur unter Monitoren, unter denen kein
  anderer steht.
- `GAP_S`, dann je Monitor zwei Zeilen unter seiner Mitte, auf seine Breite
  gekürzt: Name (`Hint`, 15 px Zeile, `SUB` → `TEXT` mit der Feder), Anschluss
  (`Key`, 14 px Zeile, `SUB`; fehlt, wenn er gleich dem Namen ist). Monitore,
  die sich waagrecht überlappen (übereinander): die Namen teilen die Breite
  gleich auf, sortiert nach Lage, damit nichts übereinanderliegt.
- `GAP_S`, die Zeile (`Label`, `TEXT`, links), Wechsel blenden über Kreuz
  (`QUICK`, eine Feder je Text).
- Herstellercodes als reine Funktion `pages::monitor_name(make, model,
  connector)`: Code → Name aus der Liste oben, `Unknown`/leer fällt weg, ein
  Modell, das schon mit dem Namen beginnt, bekommt ihn nicht doppelt, ohne
  beides der Anschluss.

V4 Verhalten und Maße:
- Hintergrund: das Bildschirm-PNG füllend auf 16:9 (breiter: der linke Teil,
  also der erste Monitor einer Aufnahme über alle; höher: die Mitte), darüber
  `WINDOW` zu 65 %. Ohne Bild `WINDOW` wie bisher.
- Karte i zeigt Bild i (füllend auf das Kartenformat 190 : 132, das der
  Lader schon zuschneidet), Radius maßstäblich, `shown` als Deckkraft; ohne
  Bild `SELECTED` wie bisher. Bilder kommen an: blenden ein (`QUICK`).
- Maßlinien an Karte 0 (an der Ecke): Höhe senkrecht `GAP_S` neben der
  Innenkante (rechts bei linken Ecken), Breite waagrecht mittig im Rand
  zwischen Karte und Bildschirmkante; 1 px Linie, 5 px Endstriche, auf ganzen
  Pixeln. Beide Zahlen („190 px“, „132 px“ aus dem Slider, Höhe nach
  `CardSize::from_width`) in einer Spalte `GAP_XS` hinter der Höhenlinie,
  Versalien mittig auf Kartenmitte bzw. Breitenlinie (`Key`, `TEXT`).
  Nur wenn der Rand mindestens 10 px hat (Bildschirm ≥ 418 px breit), sonst
  blenden Linien und Zahlen aus (`QUICK`, keine harte Schwelle beim Resize).
- Ziehen: ein 8 px breiter Streifen über der Innenkante von Karte 0 ist ein
  eigenes Ziel (`Target::Preview(block)`); Hover und Drücken hellen die Linien
  auf (`QUICK`). Ziehen setzt „Card size“ über `Widget::adopt` auf
  Startbreite + Weg / Maßstab (rechte Ecken: gespiegelt), der Slider rastet
  auf 10 und klemmt 140 bis 380; geschrieben wie beim Slider (Drossel 100 ms
  über `capture`, der letzte Wert beim Loslassen). Solange gezogen wird,
  übernimmt die Zeile keinen Wert vom Daemon (wie eine gezogene Zeile). Die
  Karten folgen mit `SNAP`. Tastatur bleibt der Slider; kein Tab-Halt.

Lader `linux/app/shots.rs`:
- Ein Thread beim Öffnen (`spawn`, Kanal an das Fenster wie `poll_replay`),
  liest `resolve_save_dir`: nur Dateien mit Endung `.png` (Groß/klein egal),
  nach Änderungszeit, neueste zuerst; höchstens 16 Dateien angefasst. Für
  jede die Header-Maße; Grenzen wie der Shelf (256 MiB, 64 MP); nicht
  lesbar oder zu groß: übersprungen. Erstes Bildschirm-PNG → Bildschirm
  (in 1600 × 900 eingepasst), die anderen → Karten (zugeschnitten auf
  384 × 267), bis 8 Karten da sind. Ergebnis: `Shots { screen, cards }` aus
  `ui::Picture` (vormultipliziert, Stufen halber Größe bis ≥ 64 px, damit
  das Verkleinern beim Zeichnen höchstens halbiert und nichts flimmert).
- `--render` lädt dieselben Bilder sofort (ohne Thread), damit der Render
  zeigt, was das Fenster zeigt.
- Geteilter Code bekommt keine Pfade: `Shots`/`Picture` sind nur Pixel; die
  Regeln „ganzer Bildschirm“, Einpassen und der Zuschnitt sind reine
  Funktionen in `app/preview/shots.rs`.

Randfälle: kein Speicherordner oder keine PNG (nur MP4): `Shots` leer, alles
wie bisher, kein Fehler. Unlesbares PNG: übersprungen. Riesiges Bild: Grenze
oder einmal verkleinert. Ein Monitor: eine Zeile „One clip“, der Fuß darunter.
Drei Monitore: drei Monitore, Namen gekürzt. Abgesteckter Ausgang: halb an
und die Zeile. Ordner wechselt bei offenem Fenster: die Bilder bleiben die
beim Öffnen geladenen.

Code (in Reihenfolge, V1 ein Commit, V4 ein Commit):
1. V1: `pages::monitor_name` (+ Test), `linux/app/window.rs::displays` nutzt
   ihn, `rows::display_entries` hängt den Anschluss an; `preview/displays.rs`
   (Maße, Fuß, Namen, Zeile, Bild falls da), `Preview::height` bekommt die
   Schriften (`layout.rs`), `ui/picture.rs` (Pattern in einen runden Pfad),
   `preview/shots.rs` (reine Regeln), `linux/app/shots.rs` (Lader) und sein
   Kanal in `linux/app/mod.rs`, `App::set_shots`. Tests: Codes, Teile je
   Monitor im Block, Zeile je Ziel, Kreuzblende, Gate (alles in `GAP_M`).
2. V4: `preview/shelf.rs` (Hintergrund, Kartenbilder), `preview/dims.rs`
   (Maßlinien, Kante, Ziehen), `Target::Preview` in `app/mod.rs`
   (`target_at`, `press`, `moved`), `writes.rs::update` hält die gezogene
   Zeile. Tests: Zuschnitt 16:9 und Kartenformat, Maßzahlen folgen dem
   Slider, Ziehen in 10er Schritten schreibt gedrosselt, Hover hellt auf,
   Sichtbarkeit nach Randbreite; Gate (Maßzahlen im Bildschirm, Kontrast
   `TEXT` auf dem dunkelsten zulässigen Hintergrund).

Abnahme: `cargo fmt --check`, `cargo clippy --all-targets` ohne neue
Warnung, `cargo test` grün, `git diff --check`; Renders (dunkel) der Seiten
Recording und Shelf bei 760 und 360 px Fensterbreite mit der echten Schrift
und den echten Screenshots, einmal mit einem Bildschirm-PNG zur Probe;
Zahlen der Gates in den Bericht. Live (nur nach Rückfrage): Ziehen der Kante
im echten Fenster, Bilder nach dem Öffnen.

Macht nicht (V1, V4): Mauszeiger-Form über der Kante (das Fenster setzt nur
den Standard-Zeiger), Neuladen der Bilder bei neuem Ordner, Videos und
Bilder außerhalb des Speicherordners, Ziehen der Höhe, V2 (Ecken-Segment
und Klick in ein Viertel).

#### V4b Maße, die zum Fenster passen (27.10., Meldung des Nutzers)

Meldung zum Render von V4: „so wie im bild sieht das kacke aus es soll schon
zum ui und so passen“. Befund am Render (760 px, unten rechts): beide Zahlen
stehen übereinander in der Ecke links unter der Karte, „190 px“ sitzt neben
dem Ende seiner Linie statt an ihr, die Breitenlinie klebt 9 px über der
Bildschirmkante, die Zahlen schweben ohne Fläche über dem Screenshot. Das
liest sich wie eine technische Zeichnung, nicht wie ein Teil des Fensters.

Soll:
- Jede Zahl steht in einer kleinen Fläche (`RAISE`, Radius `GAP_XS`, Höhe
  `Key`-Versalhöhe + 2 · `GAP_XS` = 16, Innenrand `GAP_XS` rundum, Versalien
  exakt mittig), mittig an **ihrer** Linie, auf der Seite weg von der Karte,
  `GAP_XS` Abstand. Damit heißen sie überall gleich: wie die Werte der
  Controls.
- Höhe: Linie `GAP_S` neben der Innenkante der vordersten Karte (wie jetzt),
  Zahl links davon (rechte Ecken) bzw. rechts davon, auf der Kartenmitte.
- Breite: Linie `GAP_S` hinter der **letzten** Karte des Stapels, auf der
  freien Seite (unten: über der obersten Karte, oben: unter der untersten),
  Zahl mittig darüber bzw. darunter. Dort ist Platz; der Rand zur
  Bildschirmkante ist es nicht.
- Linien 1 px, Endstriche 1 × 5 px, `SUB`; Hover und Drücken der Kante: Linien
  `TEXT`, Flächen `HOVER` (`QUICK`, wie jetzt).
- Die Lage der Breitenlinie ist eine Feder in Bildschirmkoordinaten: neue
  Kartenzahl `SNAP`, neue Ecke `GLIDE` (wie die Karten), sonst gesetzt. Sie
  wandert also mit dem Stapel, statt zu springen.
- Sichtbar, wenn hinter dem Stapel Platz für Linie und Zahl ist
  (`GAP_S + 1 + GAP_XS + 16 + GAP_XS`) und neben der vordersten Karte Platz für
  die Höhenzahl; sonst blendet das Maß aus (`QUICK`, wie jetzt). Bei 360 px
  zeigt es damit (bisher nicht); bei vielen großen Karten, die den Bildschirm
  füllen, nicht.

Code: `preview/dims.rs` (`Measure` mit `width_chip`/`height_chip` statt
`label_x`/`width_y`/`height_y`, `measure` bekommt den Rand der letzten Karte
und die Breiten der Zahlen, Zeichnen), `preview/mod.rs` (Feder `far`),
`preview/shelf.rs` (`sync_shelf` setzt die Feder). Tests: Lage an der
vordersten und der letzten Karte für unten rechts und oben links, Zahl mittig
an ihrer Linie, Versalien mittig in der Fläche, Feder bei neuer Kartenzahl,
Ausblenden bei vollem Bildschirm; Gate: Linien, Flächen und Kante auf dem
Bildschirm bei allen Breiten.

Macht nicht: andere Linienstärken, Maße an anderen Karten, Zahlen in der
Karte.

#### V5 Keine eigenen Bilder des Nutzers (27.10., Meldung des Nutzers)

Meldung: „nehm auch nicht einfach bilder von mir bitte“. Das Fenster zeigt
die Screenshots des Nutzers nicht und liest den Speicherordner überhaupt
nicht mehr. Das ersetzt in V1 und V4 alles zu Bildern („Gemeinsame Quelle
für echte Inhalte“, Hintergrund, Kartenbilder, Lader).

Befund am Code (HEAD `5c17e38`): Bilder kommen nur über
`linux/app/shots.rs` (Thread + Kanal in `linux/app/mod.rs`, dazu das direkte
Laden in `render`), landen über `App::set_shots` in `preview::State`
(`shots`, `shots_in`) und werden nur in `preview/displays.rs::draw_displays`
(Bildschirme, Fugen) und `preview/shelf.rs::draw_shelf` (Hintergrund mit
`BACKDROP_DARK`, Kartenbilder) gezeichnet. `ui/picture.rs` und
`preview/shots.rs` nutzt sonst niemand (`grep` nach `picture`, `shots::`,
`cover`, `same_shape`). `docs/codebase.md` und README nennen die Bilder
nicht.

Soll:
- V1: Bildschirme als Flächen wie ohne Bild bisher: aus `WINDOW`, an
  `SELECTED` 30 % Richtung `SUB`, halb an gemischt; Fuß, Namen, Zeile bleiben.
- V4: Bildschirm `WINDOW`, Karten `SELECTED`; das Maß (Flächen, Linien,
  Kante) bleibt genau wie in V4b.
- Kein Ersatzbild, keine erfundenen Inhalte.

Code: weg mit `git rm` `linux/app/shots.rs`, `app/preview/shots.rs`,
`ui/picture.rs`; in `linux/app/mod.rs` Modul, `save_dir`, Kanal und das
Laden in `render`; in `preview/mod.rs` `shots`, `shots_in`, `set_shots`,
`BACKDROP_DARK`-Export und die Feder in `preview_moving`; Zeichnen in
`displays.rs` und `shelf.rs` ohne Bild; Tests nur für Bilder
(`a_whole_screen_is_told_from_an_area_and_pictures_fit_and_cover`,
`a_picture_keeps_halves_down_to_64_px_and_premultiplies`,
`the_cards_show_the_newest_screenshots_on_the_darkened_screen`, der
Lader-Test) und das Kontrastpaar „measure on a white screenshot“. Ein Test
prüft stattdessen per Pixel, dass Karten `SELECTED` und der Bildschirm
`WINDOW` sind.

Abnahme: fmt, clippy ohne Warnung, `cargo test`, `git diff --check`,
`grep` findet keinen Lesezugriff auf den Speicherordner im Fensterpfad;
Renders Shelf und Recording bei 760 (Skala 2) und 360.

### Phase 5f: Squircle-Ecken für alle Flächen (27.10., Wunsch des Nutzers)

Meldung zur neuen Selector-Leiste (gilt für das ganze Produkt): „die kanten
und so wirken nicht squircle und etwas unclean … mathematisch alles
gestützt“. Teil des Selector-Plans
(`docs/plans/selector/2026-10-27-selector-toolbar.md`, S4/S5); hier, weil es
`ui::Canvas` und damit Settings-Fenster, Vorschauen und Selector betrifft.

**Befund:** `ui::push_rounded` (`src/ui/mod.rs:294`) setzt Kreisbögen als
Kubiken an die geraden Kanten. Am Übergang springt die Krümmung von 0 auf
1/r; das Auge sieht den Knick als „Ecke angesetzt“. Benutzer: `Canvas::fill`
(`mod.rs:257`), `Canvas::ring` (`mod.rs:235`), `picture.rs:91` (Bild in
runder Fläche) über `rounded_rect`, und `icons.rs:261` (Lucide `<rect rx>`).

**Mathe:**
- Jede Ecke ist ein Quadrant der Superellipse |x/R|ⁿ + |y/R|ⁿ = 1 um den
  Punkt, der R von beiden Kanten entfernt ist. n = 4: die Krümmung ist an der
  Stelle, an der die Kante in die Ecke übergeht, 0 wie die der Geraden
  (stetige Krümmung, kein Knick).
- R so, dass der 45°-Punkt der Ecke genau dort liegt, wo ihn der Kreis des
  Tokens r hinsetzt: der Kreis liegt dort r(1 − 1/√2) von beiden Kanten
  entfernt, die Superellipse R(1 − 2^(−1/n)). Gleichsetzen:
  R = r · (1 − 1/√2) / (1 − 2^(−1/4)) = r · 0,292893 / 0,159104 ≈ 1,8409 r.
  Jedes Radius-Token behält damit seine wahrgenommene Größe; nur der Verlauf
  wird glatt.
- Klemmen: R ≤ m = min(w, h)/2 (und r ≤ m wie bisher). Wird R geklemmt,
  wird n so gewählt, dass der 45°-Punkt weiter stimmt:
  1 − 2^(−1/n) = r(1 − 1/√2)/R ⇒ n = −ln 2 / ln(1 − r(1 − 1/√2)/R). Bei
  r = m ergibt das n = 2, also genau den Kreis: Pillen (Schalter-Spur,
  Slider, Scroll-Daumen) und Kreise (Knöpfe) bleiben rund, dazwischen geht
  die Form stetig von n = 4 nach n = 2. Kein Sprung beim Ziehen über die
  Schwelle.
- Verschachtelt: die Token-Regel bleibt (innen r − Abstand), danach dieselbe
  Abbildung für beide. Am 45°-Punkt ist der Abstand der beiden Konturen dann
  genau der des Kreispaars, also der Abstand selbst.
- Abtasten: Quadrant in Polarform über θ ∈ [0, π/2], Punkt
  R·(cos θ, sin θ)/(cosⁿ θ + sinⁿ θ)^(1/n) (glatt in θ; die Form
  (cos^(2/n) θ, sin^(2/n) θ) hat an den Enden unendliche Steigung und braucht
  dreimal so viele Stücke), adaptiv halbiert, bis der Abstand der Kurvenmitte zur Sehne unter 0,025
  logischen px liegt (≤ 0,1 Gerätepixel bei Skala 4, dem größten
  `--render`-Wert). Füllung mit Kantenglättung wie bisher; Flächen bleiben
  auf ganzen Pixeln, gerade Kanten ändern sich nicht.
- `push_rounded` bleibt für Lucide `<rect rx>` in `icons.rs`: Icons bleiben
  genau Lucide.

**Umsetzung:** `src/ui/mod.rs`: `push_squircle(pb, rect, r)` neben
`push_rounded`, `squircle_shape(r, w, h) -> (R, n)`; `rounded_rect` nutzt
`push_squircle` (damit `fill` und `picture`), `ring` beide Konturen.
`src/ui/tests.rs`: 45°-Punkt gleich dem Kreis ± 0,05 px für r = 2, 4, 5, 9,
12, 16; Klemmen (n = 2 bei r = m, Kreis-Abstand ± 0,05); Sehnenfehler gegen
dicht abgetastete Kurve < 0,025; Pixeltest gerade Kante gleich einer
eckigen Füllung. Danach alle App-Gates neu (Padding, Kontrast, Icons,
Überlauf, Versalien) und Zoom-Ausschnitte (4×) von Segment, Dropdown,
Schalter, Slider, Stepper, Shelf-Maßflächen, Selector-Leiste vorher und
nachher.

**Macht nicht:** das echte Shelf-Overlay (`src/shelf/paint.rs`, eigener
Zeichencode), den Windows-Selector, Icons.

### Phase 5g V6: Shelf-Spielwiese, Kartenlook, Ecken-Symbole (27.10., Wunsch des Nutzers)

Meldung: „die icons hier gefallen mir noch nicht so sehr und beim
settingsmenu das Shelf preview ist immer noch so leer und hat sich immer
noch nicht seine daseinsberechtigung verdient. vielleicht einfach 2 cards,
du kannst die card so wegklicken mit dem x wie bei normalen cards, es kommt
dann immer oben eine platzhalterkarte nach damit du so testen kannst und
dann moechte ich das man noch einstellbar macht ob die karten eine
animation haben und die transparenz und ob es auch on hover durchsichtig
bleibt oder nicht und all dieser kram [...] oder card schatten“.

Ersetzt V4 (Bildschirm im Maßstab) und V4b (Maß mit Breitenlinie hinter dem
Stapel). Die Viertel-Klick-Idee aus den Mockups M1/M2 entfällt.

#### Verhalten

1. **Vorschau = zwei echte Karten in Originalgröße (1:1).** Kein
   Bildschirm im Maßstab mehr. Die Karten malt derselbe Code wie das
   Shelf-Overlay (Radius, Schatten, Transparenz, Hover-Knopf, Animation),
   damit die Vorschau nie vom echten Shelf abweicht.
   - Fläche `PANEL`, Höhe fest `GAP_XL + 132 + 10 + 132 + GAP_XL = 322`
     (zwei Karten in Standardgröße, `GAP_XL` ≥ Schattenrand 16 + 3).
     Größere Karten laufen über den fernen Rand hinaus und werden dort
     abgeschnitten wie am Bildschirmrand; kleinere lassen Luft.
   - Die Karten stehen in der gewählten Ecke der Fläche, `GAP_XL` von
     beiden Kanten; „Corner“ ändern lässt sie dorthin gleiten (`GLIDE`).
   - „Cards shown“ 1 → eine Karte, sonst zwei.
   - Hover auf einer Karte: wie im Shelf (deckend, wenn „Solid on hover“
     an ist), X oben rechts (`close_cell`). Den Speichern-Knopf zeigt die
     Vorschau nicht, er hätte hier nichts zu tun (keine Fake-Controls).
   - Klick auf X: die Karte geht mit der Shelf-Animation (240 ms,
     ease-out-cubic, schrumpft und blendet aus, die Lücke schließt sich),
     und an der Stelle, an der das Shelf neue Karten einsetzt (neueste
     zuerst, an der Ecke), kommt eine Platzhalterkarte nach (150 ms, 0,88 →
     1 und einblenden). „Animations“ aus oder Reduced Motion: sofort.
   - Platzhalter: Fläche `SELECTED` mit Lucide `image` mittig in `SUB`,
     kein Bild des Nutzers, keine erfundenen Inhalte.
   - Maß: Höhenlinie neben der Innenkante der vorderen Karte wie bisher,
     eine Zahlenfläche „190 × 132 px“ an ihrer Mitte; die Breitenlinie hinter
     dem Stapel entfällt (bei 1:1 kein Platz dort). Kante ziehen bleibt.
2. **Neue Einstellungen**, Überschrift „Look“ unter „Cards“, alle `Live`:

   | Zeile | Key | Control | Standard | Heute |
   | --- | --- | --- | --- | --- |
   | Opacity | `shelf.card_opacity` | Slider 30–100 %, Schritt 5 | 80 | fest `CARD_OPACITY` 0,8 |
   | Solid on hover | `shelf.solid_on_hover` | Schalter | an | fest an |
   | Shadow | `shelf.card_shadow` | Schalter | an | fest an |
   | Animations | `shelf.animations` | Schalter | an | fest an |

   Das Shelf-Overlay liest sie über `ShelfPrefs`; ohne Eintrag in der Datei
   bleibt alles wie heute. Animations aus: Karten kommen und gehen ohne
   Skalieren und Blenden, die Lücke schließt sofort.
3. **Ecken-Symbole (ersetzt S12):** Variante G aus
   `scratchpad/shelf2/corner-icons.png`: Ecke als Winkel plus Karte, im
   Lucide-Raster (24, Strich 2, runde Enden). Winkel `M3 11V7a2 2 0 0 1
   2-2h4`, Karte `rect 14 × 10 rx 2` bei (7, 9); Abstand Winkel → Karte 4
   auf beiden Achsen, Karte 7 : 5 ≈ Shelf-Karte 190 : 132. Tinte genau
   2…22 × 4…20, also mittig wie Lucides Rahmen `rect 20 × 16` bei (2, 4);
   die anderen Ecken gespiegelt um 12, Tintenbox bleibt gleich.

#### Code

- **A Umbau ohne Verhalten:** das Zeichnen der Karten wandert in die Lib,
  damit das Fenster (Lib) es nutzen kann: `src/shelf/card.rs` (neu, per
  `#[path]` in `lib.rs` als `shelf_card`) mit `draw_shelf`, Schatten, Blit,
  Hover-Knöpfen, Glyphen und den Animations-Konstanten (Dauern,
  `ANIM_SCALE_MIN`, `ease_out_cubic` aus `linux/shelf/mod.rs`);
  `shelf/layout.rs` ebenso in die Lib (`shelf_layout`, im Binary
  re-exportiert). `draw_shelf` bekommt statt `&ShelfModel` die Bilder je
  Id, `CardKind` zieht mit um. `paint.rs` (1197 Zeilen, über der Grenze)
  fällt damit unter 1000; `linux/shelf/mod.rs` (3681, über der Grenze)
  wächst nicht: die Animationsteile gehen nach `linux/shelf/anim.rs`
  (12. Datei im Ordner, an der Grenze).
- **B Einstellungen:** `settings/schema.rs` vier Keys, `config.rs`
  `ShelfPrefs` (`look: CardLook`), `card.rs` `CardLook { opacity,
  solid_on_hover, shadow }` statt `CARD_OPACITY`, `anim.rs` überspringt die
  Animation bei `animations = false`. Windows: `draw_shelf_opaque` bleibt
  (deckend), Backend unberührt.
- **C Vorschau:** `app/preview/shelf.rs` neu: zwei Plätze mit Id,
  Animationen mit Startzeit wie im Shelf, Ecke als Feder; Malen über
  `shelf_card::draw_shelf` in eine `Pixmap` (Geometrie × ganzzahlige
  Skala) und `Canvas::draw_pixels` (neu, schmal). `dims.rs` auf Höhenlinie
  plus eine Fläche; Treffer für X und Karte in `edge_at`/`target_at`.
  `pages/mod.rs` Überschrift „Look“ mit vier Zeilen (Icons `blend`,
  `mouse-pointer-2`, `copy`, `move` aus Lucide 1.54, falls nicht im Sprite).
- **D Symbole:** vier `<symbol>` im Sprite ersetzt, Test in `ui/tests.rs`
  auf die neue Tinte.

#### Motion

| Wechsel | Token |
| --- | --- |
| Ecke wechseln | `GLIDE` |
| Hover deckend, X einblenden | `QUICK` |
| Karte geht, Lücke schließt, Platzhalter kommt | Shelf-Timing 240/150 ms ease-out-cubic, auf der „Ohne Spring“-Liste: die Vorschau muss die echte Animation zeigen |
| Größe, Anzahl | `SNAP` wie bisher |

Reduced Motion: alles sofort.

#### Abnahme

Tests: X-Klick nimmt die Karte und setzt eine neue an die Ecke (mit und
ohne Animation); Opacity ändert das Alpha der Kartenpixel; Solid on hover
aus hält das Alpha beim Hover; Shadow aus lässt außerhalb der Karte keine
Pixel; neue Keys parsen, fehlen, klemmen; Symbol-Tinte mittig (± 0,5).
Gates der App (Überlauf 360 und Desktop, Padding, Kontrast, Icons,
Versalien). `cargo fmt --check`, `cargo clippy --all-targets`, `cargo
test`, `cargo check --target x86_64-pc-windows-msvc` falls installiert.
Screenshot der Shelf-Seite (Ruhe, Hover, mitten in der Animation).

**Macht nicht:** Speichern, Ziehen oder Öffnen von Karten in der Vorschau,
echte Screenshots, weitere Animationsarten als an/aus, eigene Radien je
Karte, Hyprlands eigene Layer-Animation, Windows.

#### V6b Maß greifbar, Hover flüssig (27.10., Meldung des Nutzers)

Meldung: „es nur so einen pixelstreifen gibt wo dieses maß ding greifbar
wird und das ist nicht mal dort wo das maß ist sondern kurz vor anfang der
shelf card [...] das hovern ueber die cards im preview reagiert sehr
langsam also das muss performant und clean sein“.

Befund am Code (`5200dc8`): `dims.rs::measure` legt `edge` 8 px breit um
die Innenkante der Karte (`inner ± GRIP/2`); Linie und Zahl liegen 9 bis
~90 px daneben und sind nicht greifbar. `preview/shelf.rs::paint_cards`
malt jeden Frame den ganzen Shelf neu (Schatten-SDF je Pixel), bei Hover
dreimal, wandelt alles in `f32` und legt eine fensterweite `Mask` an; bei
jeder Mausbewegung über einer Karte also mehrere volle Painter-Läufe.

Soll:
- **Greifbar ist das ganze Maß:** von `GRIP/2` in der Karte über Linie und
  Zahlenfläche bis `GAP_XS` dahinter, über die Kartenhöhe. Ohne Platz für
  das Maß bleibt der Streifen an der Kante. Über dem Maß und beim Ziehen
  zeigt der Zeiger `ew-resize` (`ui::PointerShape`, das Fenster setzt ihn
  über den Cursor-Theme-Zeiger), sonst den Standardzeiger.
- **Hover ohne Neumalen:** die Painter-Ausgabe wird je Zustand (kein Hover,
  Hover auf Karte n) als `Pixmap` behalten, solange Lage, Größen,
  Animationen, Look und Skala gleich sind; ein Hover blendet nur das
  Kartenrechteck zwischen zwei behaltenen Bildern (`QUICK` wie bisher). Die
  Maske nur, wenn die Fläche über den Block hinausragt, und dann behalten.

Abnahme: Test, dass Linie und Zahl greifbar sind und der Zeiger dort
`ResizeX` ist; Test, dass Hover-Frames keinen Painter-Lauf mehr auslösen
(Zähler); bestehende Tests grün; Render Hover.

#### V7 Zwei Monitore als ein Clip ohne abgeschnittene Innenseiten (27.10., Meldung des Nutzers)

Meldung: „die beiden monitore hier sind auch connected das sieht weird aus
weil die innere seite der monitore abgeschnitten ist“.

Befund (`preview/displays.rs::seams`, `draw_displays`): bei „Combined“ füllt
ein Rechteck von `left.right() − BEZEL` bis `right.x + BEZEL` (Höhe ohne die
Rahmen oben und unten) die Fuge in Bildschirmfarbe. Damit fehlen beiden
Monitoren die inneren Rahmen und Ecken, und oben und unten bleibt eine
Kerbe aus Panel-Grund.

Soll: die Bildschirme bleiben ganz (Rahmen, Ecken). „Ein Clip“ zeigt sich
an den Gehäusen: die Fuge zwischen zwei Nachbarn wird mit `RAISE`
geschlossen, um `BODY_RADIUS` in beide Gehäuse hinein und über die
gemeinsame Höhe, sodass die inneren Gehäuse-Ecken eckig werden und beide
ein Doppelmonitor sind; die Bildschirme liegen danach obendrauf. Übereinander
ebenso. Blendet mit der Feder `bridge` (`QUICK`) wie bisher.

Abnahme: Test, dass bei „Combined“ die Innenkante beider Bildschirme
(Mitte der Fuge ± BEZEL) Rahmenfarbe hat und die Fuge selbst `RAISE` ist,
bei „Separate“ Panel-Grund; Render mit zwei Monitoren nebeneinander.

### Phase 5h: Screenshots-Seite füllen, Replay vervollständigen (27.10., Wunsch des Nutzers)

Meldung: „der screenshots tab ist fast komplett leer bis auf einen pfad,
entweder paar mehr optionen rein oder das noch woanders mit bei packen“; auf
den Vorschlag: „die einstellungen passen so und die fehlenden replay sachen
auch noch bitte. erst vorher alles planen“.

#### Befund am Code (HEAD `54d72ea`)

| Punkt | Heute | Beleg |
| --- | --- | --- |
| In die Zwischenablage | Wayland: aufs Shelf, kopiert nur mit `--copy`; X11 kopiert immer | `main.rs::decide_post_capture` (`copy: args.copy_explicit && args.copy`), `capture_flow` |
| Lupe | nur solange Alt gehalten wird, nicht im Aufnahme-Modus | `select_skia/present.rs:106`, `input.rs:289–297, 396` |
| Kartenklick | `xdg-open <datei>` für Bild und Video | `linux/shelf/mod.rs::media_open_command`, `open_media_card` |
| eddy | seit 21.07. bewusst entkoppelt: kein `--edit`, `replace` über den Socket abgelehnt, „Do not add a replacement editor integration“ | `docs/plans/platform/2026-07-21-decouple-eddy.md`, Test `removed_editor_backchannel_commands_are_rejected` |
| Replay-Monitor | `replay.output` (strict); ohne: wie das Aufnahmeziel, sonst der fokussierte | `replay/monitor.rs::replay_monitor` |
| Replay-Bitrate | `replay.bitrate_kbps` (strict, 500–200 000); ohne: aus Länge und Speicher, 2–80 Mbit/s | `replay/settings.rs::video_kbps` |
| Replay-Encoder | `replay.encoder` (strict Text), Namen wie `record_codec`, CPU abgelehnt | `linux/replay/mod.rs:394–397`, `gsr::choose` |

Zwei Abweichungen vom Vorschlag im Chat: (1) Die Lupe ist nicht dauernd an,
also kein an/aus, sondern „With Alt / Always“. (2) „Open with eddy“ mit
Ersetzen der Karte beim Speichern bräuchte den 21.07. absichtlich entfernten
`replace`-Kanal: **Entscheidung des Nutzers nötig** (siehe unten). Bis dahin
plane ich die entkoppelte Variante.

#### Verhalten

**Screenshots-Seite**, drei Gruppen:

1. **Capture**
   - *Copy to clipboard* (Schalter, neu `copy_to_clipboard`, Standard aus =
     heute): unter Wayland geht jeder Screenshot aufs Shelf *und* in die
     Zwischenablage. `--copy`/`--no-copy` auf der Kommandozeile gehen vor.
     X11 kopiert wie bisher immer; Windows unberührt (eingefroren).
   - *Magnifier* (Segment, neu `selector_magnifier` = `alt` | `always`,
     Standard `alt` = heute): „With Alt“ wie heute, „Always“ zeigt die Lupe
     beim Auswählen ohne Taste. Im Aufnahme-Modus nie (wie heute).
2. **Opening**
   - *Open screenshots with* (Dropdown, neu `shelf.open_with`, Standard
     unset = Standard-App): ein Klick auf eine **Bild**karte öffnet sie mit
     dem gewählten Programm; Videokarten bleiben bei der Standard-App. Angeboten
     werden „Default app“ und die installierten aus einer festen Liste
     (Programm im `PATH`): eddy (`eddy -f`), Satty (`satty --filename`),
     swappy (`swappy -f`), GIMP, Krita, Pinta (Datei als Argument). Ein Wert
     aus der Datei, den die Liste nicht kennt, steht unter seinem Namen da
     (wie bei `choose_text`). Gestartet ohne Shell, die Datei als eigenes
     Argument. Das Bearbeitete speichert das Programm selbst; die Karte bleibt
     wie sie ist (kein Rückkanal, Entscheidung vom 21.07.).
3. **Saving**: *Save folder* wie bisher.

**Replay-Seite**, neue Zeilen unter „Buffer“ (alle `ReplayRestart`, die
vorhandene Zeile „Restart replay“ erscheint dann wie bei Länge und Speicher):

- *Display* (Dropdown über `Bind::Choices`): „Like recordings“ (Key
  entfernt, heutiges Verhalten ohne Eintrag), jeder angeschlossene Monitor
  mit Namen wie in Recording (`Screen::entry`), ein eingetragener, der nicht
  angeschlossen ist, als „NAME (disconnected)“.
- *Bitrate* (Dropdown): „Auto“ (Key entfernt) und 10, 20, 30, 40, 60, 80
  Mbit/s; der Memory-Hinweis rechnet schon mit fester Bitrate
  („Only about N s fit“).
- *Encoder* (Dropdown): wie Codec unter Recording (`codec_entries`) ohne
  „H.264 on the CPU“ (Replay lehnt die CPU ab).

#### Code

- `settings/schema.rs`: `copy_to_clipboard` Bool Live,
  `selector_magnifier` Choice Live, `shelf.open_with` Text Live; die drei
  Replay-Keys gibt es schon.
- `config.rs`: Felder und Getter, `ShelfPrefs::open_with: Option<String>`
  (der Name aus der Liste). Die Liste `EDITORS` (Name, Programm, Argumente
  vor der Datei) ist Linux-Sache und liegt in `linux/shelf/cards.rs`, dort
  auch `media_open_command` (aus `mod.rs` hierher, damit `mod.rs` mit 3494
  Zeilen nicht wächst); das Fenster liest dieselbe Liste für sein Dropdown.
- `main.rs::decide_post_capture`: Wayland-Zweig liest
  `copy_to_clipboard`, wenn kein Flag gesetzt ist; Windows-Zweig wie heute.
- `select_skia`: `Selector` bekommt `magnifier_always`; `present.rs`
  `(self.alt_held || always) && !record_mode`; Redraw bei Bewegung auch ohne
  Alt, wenn `always`.
- App: `pages/prefs.rs` Felder `copy_to_clipboard`, `magnifier_always`,
  `open_with: Option<String>`, `editors: Vec<(String, String)>` (installiert),
  `replay_output: Option<String>`, `replay_encoder: String`;
  `linux/app/prefs.rs` füllt sie (`paths::has_cmd`); `pages/mod.rs`
  Zeilen; Icons aus Lucide 1.54 (`clipboard-copy`, `zoom-in`, `app-window`,
  `monitor`, `gauge`/`activity`, `cpu`), vorher geprüft, dass sie rendern.

#### Kanten

- Kein Editor installiert: nur „Default app“, die Zeile bleibt (zeigt, was
  passiert).
- Eingetragener Editor deinstalliert: Klick fällt auf `xdg-open` zurück und
  meldet einmal „NAME not found, opened with the default app“.
- Replay-Display auf einen abgesteckten Monitor: wie heute (strict, der
  Replay-Start meldet den Fehler); die Zeile zeigt „(disconnected)“.
- Bitrate über dem, was der Speicher hält: der Hinweis sagt es (vorhanden).

#### Abnahme

Tests: Copy-Entscheidung (Flag schlägt Config, Config schlägt Standard, X11
und Windows unverändert); Editor-Befehl (Argumente, Datei als letztes
Argument, unbekannter Name, fehlendes Programm → `xdg-open`); Lupe „always“
im Scene-Test; jede neue Zeile schreibt einen Wert, den ihr Key annimmt
(`every_bound_row_writes_a_value_its_key_accepts` zählt mit); Replay-Display
„Like recordings“ entfernt den Key. Gates der App (Überlauf 360 und Desktop,
Padding, Kontrast, Icons, Versalien, Copy). Screenshot der Screenshots- und
der Replay-Seite. README (Konfiguration, Settings-Abschnitt), Inventar.

**Macht nicht:** Karten nach dem Bearbeiten ersetzen (siehe Entscheidung),
Dateiformat oder Dateinamen beim Speichern, Windows, eine frei eintippbare
Befehlszeile (nur die Liste; ein Eintrag in der Datei wird angezeigt und
benutzt).

#### Offene Entscheidung

eddy kann über `--boltsnap-card-id` und den Socket-Befehl `replace` die
Karte nach dem Speichern ersetzen; Boltsnap hat das am 21.07. bewusst
entfernt. Optionen: (a) entkoppelt wie oben (Empfehlung: kein eddy-Vertrag,
jeder Editor gleich, nichts kann den Daemon über den Socket umschreiben);
(b) den `replace`-Kanal für Bildkarten wieder annehmen, eddy bekäme dann
`--boltsnap-card-id`. (b) dreht die Entkopplung zurück.

**Entschieden 27.10. (Nutzer): (a) nur öffnen.** Die Entkopplung bleibt.

### Phase 5i: Last des Replay-Buffers, Fenster neuer als der Daemon (27.10., Meldung des Nutzers)

Meldung: (Bild „unknown setting shelf.open_with“) „hab das hier bekommen als
ich open with eddy auswaehlen wollte und maybe wäre so eine kleine
leistungseinschaetzung bei den replay settings unten damit man nicht alles
hochballert und sich wundert woher die auslastung kommt“.

#### Befund

- Die Meldung: das Fenster (neues Binary) schreibt über den Daemon, der
  noch das alte Binary mit dem alten Schema hat und den Key ablehnt
  (`settings::load` „unknown setting“). Ein Daemon-Neustart behebt es; die
  Meldung sagt das nicht.
- Last: ein einziger Messpunkt hier (60 fps, 1920 × 1080, NVENC, 41 Mbit/s):
  gsr 24 % eines Kerns, `gsr-kms-server` 1 %, Worker 0,4 %; RSS gsr 210 MB,
  Worker 204 MB. Eine Hochrechnung für andere Einstellungen wäre geraten
  (keine Messreihe, und schwere Messläufe auf diesem Rechner sind tabu).
  Die Prozesse sind ohne Daemon-Hilfe zu finden: die Session heißt
  `worker-<Daemon-PID>-…` (`replay status`), gsr und Worker sind Kinder
  dieser PID (`/proc/<pid>/stat` Feld 4), `gsr-kms-server` ein Kind von gsr.

#### Verhalten

1. **Unten auf der Replay-Seite** ein Block „Load“ (Vorschau-Art, `PANEL`,
   eine Zeile): läuft der Buffer, die gemessene Last, einmal je Sekunde
   neu: „25 % of one core, 410 MB“ (CPU: gsr mit seinen Kindern und Worker
   über die letzte Sekunde, in Prozent eines Kerns; Speicher: Summe RSS).
   Links „Now“ in `SUB`, rechts die Zahlen in `TEXT`, `tabular`. Ist er aus:
   „Off“ und rechts nichts. Laufen ältere Einstellungen als die Seite zeigt
   (`replay_stale`), steht darunter in `SUB`: „Before your changes“.
   Zahlen gleiten nicht (Ohne-Animation-Liste: Messwerte springen ehrlich).
2. **Fenster neuer als der Daemon:** lehnt der Daemon einen Key ab, den das
   Fenster kennt („unknown setting X“ und X im eigenen Schema), sagt der
   Banner „Boltsnap is older than this window. Restart it to save this“.

#### Code

- `linux/app/load.rs` (neu): Prozesse aus `/proc`, CPU aus `utime + stime`
  (Feld 14, 15) über die Zeit, RSS aus `statm` × Seitengröße; reine
  Rechenfunktionen getestet mit festen `/proc`-Zeilen.
- `linux/app/save.rs::poll_replay`: schickt `(held, load)`.
- `app/preview/replay.rs` bzw. neu `preview/load.rs`: `Preview::Load`,
  `App::set_replay_load`; `pages/mod.rs` hängt den Block ans Ende der
  Replay-Seite.
- `linux/app/save.rs` (Fehlertext) für Punkt 2.

#### Abnahme

Tests: CPU aus zwei `/proc/stat`-Ständen, Kindersuche, Text bei an, aus und
veraltet; Banner-Text bei „unknown setting“ eines bekannten Keys. Gates der
App. Render der Replay-Seite.

**Macht nicht:** GPU- oder Encoder-Last (nur NVIDIA abfragbar, und dann für
die ganze Karte), Schätzwerte für nicht laufende Einstellungen.

### Phase 5j: Hover-Fade der Karten als Schalter (27.10., Wunsch des Nutzers)

Wunsch: „mach noch als toggle fuer das shelf eine hover animation [...] das
solid on hover ist ja nicht animiert aber das soll per toggle moeglich sein,
so ein max 150ms fade in-out“.

Befund: Der Overlay schaltet beim Hover hart um (`draw_shelf` mit
`hovered: Option<u64>`: Deckkraft 1,0 und die Knöpfe sofort, beim Verlassen
sofort zurück; `linux/shelf/mod.rs` Motion/Leave setzen `self.hovered`). Die
Vorschau blendet dagegen immer mit `QUICK` (App-Fades), passt also bei
ausgeschaltetem Fade nicht zum Overlay.

Soll:
- Neuer Key `shelf.hover_fade` (Bool, Live, Standard **aus** = heute).
  Zeile „Fade on hover“ unter „Look“, nach „Solid on hover“.
- An: die Hover-Stärke einer Karte geht in **120 ms** linear von 0 auf 1
  und beim Verlassen zurück (unter den gewünschten 150 ms; linear, damit ein
  Wechsel mitten im Blenden von der erreichten Stärke weitergeht). Deckkraft
  `opacity → 1` (mit „Solid on hover“) und die Knöpfe blenden mit. Aus:
  wie heute sofort.
- Painter: `Frame::hovered` wird `&[(id, Stärke)]`; Deckkraft
  `mix(opacity, 1, Stärke)`, Knöpfe mit Alpha × Stärke.
- Overlay: `AnimKind::HoverIn/HoverOut` in der vorhandenen
  Animationsliste (Frame-Callbacks und Timer laufen dann schon),
  `Daemon::set_hovered` in `anim.rs`; Auftauchen und Verschwinden
  unberührt (`anim_factor`, Layout zählen nur Appear/Disappear).
- Vorschau: Hover-Fade der Karten mit 120 ms wie der Overlay, wenn an;
  sofort, wenn aus.

Abnahme: Painter-Test (Stärke 0,5 → Alpha mittig zwischen Ruhe und deckend,
Knöpfe halb), Daemon-Hilfsfunktion für die Stärke (an, aus, Wechsel mitten
im Blenden), Config-Test, Zeilen-Zählung, Gates, Render.

**Macht nicht:** andere Hover-Effekte (Größe, Schatten), andere Dauer als
Einstellung.

### Phase 5k: Kartenschatten ohne Streifen (27.10., Meldung des Nutzers)

Meldung (Bild): „der schatten bei den shelf cards hat so blur stripes, das
issue taucht generell oft bei blur effekten auf, in termo ist das aber nicht
so“.

Befund: termo blurt nicht selbst, es ist durchsichtig und Hyprland blurt
dahinter, mit Rauschen gegen Stufen (`decoration:blur:noise`). Der Kartenschatten
(`shelf_card/paint.rs::draw_card_shadow`) ist ein CPU-Verlauf: Alpha
`0,24 · (1 − d/16)` linear über 16 px, auf 8 Bit gerundet. Gemessen im
Render über Grau 64: eine Stufe je Pixel (glatt). Über einem dunklen Desktop
(≈ 0x12) macht der Compositor daraus `18 · (1 − a)`, also nur 18 → 14: vier
bis fünf Stufen über 16 px, je 3–4 px breit, die Streifen. Der Knick am Rand
der linearen Rampe wird als Kante gesehen (Mach-Band).

Soll: (1) weicher Abfall: `smoothstep` der Rampe (keine Ecke an Anfang und
Ende, Steigung 0); (2) Dithering beim Runden: Alpha × 255 plus eine Schwelle
aus „interleaved gradient noise“ (Jimenez 2014) je Pixel, abgerundet; im
Mittel exakt, als Rauschen unter einer Stufe statt als Band. Gleiche Stärke
(0,24), Lage (3 px tiefer) und Reichweite (16 px).

Abnahme: Test, dass das gerundete Alpha im Mittel über einen Streifen der
Rampe dem Sollwert auf ±0,5/255 entspricht und kein Lauf gleicher Werte
länger als 3 px ist; Bild über 0x12, Kontrast × 8 gestreckt, vorher und
nachher angesehen.

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
5. **Settings-Seiten, Tray-Einträge, `boltsnap settings`, Launcher-Eintrag
   mit Logo** (Abschnitt 2, Öffnen), README
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
| 2, Schritt 2 | umgesetzt 25.10.: `shelf/settings.rs` mit `Writer` (Thread, geordnete `mpsc`-Warteschlange, `settings::write`, Ergebnis als `DaemonEvent::SettingsWritten`), `refresh_recording_prefs` und `persist_tray_prefs` dorthin verschoben. Der Tray schreibt nur die Keys, die sich laut `config::pref_changes` ändern (Rest von CF4); `pending_writes` ersetzt die Generationen, ein Fehler gibt die Notification wie bisher und der Refresh nach dem letzten Job stellt den Stand der Datei wieder her. `refresh_recording_prefs` liest über `Config::load()`, `load_valid` ist entfernt; `save_recording_prefs` gibt es nur noch für Windows, `_at` für Windows und Tests. `shelf/mod.rs` 4016 → 3874 Zeilen. Test `pref_changes_name_only_the_changed_keys` neu, der Generationstest entfällt, der `LatestValue`-Test läuft mit Zahlen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 37 + 354 grün. |
| 2, Schritt 3 | umgesetzt 25.10.: `settings/state.rs` (`State`, `Broken`, `State::read`, JSON hin und zurück, `patch_to_json`/`patch_from_json`), `Request::GetSettings`/`SetSettings`/`SettingsWatch` und `Response::settings` in `protocol.rs`, `config::settings_state()`. Daemon (`shelf/settings.rs`): `send_settings`, `set_settings` (Job mit Client-Stream, höchstens 32 offen, sonst „busy“; Antwort nach dem Write mit frisch gelesenem Stand oder dem Fehler von `apply`), `watch_settings` (höchstens 16), `publish_settings` nach jedem Job, nur bei geändertem Stand. Windows-Shell: ein Arm „settings are not available on Windows“, nicht kompiliert (kein Target). `shelf/mod.rs` 3883 Zeilen. Tests: Stand bei kaputter Datei mit und ohne letzten gültigen Text, JSON-Roundtrip von Stand und Patch, falsche Patch-Formen, Protokoll-Roundtrip der drei Requests und einer `Response` mit Stand. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 354 grün. Live folgt nach Schritt 5. |
| 2, Schritt 4 | umgesetzt 25.10.: `watch_config` in `shelf/settings.rs` (eigener Thread, `inotify_init1(IN_CLOEXEC)`, Watch auf das Config-Verzeichnis mit `IN_CLOSE_WRITE \| IN_MOVED_TO \| IN_MOVED_FROM \| IN_CREATE \| IN_DELETE`, bei einem Symlink zusätzlich auf den Ordner des Ziels, nach jedem Burst neu aufgelöst; 150 ms Entprellung über `poll`; legt das Verzeichnis an). `IN_CREATE` kam gegenüber dem Detailplan dazu, weil `ln -s` sonst kein Ereignis gibt. Der Daemon startet ihn in `run_daemon`, `DaemonEvent::ConfigChanged` ruft `refresh_recording_prefs` und `publish_settings`; scheitert inotify, eine Logzeile und die alten Refresh-Punkte bleiben. Tests: Ereignis-Parser an einem gebauten Puffer, echter Watcher an einem Temp-Ordner (fremde Datei und `.lock` still, Schreiben und `settings::write` je ein Aufruf, ein Burst ein Aufruf, umgebogener Symlink und Edit am Ziel gesehen). Gegenprobe: ohne Watch auf das Symlink-Ziel schlägt der Test fehl. `shelf/mod.rs` 3895 Zeilen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 356 grün. |
| 2, Schritt 5 | umgesetzt 25.10.: `tray::default_targets` liefert `Focused display`, die Monitore, einen abgesteckten `output:X` als „X (disconnected)“ und `Both displays`; Beschriftung, Auswahl und Klick nutzen dieselbe Liste (CF5). Abweichung: Der bestehende Test `disconnected_default_marks_the_effective_focused_fallback` prüfte genau das Verhalten, das CF5 als Fehler einstuft; er prüft jetzt den „(disconnected)“-Eintrag. Zwei weitere Assertions verschieben sich um den neuen ersten Eintrag. Neuer Test `default_monitor_menu_offers_focused_and_keeps_an_unplugged_monitor`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 357 grün. |
| 2, live | geprüft 25.10. nach Install und Daemon-Neustart (keine Aufnahme lief), echte `config.toml` vorher gesichert: Handedit `record_show_frame` false→true→false schaltet den Tray-Haken ohne Aktion (über `busctl` aus `/MenuBar` gelesen), `settings_watch` liefert je eine Zeile; ein angehängter Syntaxfehler gibt `broken` mit Zeile 14 und weiter `record_fps = 120`, nach dem Zurückkopieren `broken: null`; `set_settings` mit `record_fps = 999` und unbekanntem Key abgelehnt, `record_show_frame` true und zurück über `set_settings` wirkt bis in den Tray; Datei am Ende byte-gleich mit der Sicherung. Tray zeigt „Focused display“. Befund dabei: zwei kaputte Saves mit demselben Fehler gaben zwei Meldungen, weil `Remembered` nach Text entdoppelte. Abweichung von Phase 1 Schritt 4: `Remembered` merkt sich jetzt die Meldung (Zeile und Grund), dieselbe Fehlerstelle wird nach weiteren Edits nicht erneut gemeldet; Test erweitert, live erneut geprüft: zwei kaputte Saves, eine Meldung. Nicht live geprüft: eine Aufnahme mit kaputter Datei (Werte kommen aus demselben `Remembered`, `get_settings` zeigte 120). |
| Review, Fix 1 | umgesetzt 25.10. (SU3, SU4, SU5): `read_noted` liest und merkt unter einem Lock, `NotFound` als `""`, andere Lesefehler als `Noted::Unreadable` ohne Merken; `load` nimmt dann den letzten gültigen Stand, `replay_settings` gibt den Fehler wie bisher, `settings_state` meldet `broken` mit „cannot be read: …“. Test `reading_notes_a_deleted_file_and_keeps_the_last_text_when_unreadable`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 358 grün. |
| Review, Fix 2 | umgesetzt 25.10. (SU6, SU7, SU10, SU11): `apply` legt für `Remove` keine fehlende Tabelle an; auf oberster Ebene zählt ein Schema-Key nur ohne Tabellenteil, `"replay.fps" = 30` ist „unknown setting“; `replace` öffnet die Temp-Datei vor dem Aufräum-Pfad und löscht nur eine selbst angelegte (`fill` ausgelagert); Lock-Kommentar korrigiert. Tests erweitert (`apply_adds_missing_keys_and_tables_and_removes_keys`, `unknown_keys_are_reported_and_known_ones_still_read`). Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 358 grün. |
| Review, Fix 3 | umgesetzt 25.10. (SU1): `StartRecording` schreibt nichts mehr; `start_recording` nimmt `Geometry`, `show_frame` und `audio_enabled` und nutzt die Werte nur für diese Aufnahme (`RecordingPrefs { audio_enabled, ..}` für das erste Segment). Ein im Selector gescheitertes Speichern meldet jetzt der Selector selbst als Notification „Recording settings could not be saved: …“ und die Aufnahme startet; damit stimmt der Satz im Status von Phase 1 Schritt 5 („ein Tray-Wert … bleibt“) auch für eine gestartete Aufnahme. Kein neuer Test (Daemon-Pfad braucht Wayland), Pfad am Code geprüft. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 358 grün. |
| Review, Fix 4 | umgesetzt 25.10. (SU2, SU8, SU9): `events` liefert Maske mit, `Watches::note` zählt `IN_Q_OVERFLOW` als Änderung und vergisst einen Watch bei `IN_IGNORED`; fällt der Ordner-Watch weg, wartet der Thread mit `poll` 1 s und versucht neu, nach Erfolg `changed()`; Entprellung 150 ms Pause, höchstens 1 s nach dem ersten Ereignis; `add_watches` gibt den Ordner-Watch zurück. Tests: Parser mit Maske, `note` bei `IN_IGNORED`, echter Watcher: Ordner gelöscht (ein Aufruf), neu angelegt und beschrieben (Aufruf), danach weiterer Edit (Aufruf). Gegenprobe: ohne Neuversuch schlägt der Test fehl. `shelf/settings.rs` 477 Zeilen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 358 grün. |
| 3 | umgesetzt 26.10. nach „Phase 3 im Detail“ (Abschnitt 3), Schritte unten; offen: Augenschein der Gleit-Bewegung oben durch den Nutzer |
| 3, Schritt 1 | umgesetzt 26.10.: `[shelf]` in `TABLES`, Keys `shelf.card_width` (140 bis 380), `shelf.visible_cards` (1 bis 8), `shelf.corner` (vier Ecken), alle `Live`; `CardSize` (`DEFAULT`, `from_width`, `dims`) und `rescale` in `shelf/thumbnail.rs`, `Corner` in `shelf/layout.rs`, `ShelfPrefs` und `Config::shelf_prefs`. Tests: Höhen 97/132/208/264 und Grenzen, `rescale` von 1×1 und 190×132, `Corner::parse`, `[shelf]` gültig, ungültig (Default plus drei `Issue`s), auf oberster Ebene unbekannt. Abweichung: `cargo clippy` meldet bis Schritt 3 und 4 Dead-Code für die noch ungenutzten Typen, sonst keine Warnung. Checks: `cargo fmt --check`, `cargo test` 41 + 362 grün. |
| 3, Schritt 2 | umgesetzt 26.10.: `linux/shelf/cards.rs` (238 Zeilen) mit `visible_cards`, `overflow_card_opacity`, `animated_item_limit`, `animated_card_layout`, `prepare_add`, `write_temp_png`, `prepare_pixels`, `prepare_pixels_with`, `decode_shelf_image`, `shelf_png_decoder`, nur verschoben (`pub(super)`, Pfade auf `super::super::…`). Abweichung: `tests.rs` braucht eine Importzeile für `decode_shelf_image` und `prepare_pixels_with`, weil `mod.rs` nur importiert, was es selbst nutzt. `shelf/mod.rs` 3890 → 3664 Zeilen, `linux/shelf/` 11 Dateien. Checks: `cargo fmt --check`, `cargo test` 41 + 362 grün, clippy nur die Dead-Code-Meldungen aus Schritt 1. |
| 3, Schritt 3 | umgesetzt 26.10.: `Daemon::shelf_prefs` (Start aus derselben `Config::load()` wie die Recording-Prefs); `MAX_VISIBLE_CARDS` entfällt, `visible_cards`, `overflow_card_opacity`, `animated_item_limit` und `animated_card_layout` nehmen `visible`, letztere zusätzlich `top` (Scroll auch bei nicht vollem Shelf). `cards::anchor(Corner)` liefert Anker und Ränder, `place_on_output` nutzt es; `apply_shelf_prefs` macht `relayout`, baut bei neuer Ecke den Layer auf demselben Monitor neu (vorher `forget_drag`, statt `clear_drag` wie geplant, weil danach ohnehin gezeichnet wird) und zeichnet. `refresh_recording_prefs` heißt `refresh_prefs` und wendet beides an. Tests: Viewport mit 1 und 8 Karten à 380×264 (Höhe, Breite, keine Überlappung, Limit), Overflow-Deckkraft mit `visible` 1, oben verankert gleiten die älteren Karten eine Stufe nach unten, unten bleiben sie, Anker aller vier Ecken. `shelf/mod.rs` 3681 Zeilen. Checks: `cargo fmt --check`, `cargo test` 41 + 365 grün, clippy nur noch Dead-Code für `dims`/`rescale` (Schritt 4). |
| 3, Schritt 4 | umgesetzt 26.10.: `cards::CARD_WIDTH` (`AtomicU32`) mit `use_card_size`/`card_size`, beim Start aus der Config gesetzt; `apply_shelf_prefs` skaliert bei neuer Größe alle Karten im Eventloop (`resize_cards`, Logzeile nur über 8 ms) und gibt sie an `Worker::rerender`. Worker: `Job::Video | Job::Rerender`, geteilte Liste (gleiche `id` ersetzt), nach jeder Nachricht abgearbeitet; Größe beim Rendern über eine übergebene Funktion gelesen, `render` für Bild (`decode_shelf_image` + `make_image_card_thumbnail`) und Video (`extract(path, size)`). `ThumbnailReady` übernimmt nur in aktueller Größe; `add_prepared` ruft `fit_card` (Stufe 1 plus Re-Render für eine Karte alter Größe). `prepare_add` und `prepare_pixels_with` bekommen die Größe, der Client-Thread übergibt `card_size()`; Platzhalter per `RgbaImage::from_pixel`; CLI rendert mit `Config::load().shelf_prefs().card`. Abweichung: `make_card_thumbnail` ist unter Linux nur noch in Tests, Windows und `examples/image_transfer_bench.rs` in Gebrauch und trägt dafür ein kommentiertes `allow(dead_code)` (ein `cfg` hätte das Example gebrochen). Tests: `prepare_add` verwirft einen Client-Thumb in 190×132, wenn 300 gilt; Worker-Test mit kleiner PNG (300×208, fehlende Datei `None`, doppelte `id` einmal); der ignorierte ffmpeg-Test prüft 300×208 und lief einmal grün mit `--ignored`. `shelf/mod.rs` 3677 Zeilen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 366 grün. |
| 3, Schritt 5 | umgesetzt 26.10.: README (`[shelf]` unter Konfiguration, Ecke im Shelf-Abschnitt), `docs/codebase.md` (`cards.rs`). Live nach Install und Daemon-Neustart (keine Aufnahme lief), echte `config.toml` vorher gesichert: zwei Karten (`boltsnap full`, 2-s-Testvideo per `add_video`), Layer 234×318; `set_settings` `shelf.card_width = 300` → `hyprctl layers` 344×470, Ausschnitt eines Screenshots zeigt beide Karten scharf in 300 px (harte Testbild-Kanten, lesbarer Text); `visible_cards = 1` → 344×252; `corner = "top-right"` → Layer bei x 1552, y 24 auf DP-3 (1920 − 24 − 344); `visible_cards = 2` plus neuer Screenshot → 344×470, neueste Karte oben, Video darunter, die dritte im Cache; Backup zurückkopiert (Handedit über den Watcher) → unten links, 234×460 für drei Karten. Datei am Ende byte-gleich mit der Sicherung, `daemon.log` leer. Abweichungen: kein gespeicherter Screenshot als dritte Karte (eine Bild-Karte behält nach dem Speichern ihre temporäre Datei, der Re-Render-Pfad ist derselbe); Screenshots per `boltsnap full -o` statt `grim` (nicht installiert), das hat nebenbei die Zwischenablage überschrieben. Nicht geprüft: die Gleit-Bewegung oben (braucht den Nutzer), die Logzeile für Stufe 1 über 8 ms kam bei drei Karten nicht. |
| Entfernen ruckelt | umgesetzt 26.10. nach „Nebenbei: ruckelndes Entfernen“ (Abschnitt 3): `cards::hold_size`, `animated_layout` hält die Layer-Größe, solange eine Karte verschwindet, `draw` fordert nur einen Frame-Callback gleichzeitig an, `place_on_output` setzt ihn zurück. Test `a_leaving_card_keeps_the_layer_size_and_the_stack_on_its_anchored_edge`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 41 + 367 grün; installiert, Daemon neu gestartet. Offen: der Nutzer schließt eine Karte und beurteilt die Bewegung (kein Klick-Tool hier) |
| 4 | umgesetzt 26.10. nach „Phase 4 im Detail“ (Abschnitt 5), Schritte unten; offen: Augenschein und Bedienung durch den Nutzer |
| 4, Schritt 1 | Probe gelaufen 26.10. (`examples/activation_probe.rs`, danach entfernt; Skript fokussiert vor jedem Versuch das vorher aktive Fenster `termo` zurück und am Ende wieder): `xdg_activation_v1` ist da; Token ohne Serial plus `activate` → aktiv bleibt `termo`, kein Fokus. Neu-Mappen (neues `xdg_toplevel`, altes verworfen) → aktiv ist `boltsnap-probe`. `urgent` steht nicht in `hyprctl clients -j`, ließ sich also nicht messen. **Entscheidung: Weg (b), Neu-Mappen.** |
| 4, Schritt 2 | umgesetzt 26.10.: `src/ui/` (Lib) mit `mod.rs` (`Rect`, `Canvas` mit Scale, abgerundete Flächen), `theme.rs` (Farben, Schriftskala Title/Label/Heading/Hint/Key, Abstände, Maße, Dauern), `text.rs` (`Fonts`, Breite, Versalhöhe aus dem `H`-Umriss, Baseline auf Versalien-Mitte, Ellipse, Caret-Positionen, Zeichnen premultipliziert), `icons.rs` (Sprite-Parser für die sieben Elemente, Pfadbefehle samt kompakter Zahlen und Bögen nach F.6.5, Zeichnen auf ganze Pixel mit Strich 1,5), `motion.rs` (Minimum-Jerk, `Tween` mit Retarget samt Geschwindigkeit, `Scroll`); `assets/icons/ui/lucide.svg` mit 20 Symbolen plus `LICENSE`. Abweichungen: Sprite statt Einzeldateien (siehe Code-Liste); der eingebettete Font hat kein „…“, `ellipsize` nimmt dann drei Punkte. Tests: 10 (`ui::tests`). Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test --lib ui::` grün. |
| 4, Schritt 3 | umgesetzt 26.10.: `src/ui/widgets/` (Trait `Widget` mit `Look`, `Outcome`, `Value`; Schalter, Segment, Dropdown mit eigener Liste im Fenster, Slider, Stepper, Textfeld, Icon-Button, Banner), `Input`/`Key`/`Mods`/`Effect` und `Clicks` (Doppelklick aus Wayland-Zeiten) in `ui/mod.rs`, Text mit Clip; `src/app/` mit `App` (Navigation mit gleitender Auswahl, Blöcke Heading/Row/Banner/Specimen, Hover-Fades, Fokus und Tab-Reihenfolge, Zeiger-Capture beim Ziehen, Overlay zuerst bei Klicks, Scroll mit Reveal), `draw.rs`, `gallery.rs` (Seiten Widgets und Text, je Widget eine deaktivierte Variante); `linux/app/mod.rs` mit `boltsnap settings --render PAGE OUT.png [--scale N]`, Windows meldet „settings are not available on Windows“ (inline in `windows/mod.rs`, der Ordner hat schon 12 Dateien). Abweichungen 6 bis 8 oben. Offscreen-Render beider Seiten bei Scale 1 und 2 angesehen: Rand und Achsen gleich, Icons auf Versalien-Mitte, Ellipse mit „…“ im echten Font. Tests: 14 (`app::tests`: Geometrie, Tab-Reihenfolge, je Widget Maus und Tastatur, Doppelklick, Hover-Fade und Verlassen, Seitenwechsel und Schließen, Scroll bis zum Ende, Zeichnen bei Scale 1 und 2). Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 65 + 367 grün, `git diff --check`. |
| 4, Schritt 4 | umgesetzt 26.10.: `linux/app/window.rs` (SCTK-Fenster App-ID `boltsnap`, Server-Deko angefordert, Minimum 560×400, `SlotPool`, ganzzahliger Buffer-Scale aus `scale_factor_changed`, jede Zeichnung fordert einen Frame-Callback an und die nächste wartet darauf, Copy über einen Thread mit `wl-clipboard-rs` im Vordergrund, Paste über einen Thread zurück per calloop-Kanal), `input.rs` (Zeiger mit `ThemedPointer` und `Clicks`, Rad mit `discrete`/`absolute`, Tastatur mit Wiederholung, `key_input`), `single.rs` (Lock `boltsnap-app.lock` über `ipc::lock_file`, Socket `boltsnap-app.sock`, Zeile `raise <page>`; Abweichung: Namen mit Präfix `boltsnap-app.`, weil sie direkt in `XDG_RUNTIME_DIR` liegen), `boltsnap settings [PAGE]`. Live mit dem Debug-Build: Fenster erscheint und zeichnet die Galerie; zweiter Aufruf `boltsnap settings text` endet nach 8 ms, aktiv wird `boltsnap`, weiter genau ein Fenster und derselbe Prozess, Seite Text, Layout folgt der gekachelten Breite 946 px; Schließen per `hl.dsp.window.close` beendet den Prozess (xdg `close`), Fokus danach zurück auf das vorherige Fenster. Screenshots per `boltsnap full --no-copy -o` (Zwischenablage unberührt). Tests: `key_input` (Tab mit Shift, Space, Ctrl+Buchstabe trotz Steuerzeichen, Umlaut als Text, Modifier allein), `read_raise`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 65 + 369 grün. Offen durch den Nutzer: Hover flackert nicht, Tastatur erreicht alles, Scroll gleitet. |
| 4b | umgesetzt 26.10. nach „Phase 4b: Überarbeitung nach `/ui`“ (Abschnitt 5). Federn (`Spring`, analytisch, kritisch gedämpft, Retarget mit Geschwindigkeit, Ende über Schwellen) ersetzen `Tween`; Tokens `QUICK` 150, `SNAP` 200, `GLIDE` 300 ms. Hover, Druck und Fokus je Ziel über `app/fades.rs`; Segment-Teile und Stepper-Knöpfe leuchten mit eigener Feder, die Dropdown-Markierung gleitet, eine neue Seite blendet ein und steigt 6 px. `app/layout.rs`: Spalte (höchstens 680, mittig), Gruppenflächen, Zeilen 46, Stapeln, schmale Navigation unter 600 px; Mindestgröße 360×320. `app/pages.rs` mit den echten Werten (`linux/app/prefs.rs` aus `Config::load()`), Banner „Preview: changes here are not saved yet“; Galerie als Stress-Fixture. Text mit Kerning, geschütztes Leerzeichen, Pfade ohne Fokus in der Mitte gekürzt. Sprite um 13 Lucide-Symbole erweitert (`history` heißt im Sprite `rotate-ccw-clock`), `layout-grid` und `type` entfallen. Abweichungen: Segment-Teile werden bei Platzmangel nach Inhalt breit statt gleich breit (sonst stapelt „System and mic“ schon bei 760 px mit dem eingebetteten Font); Zeilennamen nach README-Bedeutung („Show saved videos on the shelf“, „Keep long shelf saves“, „Corner“, „Start with Boltsnap“); `--measure` als CLI für die Messung mit dem echten Font statt eines `#[ignore]`-Tests. Gates (gemessen): Überlauf 399 Teile in ihrer Gruppe bei 360/480/760/1400 px, jede offene Liste im Fenster; Versalien-Abstand oben gegen unten 0,00 px je Control-Art mit eingebettetem Font und mit Outfit (`--measure`), Gegenprobe erkennt 1 px Versatz als 2,0; Gruppen-Innenabstand 12 an allen vier Seiten (Abweichung 0,00), gemeinsame Mittellinie Abweichung 0,00; Kontrast 12 Paare, niedrigstes 4,83:1 (`SUB` auf `HOVER`); Copy 66 Strings ohne Treffer; Screenshots bei 360, 760 und 1400 angesehen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 75 + 369 grün, `git diff --check`. Live (Debug-Build, gekachelt 946×1032): Fenster zeichnet die Recording-Seite, Hover hellt Segment-Teil und Dropdown auf (Fläche 38 → 46), Verlassen nimmt ihn zurück; Fokus und Cursor danach zurückgestellt. Nicht live prüfbar: Bewegung innerhalb einer Fläche (Hyprlands `cursor.move` meldet dem Client nur das Betreten), Klicks, Resize per Maus. Offen: „Ohne Animation“-Liste durch den Nutzer bestätigen; Augenschein durch den Nutzer. |
| 4c | umgesetzt 26.10. nach „Phase 4c: Harte Wechsel an Schwellen“: M1 bis M8 wie in der Tabelle, Schrift auf ganze Versalhöhen mit Grundlinie auf ganzen Pixeln, Icons 14. Tests neu in `app/shape_tests.rs` (7): Navigation und Zeile halbwegs und exakt am Ende, Retarget zurück, Reduced Motion springt, neue Seite gestapelt ohne Bewegung, Name des aktuellen Eintrags blendet, Scroll-Daumen blendet ein, Segment-Breiten in 1-px-Schritten ohne Sprung > 1 px, Stepper-Knopf blendet an der Grenze. Gates: Überlauf 399 Teile bei 360/480/760/1400, Versalien 0,00 px je Control (eingebettet und Outfit über `--measure`), Innenabstand 12 (Abweichung 0,00), Kontrast niedrigstes 4,83:1, Copy 66 Strings ohne Treffer; Renders bei 360, 946 und 1400 angesehen. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 82 + 371 grün. Offen: Ohne-Animation-Liste durch den Nutzer. |
| 5b | umgesetzt 26.10. nach „Phase 5b: Zeilen speichern, Replay-Puffer schalten“: `app/writes.rs` (`Bind`, `Write`, Erkennung über den zuletzt gemeldeten Wert, Drossel 100 ms beim Ziehen, `finish` beim Schließen, `reload`), `Effect::Changed`, alle 17 Zeilen gebunden, Zeile „Replay buffer“, `Notice` (Fehler-Banner, gesperrte Zeilen), Preview-Banner entfernt; `linux/app/save.rs` (geordneter Schreib-Thread, wartet beim Schließen, `notice()` aus `get_settings`, Replay-Status); Replay meldet `ReplayStateChanged` nach `start`/`stop`. Aufgeräumt: Tests nach `app/tests/` (`mod.rs`, `gates.rs`, `shape.rs`, `writes.rs`), Tastatur nach `app/keys.rs` (`app/mod.rs` 448 Zeilen). Tests neu (8): Schalter schreibt einmal, Dropdown schreibt gelisteten Wert und Segment sein Wort, Slider gedrosselt, Textfeld erst beim Bestätigen, Schließen übernimmt die Eingabe, Replay-Schalter, jede gebundene Zeile vom Schema angenommen (17), gesperrte Zeilen schreiben nichts und `reload` behält die Seite. Gates laufen zusätzlich mit gesperrtem Fehler-Banner (langer Pfad). Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 90 + 371 grün. Live (Release, Daemon neu gestartet, Config vorher gesichert und danach byte-gleich zurück, sha256 9efb7cd3…): Tasten per `hl.dsp.send_shortcut`, auf „Cards shown“ schreibt `→` `[shelf] visible_cards = 6`, Fenster zu und wieder auf zeigt 6; „Replay buffer“ per `Space` an: Status `running`, Tray-Haken `toggle-state 1`; wieder aus: `stopped`. Offen wie im Abschnitt „Macht nicht“. |
| 4d | umgesetzt 26.10. nach „Phase 4d: Ruckeln“. P1 Canvas bleibt, zwei Canvases wechseln sich ab (der Vergleich braucht keine Kopie). P2 `Abgr8888`: Hyprland bietet es hier nicht an, es bleibt beim Byte-Tausch, der Pfad greift, wo es angeboten wird. P3 Glyphen-Cache: Text 63 → 19 µs je 30 Zeichen, `App::draw` in Ruhe 1,7 → 0,95 ms. P4 Schaden nur im geänderten Rechteck, nichts geändert heißt kein neuer Puffer. Zusätzlich gefunden: die Seitenblende über tiny-skia (Fläche mit Alpha über den Inhalt) kostete allein 1,86 ms pro Frame; jetzt `Canvas::fade`, das Fensterfarbe überspringt: Frame mitten in der Animation 2,53 → 1,03 ms. Live (Release, 946×1032, 200/240 Hz, vier Seitenwechsel): Zeichnen bis Commit Median 3,43 → 2,16 ms, p90 4,19 → 2,66 ms; Abstände Median 4,92 ms. Abweichung von der Abnahme: Median 2,16 statt ≤ 1,5 ms, und es bleiben Lücken über 7 ms (27 von 336), gehäuft am ersten Frame nach dem Tastendruck und an Frames mit 1×1 Pixel Schaden, also nicht durch Rechenzeit; offen, ob der Nutzer sie sieht. Tests: Schadensrechteck, `fade` (Fensterfarbe exakt, Mischung), Cache zeichnet gleiche Pixel. Checks: fmt, clippy ohne Warnung, `cargo test` 92 + 372 grün. |
| 4e | umgesetzt 26.10. (Meldungen des Nutzers: „fonts verruecken … beim scrollen“, Icon und Schrift „nicht so hingerotzt“, `/ui` um nachgemessene Geometrie ergänzt). Ursache des Verrückens: Text rastet seit 4c auf ganze Pixel, Scroll, Einblend-Hub, Spalte und Zeilen-Interpolation liefen in Bruchteilen, also sprang Text gegen seine Fläche. Jetzt bewegt sich die Seite in ganzen Pixeln (`layout_at`, `column`, `lerp_rect`, Höhe beim Stapeln, Icon-x der Navigation). Test `the_page_moves_in_whole_pixels…` über Scroll, Resize und zwei Seitenwechsel, Gegenprobe ohne Runden schlägt an. Gate 8 neu (`measure::icon_rows`, `icon_gate`, `ink_box`): Icon-Tinte auf der Versalien-Mitte, eine Achse, eine Textkante, `GAP_S` vom breitesten Icon, je Rolle (Zeilen aller Seiten, Navigation). Gemessen, eingebettete Schrift: 22 Zeilen, schlechteste Mitte 0,32 px, Achse 0,31, Kante 0,00, Abstand Token + 0,37; Outfit (`--measure`): 0,31 / 0,31 / 0,00 / 0,37. Gegenprobe: 1 px Versatz eines Icons misst 1,00. Gates: Überlauf jetzt 627 Teile (mit Fehler-Banner-Seiten), Kontrast 4,83:1, Copy 63 Strings, Abstände 0,00. |
| 5a-Fix | 26.10., Meldung des Nutzers „Settings: no such file or directory (2)“ beim Tray-Eintrag. P1, reproduziert: der Daemon (pid 1172494) lief aus dem Binary, das `cargo install` danach ersetzt hatte; `/proc/1172494/exe` zeigte auf `/home/mt/.cargo/bin/boltsnap (deleted)`, `current_exe()` liefert genau das, `spawn` scheitert mit ENOENT. Betraf auch „Start region recording“ im Tray und Kopieren einer Karte in die Zwischenablage (beides startet das eigene Binary). Fix: `paths::own_exe()` nimmt den Pfad ohne „ (deleted)“, wenn dort eine Datei liegt (das neue Build), sonst `current_exe()`; genutzt in `tray.rs`, `shelf/mod.rs` (Region), `clipboard.rs` (2×), `ipc.rs`. Abwägung: das neue Binary startet neben einem alten Daemon; das ist genau das, was ein frisch geöffnetes Fenster auch täte. Test `a_replaced_binary_is_started_from_its_original_path`. Der laufende Daemon hat den alten Code bis zu seinem Neustart. |
| 4f | 26.10., Meldung des Nutzers: Animationen „droppen die kompletten fps“, gemeint beim Resizen, wenn die Navigation einklappt. Zwei Ursachen, gemessen mit einem nachgestellten Zieh-Resize (`hl.dsp.window.resize`, Breite 900 → 420 → 900 in 4-px-Schritten alle 4 ms, 241 configures): (1) der configure-Handler setzte `frame_pending = false` und zeichnete sofort, also ein kompletter Frame je Zeiger-Event außerhalb des Bildschirmtakts (wie beim Shelf, 8313b18); (2) `SlotPool::create_buffer` erzeugte jeden Frame einen neuen `wl_buffer` (für den Compositor neue Textur und voller Upload, egal welcher Schaden). Fix: configure merkt sich nur Größe und Schaden und zeichnet erst, wenn kein Frame unterwegs ist; bis zu drei Puffer werden wiederverwendet, sind alle beim Compositor, kommt einer dazu und der älteste geht (eine erste Fassung, die dann nur den nächsten Frame anforderte, lief in eine Leerlauf-Schleife: 1529 umsonst gezeichnete Frames in 1,9 s, gemessen und verworfen). Ergebnis: CPU des Fensterprozesses beim Resize 96,4 / 96,2 % eines Kerns (installierter Stand) → 36,8 / 35,9 % (zwei Läufe); Commits im Takt, Median 5,2 ms. Hyprlands eigene CPU ist hier kein Maß, weil jeder `hyprctl`-Aufruf der Simulation selbst Last erzeugt. Kein Unit-Test (braucht einen Compositor); Abnahme ist diese Messung. |
| 4f-Folge | offen, Verdacht aus 4f, nicht gemessen: der Shelf (`shelf/mod.rs:2030`, Layer-Draw) und der Selector (`select_skia/mod.rs:628`) legen ebenfalls je Zeichnung einen neuen `wl_buffer` über `create_buffer` an; bei Shelf-Animationen hätte der Compositor dieselbe Mehrarbeit. P2 bis gemessen. Messung wie in 4f (CPU des Daemons, Commit-Abstände) beim Erscheinen und Entfernen einer Karte, dann gleicher Fix. |
| 5c S1 | umgesetzt 26.10.: `linux/buffers.rs` (`Buffers`, bis zu drei je Größe, freier zuerst, sonst neu und der älteste geht), genutzt vom Fenster (ersetzt die Logik aus 4f) und vom Shelf-Draw (`shelf/mod.rs` bleibt bei 3681 Zeilen). Test `a_released_buffer_is_reused_before_a_new_one_is_made`. Checks: fmt, clippy ohne Warnung, `cargo test` 95 + 374 grün. Live nicht gemessen (siehe S1 Abnahme: Versuch brach den Desktop des Nutzers aus, abgebrochen); greift im Shelf erst nach einem Daemon-Neustart. |
| 5c S2 | umgesetzt 26.10. nach „S2 im Detail“ (Abweichung dort begründet: Kontur außen statt innen; `:focus-visible`-Verhalten gleich mit eingebaut). `Canvas::ring`, Token `FOCUS_W = 2`, `widgets::focus_ring`; Fläche hebt nur noch bei Hover; Schalter um die Spur, Slider um den Knopf mit 2 px Abstand, Navigation um die Auswahl; `App::keyed` (Feder `QUICK`): Tab und Pfeile in der Navigation zeigen, ein Klick blendet aus. Gemessen: Kontur auf Gruppe 6,06:1, auf Seitenleiste 6,37:1, gegen das Control 5,38:1 (vorher Fokus 1,11:1). Test `tab_shows_the_outline_and_a_click_hides_it` (Pixel 1 px links des Dropdowns), Render der fünf Arten angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 96 + 374 grün. |
| 5c S3 | umgesetzt 26.10. nach „S3 im Detail“. `Fonts::wrap` (Wörter, nach `/`, `-`, `_`, sonst zwischen Zeichen, eine Cursor-Liste je Wort), Banner bis 4 Zeilen (Abstand 20, eine Zeile wie bisher 46 hoch), Namen und Hinweise bis 3 Zeilen (Abstand ab zwei Zeilen 18/16), Zeilenhöhe je Anordnung (`side`, `stacked`), die Feder mischt zwischen beiden. Abweichung, beim Test gefunden: die Anordnung neben dem Control lässt dem Namen mindestens `LABEL_MIN` zum Umbrechen, sonst brach ein zu schmaler Name Buchstabe für Buchstabe und die Zeile sprang beim Stapeln von 46 auf 70 px. Gates: neu `gate_text_wraps_instead_of_being_cut` (238 Zeilen bei 360/480/760/1400, echte Seiten und Fehlerzustand, keine gekürzt; Gegenprobe mit einer Zeile schlägt an „Outline around a recorde...“); Innenabstand misst oben die höhere von Control und erster Textzeile, unten die tiefere; Überlauf 626 Teile, Abstände 0,00, Versalien 0,00, Icons 0,31/0,31/0,00/0,37 (Outfit). Render bei 360 px und Stress-Banner angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 98 + 374 grün. |
| 5c S4 | umgesetzt 26.10. nach „S4 im Detail“ (Abweichung: nur Schalter-Zeilen ganz klickbar). `Widget::row_click` (Schalter `true`), `App::target_at` nimmt die ganze Zeile für solche Controls; Klick und Hover laufen über den bestehenden Pfad. Test `a_click_on_a_switch_rows_name_switches_and_other_names_do_nothing` (Hover hellt den Schalter auf, Klick schreibt `record_show_frame`, Klick auf den Namen „Frame rate“ schreibt nichts und fokussiert nichts). Checks: fmt, clippy ohne Warnung, `cargo test` 99 + 374 grün. |
| 5c S5 | umgesetzt 26.10. nach „S5 im Detail“: `MENU_MIN = 120`, `Dropdown::width` mindestens so breit. Gate `gate_menus_share_a_left_edge`: 16 Dropdowns (Recording und Replay, Fehlerzustand, 760 und 1400 px) je Seite auf einer Kante; Gegenprobe ohne Mindestbreite schlägt an („only 0 dropdowns found“, das Gate verlangt mindestens 8, damit es nicht leer besteht). Render angesehen (Kanten bei 548 px). Checks: fmt, clippy ohne Warnung, `cargo test` 100 + 374 grün. |
| 5c S6 | umgesetzt 26.10. nach „S6 im Detail“ (Abweichung dort: Werte weiter aus `prefs::load()`, die Watch-Zeile ist Auslöser). `Widget::adopt` für Schalter, Segment, Dropdown, Stepper (gleiten bzw. blenden), Slider und Feld (springen); `App::update` (gleiche Struktur → Werte übernehmen, fokussierte, gezogene und Zeilen mit offenem Schreibauftrag bleiben, übernommene Werte gelten als gemeldet; sonst `reload`); `ipc::watch_settings`, `save::watch` (Thread, neu alle 2 s, wenn der Daemon weg ist); Schreibaufträge tragen (Seite, Block), das Fenster zählt offene je Zeile. Tests (`app/tests/live.rs`, 3): Wert aus dem Tray gleitet und wird nicht zurückgeschrieben; fokussiertes Feld und Zeile mit offenem Auftrag bleiben; neue Notice baut auf derselben Seite neu. Checks: fmt, clippy ohne Warnung, `cargo test` 103 + 374 grün. Live nicht geprüft (bräuchte einen Tray-Klick bei offenem Fenster und einen Screenshot; nach Absprache). |
| 4g | umgesetzt 26.10. nach „Phase 4g“. Beim Test zusätzlich gefunden: das Icon schwang beim Einklappen erst 17 px nach rechts (x 20 → 37) und dann zurück auf 15, weil zwischen „mittig in der schrumpfenden Fläche“ und „links eingerückt“ gemischt wurde; jetzt zwischen den Ruhelagen 20 und 15, ohne Runden. Name an fester Stelle (x 42). Test `closing_navigation_icons_glide_between_pixels_and_names_stay_put` (61 Proben alle 5 ms: monoton, ≥ 20 verschiedene x, Ruhelagen ganzzahlig). `/ui`-Skill um kurze Wege und Mischen zwischen Ruhelagen ergänzt. Checks: fmt, clippy ohne Warnung, `cargo test` 104 + 374 grün; Icon-Gate mit Outfit unverändert. |
| 5c S7 | umgesetzt 26.10. nach „S7 im Detail“ (Abweichung dort: während einer Aufnahme ein Banner statt Hinweisen an sechs Zeilen). Feste Hinweise „Applies to the next recording“ und „Applies when replay restarts“ entfernt; Banner „A recording is running, changes here apply to the next one“; Zeile „Restart replay“ (Icon-Knopf, `Write::RestartReplay` = stop, start) nur bei laufendem, veraltetem Puffer. Veraltet: `seconds` oder `budget_bytes` (= `memory_mib · 2²⁰ / 3`, `worker/src/live.rs:108`) passen nicht zur Datei, oder in dieser Sitzung wurde ein Restart-Key geschrieben. Zweiter Watch-Thread auf `watch_recording`, signalisiert nur bei geändertem `state`. Texte ohne Schlusspunkt (Projektregel, vom Copy-Gate gefunden). Gates laufen jetzt auch im Zustand „Aufnahme läuft, Puffer veraltet“: Copy 120 Strings, Überlauf 854 Teile, Umbruch 297 Zeilen, Dropdowns 24. Tests: Banner nur bei Aufnahme und keine Zeile wiederholt ihn; Restart-Zeile nur bei laufend und veraltet, Klick liefert `RestartReplay`. Render beider Zustände angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 106 + 374 grün. Live nicht geprüft (Aufnahme und Replay-Neustart nach Absprache). |
| 4h | umgesetzt 26.10. nach „Phase 4h“: Navigations-Icon fest auf x 16 in 14 px (eingeklappt mittig im 30er Quadrat, ausgeklappt 8 px eingerückt wie oben und unten), Name ab x 38; `PAGE_RISE` und `App::rise` entfernt, `layout_at` ohne Hub. Tests: `navigation_icons_keep_their_place_and_size_while_it_closes` (61 Proben gleich, Quadrat, Icon mittig), `a_new_page_fades_in_where_it_stays`. `/ui`-Skill: Regel „Keine Mini-Wege“ mit O-Ton, ersetzt die Gleit-Ausnahme aus 4g. Icon-Gate mit Outfit grün. Checks: fmt, clippy ohne Warnung, `cargo test` 106 + 374 grün. |
| 4i | umgesetzt 26.10. nach „Phase 4i“: Segment-Spur ohne Hover und Druck, Fokus-Kontur bleibt. Test `a_segment_track_does_not_light_up_when_pressed`, Gegenprobe mit altem `Look` schlägt an. Checks: fmt, clippy ohne Warnung, `cargo test` 107 + 374 grün. |
| 5c S8 | umgesetzt 26.10. nach „S8 im Detail“ (Abweichung dort: Knopf im Textbereich statt einem Platz, der das Control verschiebt). `Row::reset`, `Prefs::set_keys` (aus `settings_state().doc`, neu `Doc::keys`), `Target::Reset` (Hover, Druck, Fokus-Kontur, Tab vor dem Control), `RowParts::reset` und `reset_place`, Kanal `Shown` (blendet mit `QUICK`, beim Öffnen sofort), `Effect::Reset` → `Write::Reset(key)` → `Change::Remove`; der Default kommt über S6 zurück. Test `a_set_key_offers_a_reset_that_leaves_the_control_where_it_is` (nur der gesetzte Key, Control-Rechteck gleich, Knopf `GAP_S` davor, Tab-Reihenfolge, Klick liefert `Reset(\"record_fps\")`, Ausblenden halb und am Ende 0). Gates laufen im Zustand „alle Keys gesetzt“, das Überlauf-Gate prüft jetzt auch die Reset-Knöpfe (922 Teile, keiner auf Name oder Control). Render bei 946 und 360 angesehen. Befund am Rand (älter, nicht S8): mit der eingebetteten Ersatzschrift kürzt ein gestapeltes Segment bei 360 px seine Teile („System and…“, „…“); mit Outfit passt es; das Umbruch-Gate prüft Segment-Teile nicht, offen als eigener Punkt. Checks: fmt, clippy ohne Warnung, `cargo test` 108 + 374 grün. |
| 5c S9 | umgesetzt 26.10. nach „S9 im Detail“ (Abweichung dort: Hinweis nennt im Engpass, was passt, wegen der unteren Bitrate-Klemme). Length als Dropdown 15 s bis 10 min (eigener Wert aus der Datei bleibt Eintrag), `Prefs::replay_bitrate`, Hinweis unter Memory aus `video_kbps` und `AUDIO_KBPS`. Belegt: Standard 60 s/512 MiB ergibt 20 994 kbit/s, gleich dem `-q 20994` des laufenden Recorders; mit den Werten des Nutzers (30 s, 512 MiB) „Video at about 41 Mbit/s“. Gates laufen jetzt auch mit 600 s/256 MiB („Only about 5 min fit, give it more memory“); Copy-Gate-Allowlist um die Einheit „Mbit/s“ ergänzt (134 Strings). Test `replay_length_steps_and_what_the_memory_buys`. Render angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 109 + 374 grün. |
| 5c S10a | umgesetzt 26.10. nach „S10 im Detail“: Zeilen Display (Focused display, jeder Monitor aus dem `OutputState` des Fensters mit `tray::output_label`, jetzt geteilt, abgesteckter als „(disconnected)“, Both displays; schreibt `focused`/`both`/`output:<name>`) und Codec (Auto, H.264, „H.264 on the CPU“ = `libx264`, HEVC, AV1, „… via Vulkan“; HDR/10-Bit fallen weg; `gsr --info` einmal im Hintergrund, `save::probe_codecs`); `choose_text` für Text-Werte; Monitor-Ereignisse bauen neu; Icons `columns-2`, `film` ins Sprite (33). Abweichung: die Zeile „Both displays“ (Separate/Combined) heißt jetzt „Both displays as“, weil das Display-Dropdown selbst eine Option „Both displays“ hat. Copy-Allowlist um „CPU“ und „Vulkan“ (158 Strings). Test `display_and_codec_offer_what_this_machine_has`. Gates: Überlauf 1002 Teile, Umbruch 337 Zeilen, Dropdowns 30. Render angesehen. Live nicht geprüft (echte Monitor- und Codec-Liste nur im laufenden Fenster). Checks: fmt, clippy ohne Warnung, `cargo test` 110 + 374 grün. |
| 5c S10b | umgesetzt 26.10. nach „S10b im Detail“. Dropdown: höchstens 10 Einträge sichtbar, scrollt in sich (`Scroll`, neu `Scroll::jump`, öffnet mit dem gewählten Eintrag in der Mitte), Rad über offener Liste scrollt die Liste (`Widget::overlay_wheel`), Pfeile halten die Markierung sichtbar, Randeinträge mit Clip, Liste so breit wie ihr längster Eintrag (mindestens das Control, höchstens der Platz), geschlossen höchstens `MENU_MAX = 280`; Tippen springt (Groß/klein egal, 1 s Fenster, Leertaste gehört zum Wort, `Widget::text` bekommt die Zeit). `Bind::Choices` (Wert oder `None` = Key entfernen). Seite General (Icon `settings`) mit Font (Icon `type`, „Desktop font“ schreibt `Reset(\"ui_font\")`), Familien einmal im Hintergrund (`fc-list`, 330 Familien), das Fenster lädt seine Schrift neu, wenn `ui_font` sich ändert. Fixture mit 40er-Liste; Copy-Gate zählt Font-Namen nicht. Aufgeräumt: `app/pages.rs` (700 Zeilen) nach `app/pages/` (`mod.rs` 427, `prefs.rs` 85, `rows.rs` 210), dabei ein verirrter Doc-Kommentar über `general` an `pages` zurück. Tests: `a_long_list_shows_ten_scrolls_in_itself_and_jumps_to_what_is_typed`, `the_font_row_offers_the_desktop_font_and_the_installed_families`; Gates: Überlauf 1067 Teile (die offene 40er-Liste bleibt im Fenster), Umbruch 376 Zeilen, Copy 166 Strings. Render der offenen Liste mitten im Scrollen angesehen. Live nicht geprüft (echte Familienliste und Neuladen der Schrift nur im laufenden Fenster). Checks: fmt, clippy ohne Warnung, `cargo test` 112 + 374 grün. |
| 5c S11 | umgesetzt 26.10. nach „S11 im Detail“: Ordnerfelder mit Knopf `folder-open` rechts im Feld (eigene Hover-Fläche, Text endet `GAP_S` davor), Klick oder `Ctrl+O` → `Effect::Browse`, in der App `Effect::PickFolder(block)`; `linux/app/folder.rs` ruft `FileChooser.OpenFile` mit `directory`, `modal`, `current_folder` (Ablauf wie `capture/portal.rs`, `percent_decode` jetzt geteilt), in einem Thread, Antwort mit (Seite, Block), eine andere Seite verwirft sie; Abbruch (Code 1) ändert nichts; `App::pick` setzt den Text und meldet ihn wie ein bestätigtes Feld. Verfügbarkeit einmal im Hintergrund (`version ≥ 3`), ohne Portal kein Knopf. Test `a_folder_field_asks_for_the_dialog_and_saves_what_it_picks`. Render mit Hover angesehen. Live nicht geprüft (der Dialog wartet auf den Nutzer). Checks: fmt, clippy ohne Warnung, `cargo test` 113 + 374 grün. |
| 5c S12 | umgesetzt 26.10. nach „S12 im Detail“: vier eigene Symbole `shelf-top-left` … `shelf-bottom-right` im Sprite (39), Corner-Segment nutzt sie statt der Pfeile (die Pfeile bleiben für das Fixture). Karte nach dem ersten Render auf 8 × 5 vergrößert. Test `the_corner_pictograms_put_the_card_in_their_corner`. Render bei Skala 2 angesehen; Icon-Gate mit Outfit unverändert. Checks: fmt, clippy ohne Warnung, `cargo test` 114 + 374 grün. |
| 4j | umgesetzt 26.10. nach „Phase 4j“: `Fonts::id`, `Dropdown` merkt sich die Inhaltsbreite je Schrift; das Fenster gibt je Frame nur die letzte Zeigerbewegung weiter, andere Eingaben holen sie vorher nach. Sonde (Release, 330 Einträge, Liste offen): je Bewegung 1,67 ms → 96 µs, je Frame 2,09 → 0,58 ms. Test `a_dropdown_measures_once_per_loaded_font`. Checks: fmt, clippy ohne Warnung, `cargo test` 115 + 374 grün. |
| 4k | umgesetzt 26.10. nach „Phase 4k“: `assets/logo/boltsnap-mark.svg` (die zwei Potrace-Pfade des finalen Logos, Herkunft und Umrechnung im Kommentar), `app/brand.rs` (`Mark`: einmal je Skala in eine Pixmap gerendert, jeder Pfad für sich gefüllt, Schatten oben, Licht unten), Töne `PRESSED`, `SHADE`, `LIP`; Feder `mark_room` (nur mit Platz unter den Einträgen) und mit den Namen ausgeblendet. Beim Render gefunden: ein gemeinsamer Pfad schnitt die Überlappung von Punkt und Körper aus (behoben); die erste Tönung (0x12 auf 0x17) war kaum sichtbar, jetzt 0x0F. Pixel geprüft: Punkt gefüllt, Lücke zur Kerbe, Kanten oben 0x13/unten 0x1E. Tests `the_mark_shows_in_the_free_corner_only`, `both_paths_of_the_mark_parse`. Checks: fmt, clippy ohne Warnung, `cargo test` 117 + 374 grün. |
| 5d P1 | umgesetzt 26.10. nach „Phase 5d“: `Block::Preview(Preview::Shelf)` oben auf der Shelf-Seite, `app/preview.rs` (`shelf_cards` mit den Shelf-Formeln, Federn je Karte auf dem Bezugsbildschirm 1920 × 1080, Ecke `GLIDE`, Breite `SNAP`, Anzahl blendet `QUICK` an Ort und Stelle, Reduced Motion springt), folgt den Controls sofort. Abweichungen nach dem ersten Render: Bildschirm füllt die Fläche mit `GAP_M` rundum statt höchstens 360 px mittig; Radius `RADIUS` statt einer freien 6. Abstand-Gate prüft den Bildschirm (12 px rundum, Abweichung 0,00). Tests `the_cards_sit_where_the_shelf_puts_them`, `a_new_corner_glides_the_cards_there_and_a_new_card_fades_in_in_place`. Renders bei 946 und 360 angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 119 + 374 grün. |
| 5d P2 | umgesetzt 26.10. nach „P2 Displays“: Vorschauen nach `app/preview/` (`mod.rs` mit `Preview`, `State`, Höhe je Art; `shelf.rs`, `displays.rs`); `Prefs::displays` als `Screen` mit Lage und Größe aus `OutputInfo`; Vorschau oben auf der Recording-Seite (nach der Aufnahme-Notiz, sie ist wichtiger), Monitore maßstäblich und mittig, 2 px Fuge, gewählt hell, Focused alle halb, Both beide, Combined schließt die Fuge (Feder `QUICK`). Gate: alle Monitore in `GAP_M`, Bild mittig (das Abstand-Gate hatte die Shelf-Geometrie für jede Vorschau angesetzt, korrigiert). Tests: Reihenfolge, Gleichheit, Helligkeit je Ziel, Brücke blendet, ohne Monitore kein Block. Render dreier Zustände angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 121 + 374 grün. |
| 5d P3 | umgesetzt 26.10. nach „P3 Replay-Leiste“: `preview/replay.rs` (Zeitleiste über die Breite, rechts „now“, runde Schritte 5 s bis 10 min, höchstens sechs und nicht enger als 64 px, Füllung von rechts aus `duration_us`, im Engpass der nicht haltbare Teil dunkel, Länge und Füllung gleiten mit `GLIDE`); `replay::status_quiet` (direkt am Steuer-Socket, 100 ms, startet den Daemon nie; `preview_target` nutzt es mit); `Settings::seconds_held` als gemeinsame Formel für Hinweis und Leiste; Fenster fragt einmal je Sekunde (`save::poll_replay`); „Restart replay“ wird jetzt nach der Zeile „Replay buffer“ eingefügt statt an fester Stelle (die Vorschau oben hätte sie verschoben). Beim Render gefunden: die Ersatzschrift hat kein U+2212, jetzt fällt es auf den Bindestrich zurück (Outfit hat es). Tests: Schrittweiten breit und schmal, Füllung, Engpass (≈ 44 % dunkel bei 10 min/256 MiB), Länge gleitet; Gate prüft den Rand der Leiste. Render zweier Zustände angesehen. Live nicht geprüft (Füllung wächst nur bei laufendem Puffer im offenen Fenster). Checks: fmt, clippy ohne Warnung, `cargo test` 123 + 374 grün. |
| 5e V3 | umgesetzt 26.10. nach „Phase 5e“ V3: Kopfzeile („Holds 22 s“ bzw. „Off“ in `TEXT`, Bitrate in `SUB`), Leiste aus Zellen (eine je Sekunde bis 60, 16 px, 2 px Fuge, gehalten `TEXT` zu 85 %, die wachsende Zelle blendet anteilig, nicht haltbare `WINDOW`), „now“ in `TEXT`. Gate prüft den Rand über die Kopfzeile. Test `the_replay_head_says_what_it_holds_and_the_bar_is_cells`. Render angesehen. Checks: fmt, clippy ohne Warnung, `cargo test` 124 + 374 grün. |
| 5e V1 | umgesetzt 27.10. nach „V1 und V4 im Detail“: `preview/displays.rs` (Gehäuse `RAISE` Radius 5, 4 px Fuge, Bildschirm 3 px eingerückt Radius 2, Fuß 5 + 3 px nur ohne Monitor darunter, Anordnung höchstens 160 px; Name `Hint` und Anschluss `Key` unter jedem Monitor, übereinander gestapelte teilen die Breite; Zeile „One clip“/„Two clips“/„Three clips“/„The one with focus when you start“/„Not connected, the focused one records“ in `Label`, `TEXT`, mittig, Platz für die längste bei der Breite, Kreuzblende `QUICK`; Combined schließt nur die Fugen zwischen den Bildschirmen statt eines Rechtecks über alles, damit versetzte Monitore keine leere Fläche bekommen). Abweichung beim Render: ohne Bild war „halb an“ (`SELECTED` zu 50 % auf `WINDOW`) genau `RAISE` und das Gehäuse verschwand; an ist jetzt `SELECTED` 30 % Richtung `SUB` (#575757, halb #343434). `pages::monitor_name` (Herstellercodes, `Unknown` fällt weg, kein doppelter Hersteller), `Screen::entry` fürs Dropdown („BenQ ZOWIE XL LCD (DP-3)“), Copy-Gate nimmt Modellnamen aus und die Zeilen dazu. `ui/picture.rs` (`Picture`: vormultipliziert, Halbstufen bis 64 px, `Canvas::picture` mit `Pattern` in einen runden Pfad), `preview/shots.rs` (reine Regeln), `linux/app/shots.rs` (Lader im Thread, Kanal ins Fenster; `--render` lädt sofort), `App::set_shots` (blendet `QUICK`). Tests: `monitors_are_named_by_maker_and_model`, `the_line_under_the_monitors_says_what_a_recording_makes_and_crossfades`, `three_monitors_in_a_row_and_two_stacked_keep_their_names_apart`, `a_whole_screen_is_told_from_an_area_and_pictures_fit_and_cover`, `a_picture_keeps_halves_down_to_64_px_and_premultiplies`, Lader `the_newest_whole_screen_is_the_screen_and_the_others_are_cards` (Bildschirm, Bereiche, kaputtes PNG, MP4, fehlender Ordner); Gate: Namen und Zeile in `GAP_M`, Namen ohne Überlappung, Zeile ganze `Label`-Zeilen. Renders (echte Schrift, Wegwerf-Test im Binary, nicht eingecheckt) bei 760 und 360, Zustände Focused, AOC, Both Combined, abgesteckt, dazu mit einem Bild über beide Monitore bei Skala 2 angesehen. Gates: Padding 12 px, Abweichung 0,00; Versalien 0,00 px (`--measure`, Outfit); Überlauf 1067 Teile grün bei 360 bis 1400; Kontrast unverändert, niedrigstes Paar 4,83:1. Live nicht geprüft (echte Monitore und Namen aus `OutputInfo`). Checks: fmt, clippy ohne Warnung, `cargo test` 129 + 375 grün, `git diff --check`. |
| 5e V4 | umgesetzt 27.10. nach „V1 und V4 im Detail“: Hintergrund aus dem Bildschirm-PNG (`shots::cover`, breiter: linker Teil), zu 65 % Richtung `WINDOW`; Karte i trägt Bild i von der Ecke aus, ohne Bild `SELECTED`; Bilder blenden beim Ankommen ein (`QUICK`). `preview/dims.rs`: Maß an Karte 0, Höhenlinie `GAP_S` neben der Innenkante, Breitenlinie im Rand zur Bildschirmkante, 1 px mit 5 px Endstrichen, „190 px“/„132 px“ (`Key`, `TEXT`) in einer Spalte `GAP_XS` hinter der Höhenlinie, Versalien mittig auf Kartenmitte und Breitenlinie (gemessen 0,0 px Abweichung bei 760). Abweichung beim Render: mit der Breitenlinie mittig im Rand ragte die Unterlänge von „px“ bei 760 px 1 bis 2 px unter den Bildschirm; die Linie hält jetzt 9 px Abstand zur unteren Kante, die Schwelle ist 11,5 px Rand statt 10 (bei 760 sind es 11,7, das Maß zeigt; bei 360 und zwischen 600 und etwa 700 blendet es aus, Feder in `resize`). Ziehen: `Target::Preview(block)`, 8 px Streifen über der Innenkante, Hover/Drücken hellen die Linien auf, `grab_edge`/`drag_edge` setzen den Slider über `adopt` (rastet 10, klemmt), geschrieben über die Drossel wie der Slider, `update` hält die gezogene Zeile. Tests: `the_front_card_measures_190_by_132_beside_it_and_follows_the_slider`, `dragging_the_cards_edge_sets_card_size_in_steps_and_writes_like_the_slider` (30 px → 280, Drossel, Daemon-Wert zieht nicht zurück, Loslassen schreibt 310), `the_cards_show_the_newest_screenshots_on_the_darkened_screen` (Pixel: Kartenbild, leere Karte `SELECTED`, weißer Hintergrund 101); Gate: Maß auf dem Bildschirm bei allen Breiten, Kontrast `TEXT` auf weißem Screenshot hinter 65 % 4,93:1. Renders (echte Schrift, echte Screenshots: zwei fast schwarze, eine Bereichsaufnahme; dazu ein Hintergrund-PNG zur Probe) bei 360, 760, 1400 und oben links/unten links bei Skala 2 angesehen. Gates: Padding 0,00, Überlauf 1067 Teile, niedrigster Kontrast 4,83:1 (unverändert). Live nicht geprüft: Ziehen im echten Fenster, Laden im Thread nach dem Öffnen. Checks: fmt, clippy ohne Warnung, `cargo test` 132 + 375 grün, `git diff --check`. |
| 5e V4b | umgesetzt 27.10. nach „V4b Maße, die zum Fenster passen“: `preview/dims.rs` (`Measure` mit `width_chip`/`height_chip`, `far_end`), Zahlen in `RAISE`-Flächen (16 px, Radius `GAP_XS`, Versalien mittig), Höhe neben der vordersten Karte, Breite `GAP_S` hinter der letzten Karte; Feder `far` in `preview/mod.rs`, gesetzt in `sync_shelf` (`SNAP` neue Zahl, `GLIDE` neue Ecke). Abweichung: bei kleinen Karten (360 px Fenster, Karte 24 px breit) ist die Breitenzahl breiter als die Karte und wird an den Bildschirmrand geklemmt statt mittig zu stehen. Tests: `the_front_card_measures_190_by_132_beside_it_and_follows_the_slider` (neu: Lage an vorderster und letzter Karte, Zahl mittig an der Linie, zeigt jetzt auch bei 360), `the_width_line_moves_with_the_end_of_the_stack_and_leaves_a_full_screen`; Gate: Linien, Flächen, Kante auf dem Bildschirm bei allen Breiten. Renders bei 760 (Skala 2) und 360 angesehen; gemessen bei Skala 2: Ziffern oben/unten 8/8 Gerätepixel, links/rechts 9/9 und 9/10 (≤ 0,5 px logisch). Kontrast `TEXT` auf `RAISE`/`HOVER` im bestehenden Gate. Checks: fmt, clippy ohne Warnung, `cargo test` 133 + 375 grün, `git diff --check`. Live: nach Install offen (Fenster neu öffnen). |
| 5e V5 | umgesetzt 27.10. nach „V5 Keine eigenen Bilder des Nutzers“: Bilder ganz entfernt. `git rm` `linux/app/shots.rs` (Lader-Thread), `app/preview/shots.rs`, `ui/picture.rs`; in `linux/app/mod.rs` Modul, `save_dir`, Kanal und das Laden in `--render`; `App::set_shots`, `shots`/`shots_in`, `BACKDROP_DARK` samt Kontrastpaar; Zeichnen in `displays.rs` (Bildschirme an = `SELECTED` 30 % Richtung `SUB`, halb gemischt, aus `WINDOW`) und `shelf.rs` (Bildschirm `WINDOW`, Karten `SELECTED`), Maß aus V4b unverändert. Tests nur für Bilder entfernt (Zuschnitt, `Picture`, Kartenbilder, Lader), neu `the_shelf_preview_is_surfaces_only_and_reads_no_pictures` (Pixel: jede Karte `SELECTED`, Bildschirm `WINDOW`). `grep` nach `read_dir`/`image::` in `linux/app/`, `app/`, `ui/`: nichts; `resolve_save_dir` nur noch für den angezeigten Pfad der Zeile „Save folder“. `docs/codebase.md` und README nannten die Bilder nicht. Renders Shelf (`--render`, 760 Skala 2 und 360) und Recording (Wegwerf-Test mit zwei Monitoren, nicht eingecheckt) angesehen. Gates: Padding 0,00, Überlauf 1067 Teile, Kontrast 12 Paare niedrigstes 4,83:1, Versalien 0,00 px (`--measure`). Checks: fmt, clippy ohne Warnung, `cargo test` 131 + 392 grün, `git diff --check`. |
| 5a | umgesetzt 26.10. nach „Phase 5a: Öffnen ohne Terminal“: Tray-Eintrag „Settings…“ ganz unten (Prozessstart im Tray-Thread), `assets/linux/boltsnap.desktop` mit Icons 64/128/256 aus `boltsnap.ico`, `.deb`-Assets, Flake-`postInstall`, Release-Tarball, README. Abweichung: `Categories=Graphics;`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 75 + 371 grün, `desktop-file-validate` ohne Meldung. Live: Daemon neu gestartet, dbusmenu-`GetLayout` endet auf „Settings…“; `gio launch` der installierten `.desktop`-Datei öffnet das Fenster (Klasse `boltsnap`). Nicht live prüfbar: Klick im Tray. |
| 5 | geplant, freigegeben 24.10. |
| 6 (Library) | Skizze im Library-Plan |
| 5f | umgesetzt 27.10. nach „Phase 5f“ (zusammen mit Selector S4/S5, `docs/plans/selector/2026-10-27-selector-toolbar.md`): `ui::push_squircle`, `squircle_shape`, `SQUIRCLE_SCALE` 1,8408964, `SQUIRCLE_TOLERANCE` 0,025; `rounded_rect` (also `Canvas::fill`, `Canvas::picture`) und `Canvas::ring` nutzen sie, `push_rounded` nur noch für Lucide-`rect`. Abweichung: Polarform statt (cos^(2/n), sin^(2/n)), siehe Abschnitt. Gemessen: 45°-Punkt gleich dem Kreis ± 0,05 px bei r = 2, 4, 5, 9, 12, 16; Pille r = h/2 ergibt n = 2,0000, Punkte ± 0,03 px auf dem Kreis; r = 6/7,5/9 in 20 px Höhe geklemmt mit 2 < n < 4, 45°-Punkt ± 0,05; Sehnenfehler gegen die dichte Kurve 0,023 (r 4, 61 Punkte je Fläche), 0,024 (r 12, 93), 0,020 (r 40, 181), alle < 0,025; gerade Kanten pixelgleich zur eckigen Füllung. Gefunden und behoben (Zoom 4×, Skript für über zwei Reihen verschmierte gerade Kanten, Lauf ≥ 30 px): Segment-Teile lagen bei geteiltem Rest auf Bruchteilen (gewählte Fläche „Mic“ mit verschmierter linker Kante bei Skala 2), jetzt Grenzen gerundet, Summe gleich; Karten der Shelf-Vorschau lagen auf Bruchteilen (skaliert), jetzt Kanten gerundet. Danach keine verschmierte Flächenkante auf Shelf, Screenshots, Recording, Replay, General bei 760@2 und 360 (Treffer nur noch Lucide-Striche und Schrift). Gates unverändert grün: Padding 12 px (Abweichung 0,00), Achse 0,00; Kontrast 13 Paare, niedrigstes 4,83:1; Icons 24 Zeilen, Mitte 0,32, Achse 0,31, Kante 0,00, Abstand 0,37 (`--measure` Outfit: 26 Zeilen 0,31/0,31/0,00/0,37); Versalien 0,00 px je Control (eingebettet und Outfit); Überlauf 1067 Teile; Umbruch 376 Zeilen; Copy 170 Strings. Renders vorher/nachher in `scratchpad/s5/{before,after}/` (Shelf, Recording, General bei 760@2 und 360, dazu Screenshots und Replay nachher), Vergleich `scratchpad/s5/z-compare.png`. Checks: fmt, clippy ohne Warnung, `cargo test` 137 + 391 grün, `git diff --check`. |
| 5g V6 | umgesetzt 27.10. nach „Phase 5g V6“, Abweichungen (laut Review 27.10. vorher nicht alle markiert): (0) Modul `src/shelf_card/` (`layout.rs`, `paint.rs`) statt `src/shelf/card.rs` + `shelf_layout`, kein `Canvas::draw_pixels` (tiny-skias `draw_pixmap` mit Maske), Icons `square-mouse-pointer`, `squares-unite`, `orbit` statt `mouse-pointer-2`, `copy`, `move`; (1) die neue Karte kommt erst, wenn die alte weg ist (240 ms, dann 150 ms): gleichzeitig ließ die Shelf-Logik (`overflow_card_opacity`) die übrige Karte aus- und am Ende hart wieder einblenden, im Render bei 90 ms gesehen; (2) `--render` kann `--move X,Y`, `--click X,Y`, `--after MS`, für die Zustands-Screenshots. A: `src/shelf_card/` (Lib: `layout.rs` per `git mv` mit `animated_card_layout`, `animated_item_limit`, `overflow_card_opacity`, `hold_size`; `paint.rs` mit `Frame`, `draw_shelf(…, picture)`, Skala für Radius, Schatten, Striche; `CardKind`, `CardLook`, `APPEAR_MS`/`DISMISS_MS`, `appear`/`dismiss`), `shelf/paint.rs` 1197 → 501 Zeilen, `linux/shelf/anim.rs` (aus `mod.rs`, 3681 → 3494). B: Keys `shelf.card_opacity` (30–100), `shelf.solid_on_hover`, `shelf.card_shadow`, `shelf.animations`, `ShelfPrefs::look()`; Animations aus: kein Appear, Dismiss entfernt sofort. C: `app/preview/shelf.rs` (`Play`: zwei Platzhalter, Federn für Breite `SNAP`, Ecke `GLIDE`, zweite Karte `SNAP`; Malen über den Shelf-Painter in eine Pixmap, Maske auf den Block; Hover je Karte als Differenzbild mit `QUICK`), `dims.rs` (Höhenlinie plus „190 × 132 px“ am ruhenden Eckplatz), `Target::Card`, Überschrift „Look“ mit `blend`, `square-mouse-pointer`, `squares-unite`, `orbit`; Slider zeigt „80%“ ohne Leerzeichen. D: Ecken-Symbole G. Tests neu: `two_cards_rest_in_the_corner_at_their_real_size`, `a_new_corner_glides_the_cards_there`, `one_card_shown_folds_the_second_away`, `the_x_dismisses_a_card_and_a_new_one_arrives_on_top`, `without_animations_a_card_goes_and_comes_at_once`, `the_cards_are_the_shelfs_own_painting_with_the_look_of_the_page`, `the_look_sets_opacity_hover_and_shadow`, `the_preview_leaves_the_save_button_out`, Ecken-Symbol-Test mit Tinten-Mitte (± 1 px bei 48 px). Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 146 + 381 grün, Gates (Kontrast min. 4,83:1, Icons 0,31 px, Padding 0,00, Überlauf 1227 Teile bei 360–1400 px), `settings --measure` mit echter Schrift. Renders Ruhe, Hover, Dismiss-Folge, 2×, 360 px angesehen. Nicht geprüft: `cargo check --target x86_64-pc-windows-msvc` (Ziel nicht installiert; `draw_shelf_opaque` behält Signatur), live im Daemon (Installation und Neustart beim Nutzer). |
| 5g V6b | umgesetzt 27.10. nach „V6b Maß greifbar, Hover flüssig“: `dims::measure` nimmt `shown`, `edge` reicht von `GRIP/2` in der Karte bis `GAP_XS` hinter die Zahl (ohne Maß der Streifen an der Kante); `ui::PointerShape`, `App::pointer_shape`, das Fenster setzt `ew-resize`/Standard nur bei Wechsel (`linux/app/input.rs`, nach jedem Pointer-Frame). `preview/shelf.rs`: Painter-Bilder je Hover-Zustand behalten (`PaintKey` aus Größe, Kartenrechtecken, Animationen, Look, Skala), Hover mischt nur das Kartenrechteck (`blend_rect`), Maske nur wenn die Fläche über den Block ragt und dann behalten. Tests `the_whole_measure_drags_and_shows_the_sideways_pointer`, `hovering_a_card_blends_kept_pictures_instead_of_painting_again` (ein Painter-Lauf für den Hover-Zustand, danach keiner). Checks: fmt, clippy ohne Warnung, `cargo test` 148 + 381 grün, Render Hover. Live beim Nutzer offen. |
| 5g V6c | Fix 27.10. (Meldung mit Aufnahme `boltsnap-final-3573342-1.mp4`: „die animation wenn man die obere card im preview wegclickt ist tot broken“): das Platzhalterbild kam aus der Größe der ersten Karte im Layout; geht die obere, ist das die schrumpfende, und jede Karte bekam ihr gestauchtes Bild (die untere wurde von oben her leer, dann sprang sie zurück). Jetzt aus der Kartengröße (`Placed::card`). Test `the_card_left_behind_keeps_its_picture_while_the_top_one_goes`, schlägt mit dem alten Code an (21 statt 52 bei 30 ms). Checks: fmt, clippy, `cargo test` grün, Render-Folge 30–260 ms. |
| 5g V7 | umgesetzt 27.10. nach „V7 Zwei Monitore als ein Clip“: `displays.rs` `joins` statt `seams` (Fuge plus `BODY_RADIUS` in beide Gehäuse in `RAISE`, über die gemeinsame Höhe bzw. Breite), Bildschirme danach obendrauf, also mit Rahmen und Ecken. Test `one_clip_joins_the_monitors_bodies_and_keeps_their_screens_whole` (Fuge Panel bei zwei Clips, `RAISE` bei einem, Innenrahmen bleibt, keine Kerbe oben und unten); `display_shots` (ignoriert) für die Bilder. Checks: fmt, clippy, `cargo test` grün, Render beider Zustände in 2× angesehen. |
| 5h | umgesetzt 27.10. nach „Phase 5h“, Entscheidung (a) nur öffnen. Abweichungen: (1) Segment „Alt held / Always“ statt „With Alt“ (Copy-Gate: Title Case); (2) die Lupe ist nicht im Scene-Test, `Selector::magnifier_on` braucht Wayland-Objekte, getestet ist das Lesen der Config; (3) beim Bauen gefunden und behoben: ein Reset oder Entfernen eines Replay-Keys (auch der Reset-Knopf bei Frame rate) markierte die laufende Session nicht als veraltet, „Restart replay“ kam nicht (`linux/app/save.rs::send`); (4) `open_media_card` mit `media_open_command` aus `linux/shelf/mod.rs` nach `cards.rs` (`mod.rs` 3494 → 3479); (5) Display, Bitrate, Encoder unter eigener Überschrift „Video“ statt „Buffer“; (6) ein Wert, den die Liste nicht anbietet, steht als „NAME (not installed)“ da; (7) kein `ShelfPrefs::open_with`, sondern `Config::shelf_open_with`, beim Klick neu gelesen (ShelfPrefs ist `Copy`). Neu: Keys `copy_to_clipboard`, `selector_magnifier`, `shelf.open_with`; `EDITORS` (eddy, Satty, swappy, GIMP, Krita, Pinta), fehlender Editor → `xdg-open` und einmal eine Meldung; Replay-Zeilen Display („Like recordings“ entfernt den Key), Bitrate (Auto, 10–80 Mbit/s), Encoder (ohne CPU) unter „Video“; Icons `clipboard-copy`, `zoom-in`, `app-window`, `activity`, `cpu`; `rows::choices`. Tests `a_wayland_shelf_capture_copies_by_the_config_unless_a_flag_says`, `copy_magnifier_and_open_with_read_with_their_defaults`, `an_image_card_opens_with_the_chosen_editor_and_a_video_with_the_default_app`, `like_recordings_and_auto_remove_their_replay_key_and_a_monitor_sets_it`, Zählung der schreibenden Zeilen 26. Checks: fmt, clippy ohne Warnung, `cargo test` 151 + 389 grün; Gates: Kontrast min. 4,83:1, Copy 242 Texte, Icons 0,32 px, Padding 0,00, Überlauf 1467 Teile bei 360–1400 px; `settings --measure` 36 Zeilen 0,31 px. Renders Screenshots (760, 360, offenes Dropdown) und Replay angesehen. Live offen (Daemon-Neustart für „Open with“). |
| 5i | umgesetzt 27.10. nach „Phase 5i“, Abweichung: der Block bekommt die Überschrift „Load“ (wie jede Gruppe) und bricht in schmalen Spalten die Zahlen in eine zweite Zeile (Höhe aus der breitesten Form, springt nicht). `linux/app/load.rs` (`Meter`, Prozesse aus `/proc` über die Session-PID, CPU aus `utime + stime`, RSS aus `statm`), `poll_replay` schickt `(held, load)`, `preview/load.rs` (`Preview::Load { stale }`, `ReplayLoad`, „Off, nothing in use“ / „In use now“ / „Before your changes“), `save.rs::refusal` („Boltsnap is older than this window. Restart it to save this“). Tests: `/proc/stat` mit Klammern im Namen, Prozessbaum (gsr, kms-Helfer, Worker, kein fremdes Kind), Texte, Refusal; ignoriert `replay_load_now` gegen den laufenden Buffer: 36 % eines Kerns, 424 MB (`ps`: RSS 210 + 203 + 1,7 MB). Checks: fmt, clippy, `cargo test` 152 + 392 grün, Gates (Kontrast 4,83:1, Copy 242, Padding 0,00, Überlauf 1467). Render nur „Off“ (der Poll läuft nur im Fenster); live offen. |
| 5j | umgesetzt 27.10. nach „Phase 5j“, Abweichung: die Vorschau blendet mit der Feder `QUICK` (≈ 150 ms) statt linear 120 ms, weil ihre Hover über die App-Fades laufen; aus ist sie wie der Overlay sofort (`App::hover_at_once`). Key `shelf.hover_fade` (Standard aus), Zeile „Fade on hover“ (Icon `clock-fading`, Hinweis „Over 120 ms instead of at once“); `shelf_card::HOVER_MS = 120`; Painter `Frame::hovered: &[(id, Stärke)]` (Deckkraft gemischt, Knöpfe mit Alpha × Stärke); Overlay `AnimKind::HoverIn/HoverOut`, `Daemon::set_hovered`, `hover_lifts`, `hover_lift`/`hover_begun` (linear, Umkehr von der erreichten Stärke); Windows-Wrapper behält `Option<u64>`. Tests: `half_a_hover_fade_is_half_way_to_solid_with_half_its_buttons`, `a_hover_fade_goes_both_ways_and_turns_around_where_it_stands`, `a_preview_card_hovers_at_once_unless_the_hover_fade_is_on`, Config-Standard, Zählung 27. Checks: fmt, clippy ohne Warnung, `cargo test` 157 + 396 grün, Gates (Copy 248, Icons 0,32 px, Padding 0,00, Überlauf 1507). Live offen. |
| 5k | umgesetzt 27.10. nach „Phase 5k“, Abweichung nach Messung: Dithering um 1 Stufe reicht nicht, die Bänder entstehen im Ergebnis des Compositors (über 18 ändert 1/255 Alpha das Ergebnis um 0,07 Stufen); jetzt `SHADOW_SPREAD` ±3,5 Alpha-Stufen (über 18: ±0,25 Ausgabestufe, bricht jede Bandkante; über Weiß ±3,5 Stufen Korn, etwa Hyprlands Blur-Rauschen), nur im Abfall, nahe 0 schmaler (Mittel bleibt). ±7 verglichen: füllt auch die Bänder, aber deutlich körnig auf hellem Grund. `shelf_card/paint.rs` `smoothstep`, `dithered`; Test `the_shadow_is_dithered_its_average_kept_and_free_of_bands` (Gegenprobe ohne Rauschen: 21 Streifen-Spalten), ignoriert `shadow_shot`. Grenze: über sehr dunklem Grund bleiben 4–5 Helligkeitsstufen, nur ihre Kanten sind weg. Checks: fmt, clippy, `cargo test` grün; Render über 18 und 200 (× 4 gestreckt) vorher/nachher angesehen. Live offen (Daemon-Neustart). |
