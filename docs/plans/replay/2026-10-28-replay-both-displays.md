# Replay auf beiden Monitoren (2026-10-28)

**Wunsch (Nutzer, 28.10.):** „aktuell geht der replaybuffer nur auf einem
monitor, ich moechte aber ohne das es jetzt 1gb statt 500mb im speicher fuer
replay werden den 2ten dazubekommen und 40 mbit/s behalten am besten“.

Stand: **gebaut 28.10., live offen.** Umgesetzt wie geplant:
`replay/monitor.rs` (`replay_capture`, `Capture`, `Placed`, `chosen`,
`layout` mit derselben Formel wie `gsr::combined_source`), `replay/cursor.rs`
(der Cursor-Teil aus `mod.rs` hierher, für einen oder alle Monitore wie die
Aufnahme), `replay/mod.rs` (Status mit `layout`; Vollbild-Save bei beiden =
Ausschnitt des fokussierten Monitors, `request["output"]` für die Karte;
`PreviewTarget::monitor`, `FrozenSelection::prepare(target, on)` schneidet das
Bild auf den Monitor des Selectors und merkt sich den Versatz, `save` addiert
ihn), Selector öffnet bei beiden auf dem fokussierten Monitor, Seite Replay
bietet „Both displays“ ab zwei Monitoren. Abweichung: kein Worker-Live-Test
mit zusammengesetzter Quelle, der Worker ist unverändert (er sieht nur ein
größeres Video; Crop-Versatz und Ränder testen die bestehenden Crop-Tests).
Tests: `both_puts_the_monitors_on_one_canvas_where_gsr_does` (Layout =
gsr-Quelle, verschiedene Skalierungen und ein Monitor fallen mit Hinweis
zurück), `both_displays_open_the_selector_where_the_pointer_is` (Status mit
`layout`, `monitor()`, `chosen`, Crop-Wert, kaputtes Layout = Failed),
Settings-Schreibtest wählt „Both displays“ (`both`) und DP-3. `cargo test`
161 + 413 grün, clippy `-D warnings` still. `replay/mod.rs` jetzt 962 Zeilen:
nächster Split-Kandidat (Tests und Kontrollthread in eigene Dateien).
Live: Display auf „Both displays“ (startet den Puffer neu), Last-Anzeige
ansehen, Bereichs-Clip auf beiden Monitoren, Vollbild-Clip auf dem mit der
Maus.

## Ansatz

Ein Stream, beide Monitore nebeneinander auf einer Leinwand: gsr setzt die
Quellen selbst zusammen (`-w "DP-3;x=0;y=0|DP-1;x=1920;y=0"`, gsr 6.1.6,
Manpage „Combine sources with |“ und „x/y“ je Quelle). Die Aufnahme nutzt
das schon live für „Both displays“ (`gsr::combined_source`,
`record/session.rs` mit `gsr::Target::Combined`). Ein Encoder, eine
konstante Bitrate (`-bm cbr`, heute 40 Mbit/s), ein Ring im Worker mit
demselben Byte-Budget: **Speicher und Dauer bleiben gleich**, die Bitrate
verteilt sich auf doppelt so viele Pixel (je Monitor rund 20 Mbit/s).

Verworfen: zwei Recorder mit je einem Ring (doppelte Prozesse, zwei
Budgets, zwei Exporte, Synchronisation der Clips).

## Verhalten

- Neuer Wert `replay.output = "both"` (Seite Replay, Display: „Both
  displays“). Einzelne Monitore wie bisher.
- Bereichs-Clip: der Selector öffnet auf dem Monitor unter dem Zeiger (wie
  ohne Replay), zeigt dessen Teil des eingefrorenen Bildes; der Ausschnitt
  wird in Leinwand-Pixel versetzt (Lage des Monitors auf der Leinwand) und
  wie heute exportiert (GPU-Crop mit NVENC).
- Vollbild-Clip (Tray, `boltsnap replay save`): der Monitor unter dem Zeiger,
  auf der GPU zugeschnitten wie ein Bereichs-Clip (Nutzer 28.10.: „Monitor
  unter der Maus“).
- Karte im Shelf: auf dem Monitor, aus dem der Ausschnitt stammt.
- Weicher Cursor (`record_cursor`): ein Tracker je Monitor in einen Feed,
  die Leinwand wie bei der Aufnahme (`logical_frame`).
- Ein Monitor geht weg oder kommt dazu, während der Puffer läuft: wie heute
  bei einem einzelnen (die Überwachung startet neu bzw. meldet), mit den
  dann vorhandenen Monitoren.

## Umsetzung

1. `replay/monitor.rs`: `replay_monitor` liefert eine Auswahl `Single(Monitor)`
   oder `Both(Vec<Monitor>)`; `"both"` mit weniger als zwei Monitoren = der
   eine.
2. `replay/mod.rs` `start_capture`: bei `Both` `-w` mit
   `gsr::combined_source`, Cursor-Tracker je Monitor; die Session merkt sich
   die Lage jedes Monitors auf der Leinwand (Name, x, y, w, h in
   Video-Pixeln) und gibt sie im Status mit (`layout`), `output` bleibt
   `"both"`.
3. Selector (`replay::preview_target`, `FrozenSelection`): bei `layout`
   den Monitor des Selectors (fokussiert) nachschlagen; das Vorschaubild auf
   seinen Teil schneiden; `save` versetzt den Ausschnitt um dessen Lage.
   Clip-Bereitschaft prüft den Monitor gegen das Layout statt gegen
   `output`.
4. Daemon: `ReplayClipPending/Ready` mit dem Monitor des Ausschnitts für die
   Karte; Vollbild auf dem Monitor unter dem Zeiger.
5. Einstellungen: Display-Auswahl „Both displays“ (Wert `both`), Schema
   akzeptiert ihn, Neustart-Hinweis wie bei den anderen Replay-Werten.
6. Doku: `docs/replay.md`, README.

## Risiken und Proben vorher

- **Vorschaubild:** geprüft, kein Risiko: der Worker dekodiert für das
  eingefrorene Bild nur den Schwanz ab dem letzten Keyframe
  (`worker/src/live.rs:187` `tail_snapshot`, dann `crop::preview`), bei
  `-keyint 1` höchstens eine Sekunde Frames; doppelte Fläche verdoppelt nur
  diese.
- **Last:** Erfassen zweier Monitore und Kodieren der doppelten Fläche. Die
  Last-Anzeige der Replay-Seite misst es; Probe vorher mit der
  synthetischen Quelle, keine Desktop-Probe ohne Okay.
- **Export:** der Bereichs-Crop arbeitet auf der Leinwand; GPU-Crop
  (`nvdec_edges`) mit geraden Kanten bleibt bitgenau, auch mit Versatz.

## Tests

`replay_monitor` mit `both` (zwei, einer, keiner), Leinwand-Layout aus
`combined_source`-Positionen, Versatz des Ausschnitts je Monitor (links,
rechts, verschiedene Skalierungen), Vorschau-Schnitt, Status mit `layout`
zu `PreviewTarget`; Worker-Live-Test mit einer zusammengesetzten Quelle
(zwei `testsrc2` nebeneinander) inklusive Bereichs-Clip aus der rechten
Hälfte.

## Macht nicht

Keine zwei Puffer, keine höhere Bitrate oder mehr Speicher, keine Auswahl
über beide Monitore hinweg in einem Bereichs-Clip.
