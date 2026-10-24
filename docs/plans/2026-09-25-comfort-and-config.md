# Komfort-Vorschläge und Konfigurationsfehler, 2026-09-25

Lesender Durchgang über CLI, Konfiguration, Shelf, Recording-Popup, Tray,
Selector und Replay auf `7d59866`. Nichts davon ist umgesetzt. Neue Features
brauchen laut AGENTS.md erst die Zustimmung des Maintainers; die Fehler unten
sind Code-Belege, **nicht live reproduziert**.

## 1. Fehler

### K1 (P1): Daemon schreibt veraltete Recording-Einstellungen zurück

**Auslöser/Wirkung:** Der Daemon liest die sieben Recording-Einstellungen einmal
beim Start (`src/platform/linux/shelf/mod.rs:1186`, `config.recording_prefs()`).
Jeder Tray-Toggle schreibt über `persist_tray_prefs` (`mod.rs:1493`) alle
sieben aus dieser Kopie zurück (`src/config.rs:312-354`). Wer `record_cursor`
bei laufendem Daemon von Hand ändert, bekommt beim nächsten REC den alten Wert
(Cursor kommt aus dem Cache, Codec und FPS werden frisch gelesen), und der
nächste Toggle überschreibt die Datei wieder. `record_flow` speichert die
Audio-/Frame-Schalter vor der Abbruchprüfung (`src/main.rs:350-357`): Toggle
plus Esc ändert die Datei, nicht den Daemon, und der nächste Tray-Klick dreht
es zurück. `save_dir` wird ebenfalls nur beim Start gelesen (`mod.rs:1185`).

**Fix-Idee:** Einstellungen beim Recording-Start frisch lesen und beim Schreiben
nur den geänderten Schlüssel setzen. Größe S-M. Betroffen: `shelf/mod.rs`
(`recording_prefs`, `persist_tray_prefs`, Start-Pfad), `config.rs`.

**Umgesetzt:** statt nur geänderte Schlüssel zu schreiben (der Schreib-Thread
fasst Schreibvorgänge zusammen, einzelne Schlüssel gingen dabei verloren)
liest der Daemon die Datei neu, bevor er eine Einstellung ändert oder eine
Aufnahme startet: `refresh_recording_prefs` vor jeder Tray-Aktion, im
`StartRecording`-Request, in `start_recording`, `start_default_recording` und
`start_recording_outputs`. Das Schreiben danach gibt die frischen Werte
zurück. Übersprungen wird der Refresh, solange ein eigener Schreibvorgang offen
ist (`settled_prefs_generation` hinter `prefs_generation`); eine fehlende oder
kaputte Datei (`Config::load_valid`) ersetzt nie die bekannten Werte.
`save_dir` wird beim Speichern einer Bildkarte frisch aufgelöst, `--save-dir`
hat weiter Vorrang. Rest-Race: eine Handänderung in den Millisekunden zwischen
Refresh und Schreiben geht verloren. Kein automatischer Test für den
Eventloop-Pfad; live offen: Datei bei laufendem Daemon ändern, Tray-Toggle,
Datei prüfen.

### K2 (P2): Speichern der Einstellungen sortiert und löscht Kommentare

`save_recording_prefs` geht über `toml::Table` und `to_string_pretty`
(`src/config.rs:306-356`). Kommentare und Reihenfolge der Datei gehen verloren.
`toml_edit` ist über `toml` 0.8 schon im Baum und erhält beides. Passt zu K1.

**Umgesetzt:** `save_recording_prefs_at` liest die Datei als
`toml_edit::Document` (0.20.2, war schon im Lock) und setzt nur die sieben
Werte. Bestehende Schlüssel behalten Platz und Inline-Kommentar, neue landen
auf oberster Ebene vor `[replay]`. Eine kaputte Datei wird weiter nicht
überschrieben. Test `recording_prefs_write_keeps_comments_order_and_tables`.

### K3 (P3): `--save` ignoriert `save_dir`

`paths::default_save_path` (`src/platform/linux/paths.rs:130-137`) schreibt fest
nach `~/Pictures/Screenshots/boltsnap-<unix-ms>.png`, der Shelf-Save nach
`save_dir` mit lokaler Zeit und ohne Überschreiben. Fix: `resolve_save_dir` und
`unique_saved_path_at` wiederverwenden. Größe S.

