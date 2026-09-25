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
| V2 | umgesetzt, live offen |
