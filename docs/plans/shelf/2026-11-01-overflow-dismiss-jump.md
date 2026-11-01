# Shelf: Sprung beim Entfernen, wenn das Shelf überläuft (2026-11-01)

Status: **umgesetzt (01.11.), automatisiert grün, live offen.** Meldung des Nutzers: „wenn man zu viele
images hat und dann welche removed jumped das so komisch rum“.

## Befund

Priorität P2 (sichtbar bei jedem Entfernen mit Überlauf, kein Datenverlust).

- Auslöser: mehr Karten als `visible_cards` (beim Nutzer 6, Ecke
  `bottom-right`), Karten verschieden hoch (Hoch- und Querformat, Video).
- Ursache: `animated_card_layout` (`src/shelf_card/layout.rs:170-231`) gibt
  im Überlauf (`items.len() > visible`) als Höhe die **vollen** Höhen der
  ersten `visible` Einträge zurück, auch die der Karte, die gerade
  zusammenklappt (`:211-219`). Die nachrückende Karte zählt nicht. Unten
  verankert sitzt der Stapel auf der Unterkante (`hold_size` `:234-245`), also
  gleiten die Karten nur um die Höhe der entfernten Karte und springen am Ende
  (`tick_animations` → `remove_card_now` → `relayout`,
  `shelf/cards/anim.rs:243-263`, `shelf/placement.rs:213-229`) um den
  Unterschied zur nachrückenden Karte.
- Zweiter Fehler, auch ohne Überlauf: die Lücke über Karte `i` klappt nur mit
  ihr selbst und, für `i == 1`, mit Karte 0 zu (`:191-193`). Klappen die
  obersten zwei zusammen, bleibt über der dritten eine Lücke stehen, die am
  Ende wegspringt.
- Reproduziert ohne Desktop über eine Frame-Simulation der echten Funktionen
  (8-ms-Schritte, `DISMISS_MS` 240, Höhen 100/120/90/150/100/110/200/80):
  oberste Karte weg → im letzten Frame alle Karten −100 px; zwei schnell
  nacheinander → −111 und +40 px; 7 Karten, oberste zwei → +29/+18 px. Gleich
  hohe Karten und oben verankerte Shelfs springen nicht (deshalb selten
  bemerkt).

## Ziel

Jede Karte bewegt sich während des Entfernens stetig und steht im letzten
Animations-Frame genau dort, wo das fertige Layout sie hinstellt. Gilt für
eine und für mehrere überlappende Entfernungen, mit und ohne Überlauf, oben
und unten verankert.

Nicht: kein anderes Timing, keine andere Kurve.

## Umsetzung

1. `animated_card_layout`:
   - Lücke über Karte `i`: `gap · (1 − c_i) · (1 − Π_{j<i} c_j)`; über der
     ersten sichtbaren Karte also keine, wie im fertigen Layout.
   - Höhe im Überlauf: die Unterkante dort, wo `visible` Karten-Anteile voll
     sind (Anteil einer Karte `1 − c`); die nachrückende Karte trägt ihren
     Rest anteilig samt Lücke bei. Ohne Überlauf wie bisher das Ende des
     Inhalts. Bei Ende der Animation ergibt das genau `Layout::compute`.
2. `Daemon::animated_layout` (`shelf/cards/anim.rs:268-301`): die Fläche
   bleibt auf `max(jetzt, fertiges Layout)` stehen, damit eine wachsende Höhe
   nicht jeden Frame die Fläche ändert (der Grund für `hold_size`).
3. Tests in `layout.rs`: Übergang am Ende (Karte mit `c = 1` gegen fertiges
   Layout ohne sie) gleich, bezogen auf die verankerte Kante, für
   Überlauf unten, zwei überlappende Entfernungen, die obersten zwei ohne
   Überlauf. Scheitern heute.

Risiken: Breite folgt weiter `take(visible)`; verschieden breite Karten
werden nur über `hold_size` gehalten (unverändert). Die Hit-Tests nutzen
dasselbe Layout, bleiben also deckungsgleich mit dem Bild.

Prüfung: `cargo fmt --check`, `cargo test`; Frame-Simulation ohne Sprung.
Live: nach Install mit dem Nutzer 8+ gemischte Screenshots, oberste und
mittlere entfernen.

## Umsetzung (01.11.)

- Wie geplant, plus eine Abweichung: der Appear-Pfad sprang gespiegelt.
  Gemessen: volles Shelf unten, neue Karte (100) und rausgehende (200) →
  Höhe im ersten Frame 864 → 764, alle Karten +100 px. Gleiche Ursache, also
  mitbehoben: die ankommende Karte belegt ihren Anteil mit dem Fortschritt,
  die Unterkante folgt dem Scroll. Deshalb hält `animated_layout` die Fläche
  jetzt auch beim Ankommen (einmal wachsen, am Ende einmal schrumpfen statt
  jeden Frame `set_size`).
- Ohne Überlauf und ohne Animation ist die Höhe wie vorher das Ende des
  Inhalts; die Settings-Vorschau (`app/preview/shelf.rs`, gleich große Karten)
  rechnet unverändert.
- Tests in `shelf_card/layout.rs`:
  `a_card_leaving_a_full_shelf_ends_where_the_shelf_settles`,
  `a_card_arriving_at_a_full_shelf_starts_where_the_shelf_stood`,
  `overlapping_dismissals_hand_over_without_a_jump`,
  `the_top_two_leaving_take_their_gaps_along`. Die drei Entfernen-Tests
  scheitern gegen die alte Funktion (gegengeprüft), der Ankommen-Test laut
  Messung oben ebenso. Frame-Simulation danach: kein Sprung am Übergang,
  größter Schritt je 8-ms-Frame 31 px bei zwei überlappenden Entfernungen
  (Weg 210 px in 240 ms, `ease_out_cubic`).
- `cargo fmt --check`, `cargo test` 178 + 426 grün.
- Live offen: mit dem Nutzer nach Install.