### K4 (P3): Popup-Timer bleibt bei 99:59 stehen

`fmt_elapsed` kappt auf `99 * 60 + 59` (`src/shelf/recording.rs:96-99`).
Aufnahmen über 100 Minuten zeigen 99:59. Einzeiler (Stunden anzeigen).
`src/shelf/` ist mit der eingefrorenen Windows-Shell geteilt; die Signatur
bleibt gleich.

### K5 (P2): Fehlgeschlagener Aufnahmestart per Hotkey ist unsichtbar (30.09.)

Auslöser: Root-Platte mit 288 MB frei. `require_recording_reserve`
(`src/record/finalize.rs`) lehnt den Start unter 2 GiB ab, der Daemon antwortet
mit einem Fehler, und `boltsnap record` schreibt ihn nur auf stderr. Hotkey
(Hyprland `exec`) und Tray (`TrayAction::StartRegion`, stderr auf `/dev/null`)
haben kein Terminal. Für den Nutzer: Auswahl ziehen, nichts passiert, das
nächste Alt+Print sieht `idle` und öffnet wieder die Auswahl statt der
Controls. Reproduziert mit `boltsnap record full` → `recording stopped to
preserve 2 GiB of free disk space (291008512 bytes available)`, Status danach
`idle`.

Fix: der Linux-`record_flow` (`src/main.rs`) meldet jeden Fehler zusätzlich
über `platform::shelf::notify` als „Recording: …“, wie `replay` es über
`notify_error` schon tut. Abbruch der Auswahl bleibt `Ok` und still. Windows
unverändert (eingefroren). Alternative Vorabprüfung des Platzes vor der
Auswahl verworfen: doppelte Logik, und andere Startfehler blieben unsichtbar.

## 2. Vorschläge (nach Nutzen/Aufwand)

| # | Vorschlag | Größe | Dateien | Live-Test |
| --- | --- | --- | --- | --- |
| V1 | Tray reagiert auf Aufnahmen: Stop/Controls-Eintrag, Icon-Status, „Clip fullscreen“ grau ohne Replay | S | `tray.rs`, `shelf/mod.rs` | ja |
| V2 | Tastenkürzel im Recording-Popup (Space, Enter, D) und Discard mit Bestätigung | S | `shelf/mod.rs`, `src/shelf/recording.rs`, `paint.rs` | kurz |
| V3 | Shelf-Rückmeldung: Kopieren blinkt, Mittelklick schließt, gespeicherte Karte zeigt Haken; später Scrollen | S (Scroll M) | `shelf/mod.rs`, `src/shelf/paint.rs`, `layout.rs` | ja |
| V4 | `boltsnap doctor` prüft gsr/Encoder, ffmpeg, pactl, Plugin, Worker und Config-Tippfehler | S | `paths.rs`, `config.rs` | nein |
| V5 | Letzten Aufnahmebereich im Selector vorauswählen | S-M | `select_skia/mod.rs`, `main.rs` | ja |
| V6 | `--delay N` für Screenshots | S | `main.rs` | kurz |
| V7 | Fenster-Snapping im Selector über `hyprctl clients -j` | M-L | `hypr.rs`, `select_skia/`, `src/selector/` | ja |
| V8 | `boltsnap replay save [SECONDS]` (nur die letzten N Sekunden) | M | `replay/mod.rs`, Worker `live.rs`, `src/replay/ring.rs` | ja |

**V1 umgesetzt:** `tray.rs`: `recording_entries` setzt während einer Aufnahme
Pause bzw. Resume, „Save recording to shelf“ und „Save recording to disk“ an
den Menüanfang (`TrayAction::Recording`, im Daemon über
`handle_recording_action`, Fehler als Benachrichtigung); beim Speichern ein
grauer Eintrag „Saving recording…“. Discard bewusst nicht im Tray. Der
Snap-Punkt des Icons wird rot (Aufnahme, Speichern) bzw. gelb (Pause), Alpha
und Körper bleiben unverändert; ksni meldet das Icon neu, sobald der Snapshot
wechselt. „Clip fullscreen to shelf“ ist nur bei laufendem Replay aktiv. Tests
für Einträge, Clip-Sperre und Einfärbung; ob die Tray-Leiste (Waybar) das neue
Icon sofort zeigt, ist live offen.

