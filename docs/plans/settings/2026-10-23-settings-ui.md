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
| 5a | umgesetzt 26.10. nach „Phase 5a: Öffnen ohne Terminal“: Tray-Eintrag „Settings…“ ganz unten (Prozessstart im Tray-Thread), `assets/linux/boltsnap.desktop` mit Icons 64/128/256 aus `boltsnap.ico`, `.deb`-Assets, Flake-`postInstall`, Release-Tarball, README. Abweichung: `Categories=Graphics;`. Checks: `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 75 + 371 grün, `desktop-file-validate` ohne Meldung. Live: Daemon neu gestartet, dbusmenu-`GetLayout` endet auf „Settings…“; `gio launch` der installierten `.desktop`-Datei öffnet das Fenster (Klasse `boltsnap`). Nicht live prüfbar: Klick im Tray. |
| 5 | geplant, freigegeben 24.10. |
| 6 (Library) | Skizze im Library-Plan |
