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

Risiken: V2/V3 berühren mit Windows geteilte Shelf-Dateien (alte Signaturen
behalten wie bei E1). V7 muss auf anderen Compositors auf die Bereichsauswahl
zurückfallen. V8: ein alter Worker ignoriert das neue Feld und würde still den
ganzen Clip speichern; das muss ein Fehler werden.

## 3. Stand

Nutzer-Entscheidung 26.09.: K1 und K2 zusammen, danach V1 und V2.

| ID | Stand |
| --- | --- |
| K2 | umgesetzt |