**V2 umgesetzt:** Klick und Taste laufen über `press_popup_button`
(`shelf/mod.rs`). Tasten nur bei offenem Popup: Space, Enter/KP_Enter, d/D,
Delete/KP_Delete (`popup_key_button`), Esc schließt wie bisher. Discard
schaltet beim ersten Druck auf CONFIRM und verwirft erst bei einem zweiten
innerhalb von 3 s (`DISCARD_CONFIRM`); jeder andere Button und Esc entschärfen,
`tick_recording` zeichnet nach Ablauf neu. SCTK 0.19 liefert über
`get_keyboard(.., None)` keine Tastenwiederholung, Gedrückthalten bestätigt
also nicht. `paint::PopupHints` zeichnet Tastennamen unter die Labels; die
Windows-Shell ruft weiter `draw_recording_popup` ohne Hinweise, Signatur
unverändert. Tests: Tastenzuordnung, Bestätigungsfenster, Pixel der Hinweise
und des CONFIRM-Zustands. Live offen.

### V9: Recording-Controls im neuen Stil, nie in der Aufnahme (26.09.)

**Wunsch des Nutzers:** der Popup-Stil gefällt nicht, und das Popup soll aus
Aufnahmen herausbleiben.

**Befund:** gsr nimmt Monitore per KMS auf (`gsr-src/src/capture/kms.c`), also
aus dem fertig komponierten Scanout-Buffer. Hyprland 0.56 hat die Layer-Regel
`noScreenShare` (`LayerRuleApplicator.hpp`), die greift aber nur bei
Screencopy/Portal und malt dort ein schwarzes Rechteck. Ausblenden per
Compositor geht damit nicht.

**Entscheidung (Nutzer):** Auto-Pause. Öffnen der Controls während einer
Aufnahme pausiert sie; das Popup erscheint erst, wenn die Segmente gestoppt
sind (gsr-Stop über die Pfadzeile, live etwa 150 ms). Esc schließt und nimmt
weiter auf, wenn die Controls selbst pausiert haben; Resume (Space) ebenso.
Jede Fortsetzung, egal über welchen Weg (Tray, IPC), entfernt das Popup vor
dem Start des neuen Segments. Kosten: eine verlustfreie Segmentgrenze pro
Öffnen. Verworfen: Platzierung auf einem nicht aufgenommenen Monitor (hilft
nicht, wenn beide Monitore aufgenommen werden, was der Nutzer meist tut).

**Stil (Entwurf vom Nutzer abgenommen):** mt-ui-style, dunkel, opak `#121212`,
Radius 14, Rand 16 rundum. Kopfzeile: Statuspunkt (grau bei Pause, rot bei
Aufnahme), Titel 15 px Gewicht 600 in Satzschreibung („Paused“, „Save failed“,
„Saving…“), Timer 15 px in `sub`. Darunter eine Reihe 30 px hoher Buttons
(Radius 9, Abstand 8, Innenrand 12), Label 13 px, Taste 11 px gedimmt rechts;
„Save to shelf“ als einziger invertierter Hauptbutton, sonst `#222222`.
Button-Breite aus dem breitesten möglichen Label (Pause/Resume,
Discard/Confirm), damit nichts springt; Popup-Größe aus dem Layout statt der
festen 408×148.

**Umsetzung:**
1. `src/shelf/paint.rs`: `controls_layout(font)` (Größe und Button-Rechtecke)
   und `draw_recording_controls(...)` für Linux; `draw_recording_popup` und
   `popup_hit` bleiben für die eingefrorene Windows-Shell unverändert. Die
   V2-`PopupHints` fallen wieder weg.
2. `src/platform/linux/shelf/mod.rs`: zwei Schriftgewichte (500/600) über
   `font::load_ui_font_weights`; Layout beim Erzeugen des Popups; Hit-Test über
   das Layout. `show_recording_controls` pausiert bei laufender Aufnahme
   (`auto_paused`, `controls_pending`) und erzeugt das Popup im
   `ChildrenStopped(Pause)`-Zweig. Esc fortsetzen nur bei `auto_paused`;
   `handle_recording_action(Resume)` entfernt das Popup vorher.
