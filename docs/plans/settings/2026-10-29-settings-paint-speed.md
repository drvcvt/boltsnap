# Settings-Fenster: schneller malen

Stand 2026-10-29. Herkunft: [Tempo-Durchsicht im Replay-Speicherplan](../replay/2026-10-28-replay-memory.md#tempo-durchsicht-2910).
Nutzer 29.10.: „1-3 klingt nach guten optimierungen wenn die nichts haesslich
oder pixelig machen. sonst kannst du gerne mit dem planen anfangen“.

Ziel: weniger CPU beim Bedienen des Settings-Fensters, bei **gleichen Pixeln**.
Jede Stufe hat als Tor einen Pixelvergleich gegen den Stand davor, nicht
„sieht gleich aus“.

## Befund

Profil vom 29.10. (`perf -e cpu-clock` auf dem unstripped Build in
`target/prof`, Fenster beim Bedienen, 240 Hz): bis zu einem ganzen Kern.
Formen (tiny-skia `fill_path`, Blitter, Squircle) ~50 %, ttf-parser ~20 %,
libc ~17 %, libm ~10 %. Dazu aus dem Code:

| # | Priorität | Befund | Beleg |
|---|---|---|---|
| F1 | hoch | Jede Mausbewegung malt ein ganzes Bild, auch wenn sich nichts ändert. Erst danach vergleicht `changed` mit dem letzten Bild und schickt nichts. Bei bewegter Maus also bis zu 240 volle Bilder pro Sekunde ohne sichtbare Änderung. Neu, stand nicht in der Durchsicht. | `linux/app/window.rs` `input` (Move ruft `changed`, also `dirty = true`), `draw` |
| F2 | mittel | Die Glyphen-Raster sind schon gecacht (`text.rs` `glyphs`). Die ttf-parser-Zeit kommt von **Metriken**: jedes `h_advance` und `kern` rechnet `height_unscaled` neu (`ascender`/`descender` mit MVAR-Deltas, `parse_delta`), dazu `glyph_hor_advance` mit HVAR und die cmap-Suche. `cap_height` liest bei jedem `baseline` den Umriss von `H`. Layout und Ellipsen messen jedes Bild alle Texte. | `ui/text.rs` `advances`, `width`, `cap_height`. ab_glyph 0.2.32 `scale.rs:88,162,187`: `scale.x / height_unscaled() * unscaled` |
| F3 | mittel | Jede abgerundete Fläche rechnet ihre vier Ecken jedes Bild neu: je Teilungsschritt `sin_cos`, dreimal `powf`, `hypot`. | `ui/mod.rs` `push_squircle`, `flatten` |
| F4 | mittel | Jedes Bild malt das ganze Fenster: `clear` (`Pixmap::fill` 7 %), alle Flächen, Texte und Icons, auch wenn nur ein Hover in einer Zeile läuft. | `app/draw.rs` `App::draw` |

Nicht reproduziert ist hier nichts Neues; F1 bis F4 folgen direkt aus dem Code
und dem Profil.

**Abweichung von der Durchsicht:** Hebel 2 heißt dort „Glyphen einmal
rastern“. Das gibt es schon, gecacht werden müssen die Metriken (F2). Hebel 1
(nur Geändertes malen) bekommt mit F1 eine billige Vorstufe.

## Verhalten

Der Nutzer sieht nichts anderes: gleiche Pixel, gleiche Federn, gleiche
Zeiten. Das Fenster braucht nur weniger CPU.

- Maus bewegt sich in einer Zeile, nichts wechselt: kein neues Bild (F1).
- Maus wechselt die Zeile: Hover blendet wie heute; gemalt wird nur, was die
  Blende berührt (F4, Stufe D).
- Scrollen, Seitenwechsel, Navigation auf/zu, offene Liste, Ziehen: volles
  Bild wie heute, nur billiger (Stufen B, C).

## Stufen

Reihenfolge 0, B, C, A, D. Jede Stufe ein eigener Commit, jede mit Messung.

### Stufe 0: Messwerkzeug

`boltsnap settings --render PAGE OUT.png … --frames N`: malt nach dem
Einschwingen (und nach `--move`/`--click`) N Bilder im Abstand von 1/240 s
und gibt Median und Maximum der Malzeit je Bild aus. Das letzte Bild landet in
`OUT.png`. Ab Stufe D malt es wie das Fenster (Teilbild, wo erlaubt) und
vergleicht jedes Bild mit einem vollen Bild: `identical N/N`.

- Datei: `src/platform/linux/app/mod.rs` (`render_options`, `render`).
- Kein Desktop, keine großen Dateien; die PNGs liegen im Scratchpad.

Gemessen wird vor B und nach jeder Stufe: Seiten Shelf und Recording, ruhig
(`--frames 240`) und mit Hover-Blende (`--move` auf eine Zeile, `--after 0`).

### Stufe B: Metriken merken (F2)

`Fonts` merkt sich je Schnitt einmal `height_unscaled` und die Versalhöhe von
`H` (unskaliert), je Zeichen die Glyphe und deren unskalierte Breite, je Paar
das unskalierte Kerning. Skaliert wird mit genau der Rechnung von ab_glyph:
`(scale.x / height_unscaled) * unscaled`. Das ergibt bitgleiche Floats, also
gleiche Lagen und Pixel.

- Datei: `src/ui/text.rs` (`advances`, `width`, `carets`, `draw_clipped`,
  `cap_height`). `RefCell<HashMap>` wie der Raster-Cache daneben.
- Randfälle: `\u{a0}` und `−` bleiben auf Leerzeichen und Bindestrich
  gemappt (der Cache speichert das Ergebnis von `glyph`). Die Größen bleiben
  frei, gespeichert wird unskaliert, der Cache wächst nur mit den Zeichen.
- Tor: PNGs aller fünf Seiten (Skala 1 und 2) und der `--fixture`-Galerie
  byte-gleich zum Stand davor (`cmp`); `cargo test`.

### Stufe C: Squircle-Ecken merken (F3)

Je Eckform (Halbachse `R` und Exponent `n` aus `squircle_shape`, als Bits)
und Laufrichtung merkt sich `ui` die abgeflachten Eckpunkte relativ zur
Eckmitte. `push_squircle` setzt nur noch `Mitte + u·a + v·b` mit genau der
Formel von heute zusammen. Der Cache wird geleert, wenn er über 256 Formen
hat (animierte Höhen erzeugen viele).

- Datei: `src/ui/mod.rs`. Nur das Settings-Fenster und seine Vorschauen
  nutzen `push_squircle`; der Selector hat eigene Ecken (`selector/render/mod.rs`
  `rounded_rect`).
- Abweichung, die bleibt: die Teilung entscheidet heute in Fensterkoordinaten,
  danach in Eckkoordinaten. Das kann in Grenzfällen einen Teilungsschritt mehr
  oder weniger geben; die Toleranz bleibt 0,025 logische Pixel. Tor: PNG-
  Vergleich wie bei B; jede Abweichung wird mit Anzahl Pixel und größter
  Kanaldifferenz berichtet, bevor committet wird.

### Stufe A: Mausbewegung ohne Wirkung malt nicht (F1)

Eine Mausbewegung malt nur, wenn sie etwas ändern kann: das Hover-Ziel
wechselt, etwas wird gezogen (`capture`), eine Liste ist offen oder danach
läuft eine Feder (`App::animating`). Sonst bleibt das Bild stehen, nur der
Zeiger-Look wird wie heute gesetzt.

- `src/app/mod.rs`: `App::handle` bleibt; neu ein kleiner Vergleich vor und
  nach der Bewegung (Hover-Ziel, `capture`, offene Liste, `animating`).
- `src/platform/linux/app/window.rs`: Move setzt nicht mehr `dirty`, sondern
  nur `pending_move`; `redraw` läuft bei `dirty || pending_move`. `draw`
  wendet die Bewegung an und malt nur, wenn sie zählt. Bewegungen bleiben
  pro Bild gesammelt: wird nicht gemalt, geht trotzdem ein leerer Commit mit
  Frame-Callback raus, wie heute bei laufender Animation ohne neue Pixel.
- Randfälle: Karten der Shelf-Vorschau wechseln den Hover ohne Blende
  (`hover_at_once`), das ist ein Zielwechsel, zählt also. Teil-Hover in
  Segment, Stepper und Ordner-Knopf laufen über Federn. Ziehen am Regler
  und an der Vorschau-Kante hat `capture`.
- Tor: Unit-Tests in `src/app/tests/`: Bewegung in derselben Zeile ohne
  Feder zählt nicht; Zeilenwechsel, Ziehen, Kartenwechsel und offene Liste
  zählen.

### Stufe D: nur Geändertes malen (F4)

Ein Bild darf ein Teilbild sein, wenn seit dem letzten gemalten Bild nur
Hover-, Druck- und Fokus-Blenden von Zielen mit fester Lage und Federn von
Controls liefen und keine Eingabe außer Mausbewegungen kam. Dann ist der
Schaden die Vereinigung der Ziel-Rechtecke (Zeile des Controls oder des
Reset-Knopfs, Navigationseintrag, Vorschau-Block), die jetzt oder beim letzten
Bild liefen oder deren Hover wechselte, um 2 Gerätepixel vergrößert und auf
ganze Gerätepixel gerundet. Alles andere (Scrollen, Seiten- und
Navigationsfedern, `keyed`, Räume auf/zu, Vorschau in Bewegung, offene Liste,
Ziehen, Tasten, Klicks, Daten vom Daemon, neue Größe) malt voll.

**Abweichung 29.10. (gemessen, vor dem Bau):** Die Maske von tiny-skia malt
nicht byte-gleich. Probe (Wegwerf-Test in `ui/tests.rs`, nicht committet):
deckende Kantenpixel von `fill_path` und `stroke_path` weichen mit Maske um 1
von 255 ab (154 und 476 Bytes je Form), mit Alpha unter 1 und bei
`draw_pixmap` und `fill_rect` nichts. Ein Teilbild in eigener Pixmap, um ganze
Gerätepixel verschoben, ist dagegen exakt: 0 abweichende Pixel bei Skala 1
und 2, auch bei Schnitten mitten durch Formen. D malt deshalb so:

- `src/ui/mod.rs`: `Canvas` bekommt einen Ursprung in Gerätepixeln.
  `transform` verschiebt um ihn, `fade` und eine neue `bounds` rechnen mit
  ihm. `Canvas::part(rect)` gibt die Leinwand nur für ein Rechteck (um 2
  Gerätepixel vergrößert, auf ganze Pixel gerundet), `Canvas::paste` setzt
  sie zurück ins ganze Bild.
- Ursprung beachten: `ui/text.rs` `draw`/`draw_clipped`, `ui/icons.rs`
  `transform`, `app/brand.rs`, `app/draw.rs` `draw_part`,
  `app/preview/shelf.rs` `draw_clipped` (dort die Blockmaske nur skaliert,
  nicht verschoben). Gefunden per `grep` nach `canvas.scale`, `draw_pixmap`,
  `.transform()`, `canvas.width()`.
- `src/app/mod.rs`: `animating` teilt sich in das, was überall wirken kann
  (Navigation, Seite, Daumen, `keyed`, Marke, Vorschau, Räume, Scrollen, offene
  Liste, Ziehen, Blöcke im Kommen oder Gehen) und das Örtliche (Blenden,
  Controls). Was künftig dazukommt, landet damit von selbst im vollen Bild.
- `src/app/draw.rs`: `App::damage(now, full) -> Option<Rect>` mit dem, was
  beim letzten Bild lief (Ziele, Hover, ob Globales lief).
- `src/platform/linux/app/window.rs`: jede Auslösung außer Bewegung und
  laufender Animation setzt `full`. Für ein Teilbild kopiert das Fenster das
  letzte Bild in die Leinwand und setzt das Teilbild hinein; `changed` findet
  danach wie bisher den Damage.
- Tor (fail-closed): Test in `src/app/tests/paint.rs` fährt Hover über
  mehrere Zeilen, Teile eines Segments und die Navigation, Wechsel mitten in
  der Blende, auf zwei Seiten und der Galerie bei Skala 2, Bild für Bild als
  Teilbild und als volles Bild, und verlangt byte-gleiche Pixel und echte
  Teilbilder. Gegenprobe: ohne ein Ziel (die Navigation) schlägt er fehl.
- Nachtrag beim Bau: ein Teilbild je Ziel statt einer Box um alle (siehe
  Stand).
  Mit der echten Schrift dasselbe über `--frames` (`identical N/N`).

## Macht nicht

- Kein GPU-Malen, kein Wechsel weg von tiny-skia.
- Scrollen verschiebt keine Pixel, es malt weiter voll (nur billiger durch B
  und C).
- Keine Teil-Kopie in den Wayland-Puffer; der Upload bleibt ganz.
- Selector, Shelf und Aufnahme-Leiste bleiben unberührt.
- Federn, Zeiten, Rundungen in der Textmischung (`roundf`) bleiben.

## Ergebnis

Volles Bild (Scrollen, Seitenwechsel): −17 % (Shelf) und −26 % (Recording)
durch B und C. Mausbewegung ohne Wirkung: kein Bild mehr (A). Hover-Blende:
etwa ein Zehntel (Zeile) bis ein Drittel (Navigation) eines vollen Bilds (D).
Pixel überall byte-gleich zum Stand vor dem Plan.

## Prüfungen

Je Stufe `cargo fmt --check`, `cargo test` (nice, `jobs = 2`), PNG-Vergleich,
`--frames`-Messung. Live mit dem Nutzer offen: perf auf das echte Fenster
beim Bedienen, vorher/nachher (synthetische Mausbewegungen kommen in
Hyprland nicht als Motion an).

## Stand

| Stufe | Status | Messung |
|---|---|---|
| 0 | gebaut 29.10. | Ausgangswert (Median je Bild, 760 × 520, Skala 1, je 3 × 480 Bilder): Shelf 1,07–1,21 ms, Recording 1,34–1,38 ms, mit Hover-Blende gleich |
| B | gebaut 29.10. | 23 Referenzbilder (fünf Seiten × Skala 1 und 2, Hover auf Zeile und Navigation, Galerie × 3) byte-gleich; Shelf 0,96–1,02 ms (−10 %), Recording 1,16–1,27 ms (−14 %); `cargo test` grün |
| C | gebaut 29.10. | 23 Referenzbilder byte-gleich (der Grenzfall in der Teilung kam nicht vor); Shelf 0,89–0,93 ms (−6 % zu B), Recording 1,01–1,02 ms (−13 % zu B); zusammen mit B −17 % und −26 % zum Ausgangswert; `cargo test` grün |
| A | gebaut 29.10. | `tests/paint.rs`: Bewegung auf demselben Control ohne Feder malt nicht, Zeilenwechsel, Teil-Hover, Ziehen und offene Liste malen; Gegenprobe ohne die `animating`-Bedingung lässt den Teil-Hover-Test fehlschlagen. Live offen (Fenster mit bewegter Maus, perf) |
| D | gebaut 29.10. | Erste Fassung mit einer Box um alle Ziele fiel im eigenen Test durch: deren Kante schnitt das Replay-Icon der Navigation, und eine gestrichene Kurve malt tiny-skia an der Pixmap-Kante leicht anders. Seitdem ein Teilbild je Ziel (Zeile, Navigationseintrag), Kanten nur an Element-Grenzen. `tests/paint.rs`: vier Tests, Bild für Bild Teilbild gegen volles Bild byte-gleich (Recording mit Wechseln mitten in der Blende, Shelf und General mit jedem Control und Eintrag nacheinander, Galerie bei Skala 2, Galerie mit Klick auf jedes Control: 66 von 153 Bildern Teilbilder, Schalter, Segmente, Fokus). Gegenprobe ohne Navigationsziele: die drei Hover-Tests schlagen fehl. Ohne die Ziele des letzten Bilds schlägt nichts fehl: eine Feder ist erst fertig, wenn ihr Wert schon pixelgleich am Ziel ist (`motion.rs` `done`); sie bleiben als Sicherheitsnetz. Echte Schrift (`--frames 240`, fünf Seiten × drei Mauspunkte): 240/240 identisch. 23 Referenzbilder byte-gleich. Während der Blende (24 Bilder): Zeilen-Hover Shelf 0,08–0,09 ms, Recording 0,11–0,12 ms (vorher ~1,0–1,3 ms), Navigations-Hover 0,26–0,47 ms. `cargo test` grün (Lib-Suite ~12 s statt ~2 s durch diese Tests im Debug-Build), clippy sauber |

## Live-Abnahme und Befunde, 30.10.

**Messung:** je 25 s ununterbrochen Maus über Zeilen und Navigation der
Recording-Seite, Fenster nach 25 s per `timeout` zu, `/usr/bin/time`:
Stand vor dem Plan 53 % CPU (13,0 s user, 0,5 s sys), jetzt 29 % (7,0 s
user, 0,4 s sys).

**L1, mittel, behoben: Stufensprung am Ende des Seiten-Einblendens.** Vom
Nutzer gesehen: die Einstellungen werden am Ende der Animation minimal
heller. Nicht aus diesem Plan, schon vorher so. Ursache: `Canvas::fade`
(`src/ui/mod.rs`) mischte mit `>> 8` abgeschnitten; im Ausklang der Feder
steht `keep` bei 255/256, eine Gruppenfläche ist dann 27 statt 28, bis die
Blende endet und alles außer dem Fensterhintergrund eine Stufe springt.
Nachgewiesen mit einer Bild-für-Bild-Aufnahme des Seitenwechsels (echte
Schrift, 240 Bilder): 195 969 Pixel springen bei 408 ms, davor 80 ms auf
der dunkleren Stufe. Erster Fix (`9a6d6b6`): gerundet. Nachschau des
Nutzers: der Sprung kommt jetzt früher. Grund: eine Gruppenfläche hat von
18 nach 28 nur zehn Stufen; neun kommen im Abstand von 12–21 ms, die
letzte erst 66 ms später im Ausklang der Feder (bei 229 ms) und wird als
Sprung gesehen. Zeitliches Banding. Zweiter Fix: `fade` mischt in 1/4096
statt 1/256 und dithert mit einer festen 4 × 4-Bayer-Matrix am absoluten
Fensterpixel (Teilbilder bleiben gleich dem vollen Bild). Aufnahme danach:
das Mittel der Zeile 300 steigt Bild für Bild stetig, der letzte Schritt
bei 441 ms ist +0,04 Stufen (vorher +1 auf der ganzen Fläche). Ruhebilder
unverändert, da `fade` bei `keep = 1` nichts tut. Test
`a_fading_area_shows_its_exact_mix_on_average` in `src/ui/tests.rs`
(schlägt mit bloßem Runden fehl: bei keep 243/256 Mittel 27 statt 27,49).
Live-Nachschau durch den Nutzer offen.

**L2, niedrig, offen: ein Pixel Abweichung im echten Fenster.** Eine
Prüf-Version verglich 40 s lang jedes gezeigte Bild mit einem frisch
gemalten: einmal 1 Pixel bei (302, 491), Skala 1, um 7/255 daneben. Nicht
nachgestellt; die Tests mit der Ersatzschrift und `--frames` mit der echten
Schrift waren identisch. Vermutlich eine Kante, die eine Teilbild-Grenze
schneidet (wie das Replay-Icon in Stufe D). Nächster Schritt: Ort und
Element bestimmen, dann die Teilbild-Grenze daran ausrichten.

**L3, niedrig, offen: `damage` merkt das Bild vor dem Zeigen.** Findet
`Buffers::next` keinen Puffer, kehrt `draw` (`window.rs`) zurück, nachdem
`App::damage` das Bild schon als gemalt vermerkt hat; das nächste Teilbild
setzt dann auf einem älteren Stand auf. In 40 s Prüflauf nie eingetreten.
Fix: in diesem Rückweg `full` setzen.