3. Tests: Layout (gleicher Rand, gleiche Höhen, keine Überlappung, Hit-Test),
   Zeichnen aller Zustände, Entscheidungsfunktionen für Öffnen und Esc.
4. README und dieser Plan.

**Risiko:** Das Entfernen des Popups und der Start des neuen gsr-Segments
laufen im selben Eventloop-Durchgang; das Zerstören geht erst beim Flush nach
dem Dispatch raus. gsr braucht bis zum ersten Frame deutlich länger (Prozess,
KMS, NVENC), live zu prüfen. Nicht umfasst: das Tray-Menü selbst ist Teil der
Waybar und bleibt in Aufnahmen sichtbar.

**Umgesetzt:** `paint::controls` (`controls_layout`, `draw_recording_controls`,
431×92 mit Outfit), Daemon mit `popup_fonts` (500/600 über
`load_ui_font_weights`), `popup_layout` für Größe und Hit-Test.
`show_recording_controls` folgt `controls_open_plan`: Recording → Pause und
`controls_pending`, Pausing → warten, sonst sofort; der
`ChildrenStopped(Pause)`-Zweig zeigt die wartenden Controls. Esc setzt nur
fort, wenn `esc_resumes` (von den Controls pausiert, nicht nach fehlgeschlagenem
Save). `handle_recording_action(Resume)` entfernt das Popup vor dem Spawn und
zeigt es wieder, wenn der Start scheitert. Der CONFIRM-Zustand bekommt einen
eigenen Eventloop-Timeout (`discard_confirm_remaining`,
`expire_discard_confirm`), weil im pausierten Zustand kein Aufnahme-Tick läuft;
`remove_popup` entschärft. Weggefallen: `PopupHints` aus V2 (der Windows-Painter
ist wieder wie vor V2), `font::load_ui_font`, `load_popup_font`,
`load_fontconfig_font` (unbenutzt). Tests: Layout (Rand 16 auf drei Seiten,
Abstand 8, Höhe 30, Hit-Test), Zeichnen (einziger Hauptbutton, Confirm,
Speichern ohne Hauptbutton, opakes Panel), `controls_open_plan`,
`esc_resumes`. Ein Render aller fünf Zustände mit Outfit wurde angesehen.
Live offen: Popup nicht im Video (ffprobe/Sichtprüfung der Segmentgrenze),
Verzögerung beim Öffnen, Esc setzt fort.

Risiken: V2/V3 berühren mit Windows geteilte Shelf-Dateien (alte Signaturen
behalten wie bei E1). V7 muss auf anderen Compositors auf die Bereichsauswahl
zurückfallen. V8: ein alter Worker ignoriert das neue Feld und würde still den
ganzen Clip speichern; das muss ein Fehler werden.

## 3. Stand

Nutzer-Entscheidung 26.09.: K1 und K2 zusammen, danach V1 und V2.

| ID | Stand |
| --- | --- |
| K1 | umgesetzt, live offen |
| K2 | umgesetzt |
| V1 | umgesetzt, live offen |
| V2 | umgesetzt, live offen; Tastenhinweise in V9 neu gezeichnet |
| V9 | umgesetzt, live offen |
| K5 | umgesetzt; `cargo fmt --check` und `cargo build --release` grün, `cargo test` nicht gelaufen (Platte voll, kein Anvil-Worker online); Notification live offen |

## 4. Config-Review (2026-10-23)

Read-only-Review der Config- und Settings-Schicht als Grundlage für das
[Settings-UI](settings/2026-10-23-settings-ui.md). Nichts reproduziert, alles
aus dem Code; jeder Befund am 23.10. selbst gegengelesen. Die Fixes sind
Phase 1 und 2 des Settings-Plans, dort steht die Umsetzung.

| ID | Prio | Befund | Beleg | Sicherheit |
| --- | --- | --- | --- | --- |
| CF1 | P2 | Ein Syntaxfehler in `config.toml` setzt still alles außer den 7 Recording-Prefs auf Default: Aufnahme mit 240 fps und `auto`-Codec statt `record_fps = 120` (das gsr-Hang-Setup), `record_dir`, `shelf_to_disk_after`, `ui_font` zurück. Der Selector bricht nach der Bereichswahl ab, weil `save_recording_prefs` an der kaputten Datei scheitert. Replay-Autostart fällt still aus, der Watchdog macht `Err(_) => continue` ohne Meldung. | `config.rs:232-235` (`Config::default()` bei Parse-Fehler), `shelf/mod.rs:1529` (nur Prefs über `load_valid`), `main.rs:354`, `370`, `replay/mod.rs:279-282`, `799` | verifiziert |
| CF2 | P2 | Der Writer ersetzt eine verlinkte `config.toml` (stow, home-manager) durch eine normale Datei und setzt den Modus auf 0666&umask zurück; kein `fsync` des Verzeichnisses. Heute nicht betroffen (reguläre 0644-Datei). | `config.rs:362-367` (`File::create(tmp)`, `rename(tmp, path)`) | verifiziert |
| CF3 | P2 | Falsche Typen und unbekannte Enum-Strings werden still ignoriert und beim nächsten Tray-Schreiben mit dem Default überschrieben; jeder Write schreibt alle 7 Keys, auch nie gesetzte. Unbekannte oder falsch platzierte Keys (unter `[replay]` angehängt) meldet niemand. | `config.rs:193-230`, `270-300`, `325-358` | verifiziert |
| CF4 | P2 | Mehrere Schreiber ohne Lock, jeder schreibt seinen ganzen Snapshot: die CLI liest vor dem Selector (`main.rs:354`) und schreibt danach (`370`) alle 7 Prefs, über die ganze Auswahl hinweg. Temp-Name nur pro Prozess eindeutig. | `config.rs:362`, `main.rs:354-370`, `shelf/mod.rs:1526` | plausibel (heute überlappen die Flows nicht, mit Editor oder UI schon) |
| CF5 | P3 | Tray kann „Focused“ als Default-Ziel nicht wählen; ein abgesteckter `output:X` zeigt fälschlich den fokussierten Monitor als gewählt. | `tray.rs:166-173`, `368-374` | verifiziert |
| CF6 | P3 | Kein Datei-Watcher: Handedits wirken erst bei der nächsten Aktion; `disk_add_to_shelf` wird beim Finalize aus dem Cache gelesen. | `shelf/mod.rs:3287` | verifiziert |
| CF7 | P3 | Leeres oder relatives `XDG_CONFIG_HOME`/`XDG_CACHE_HOME` ergibt einen vom Arbeitsverzeichnis abhängigen Pfad; die Spezifikation verlangt, sie zu ignorieren. | `paths.rs:92-113` | verifiziert |
| CF8 | P3 | `BOLTSNAP_SAVE_DIR`/`BOLTSNAP_RECORD_CODEC` und `$VAR`-Expansion gelten in der Daemon-Umgebung, nicht in der Shell; unbekannte Variablen werden leer (`$XDG_VIDEOS_DIR/boltsnap` → `/boltsnap`); `--save-dir` wirkt nur bei `daemon`. README:299-304 und :537 versprechen mehr. | `config.rs:410`, `426`, `447`, `main.rs:216-221`, `265` | verifiziert |

K3 (`--save` ignoriert `save_dir`) bleibt offen wie oben.

**Bestandsaufnahme** (Stand 23.10.): 14 Top-Level-Keys (`save_dir`,
`record_codec`, `record_profile`, `record_fps`, `record_dir`,
`record_default_target`, `record_both_mode`, `record_cursor`,
`record_show_frame`, `record_disk_add_to_shelf`, `record_audio_enabled`,
`record_audio_source`, `record_shelf_to_disk_after_seconds`, `ui_font`) und 7
unter `[replay]` (`duration_seconds` 1-600, `memory_mib` 64-4096, `fps` 1-240,
`encoder`, `output`, `autostart`, `bitrate_kbps` 500-200000). Wann jeder Key
wirkt und wie er im UI erscheint, steht in der Tabelle des Settings-Plans,
Abschnitt 2. Fest eingebaut und Kandidaten für Settings: `MAX_VISIBLE_CARDS`
5 (`shelf/mod.rs:330`), `CARD_W/H` 190×132 (`shelf/thumbnail.rs:6-7`),
Anker unten links mit Rand 24 (`shelf/mod.rs:1695-1700`), `CARD_OPACITY` 0,8
(`shelf/paint.rs:11`), Notifications immer über `notify-send` ohne Schalter
(`shelf/mod.rs:529-539`).
