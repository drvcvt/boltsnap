# Shelf: Animationen in voller Bildrate (2026-11-01)

Status: **Befund und Plan, nichts gebaut.** Nutzer: „hab einfach das gefuehl
es geht noch fluessiger“, Übergänge wirken „stop motion like“.

## Befund (gemessen)

Bildschirme: 200 Hz (5,0 ms je Frame) und 240 Hz (4,2 ms).

| Was | Kosten je Frame | Messung |
| --- | ---: | --- |
| Shelf, 6 Karten 190 × 132, Schatten an (234 × 886) | **8,1–8,8 ms** (p95 bis 10,5) | Wegwerf-Test um `shelf::paint::draw_shelf`, Release, 300 Frames, Ruhe und Appear+Hover |
| Selector, ziehen/hovern (regional) | 0,1–0,3 ms je 1080p-Monitor | `examples/selector_bench.rs` |
| Selector, große Änderung (regional) | 1,1 ms | ebenda |
| Settings-Fenster | Median 0,08–0,11 ms | `boltsnap settings --render … --frames 48 --move` |

Der Shelf verpasst damit jeden zweiten Takt oder mehr: Karten erscheinen,
gehen und hovern mit halber Bildrate oder weniger, das ist das Ruckeln.

Ursache (`src/shelf_card/paint.rs`): jeder Frame berechnet jede Karte neu,
auch die ruhenden:
- `draw_card_shadow` (`:314-358`): je Karte (w + 2·16) × (h + 2·16) Pixel,
  je Pixel Wurzel, Smoothstep, Dither und Float-Blend.
- `blit_thumb_card_anim` (`:39-93`): je Pixel `rr_coverage` mit Wurzel und
  Float-Mathe, auch für ein ruhendes, deckendes Karteninneres.

## Ziel

Jeder Shelf-Frame unter 1 ms bei 6 Karten, also Animationen in voller
Bildrate auf 200/240 Hz; Pixel unverändert (Schatten, Dither, Ecken, Hover,
Deckkraft), geprüft gegen die heutigen Bilder.

## Plan

1. **Karten-Sprite je Karte**: Schatten und abgerundete Karte einmal in
   ein vormultipliziertes Sprite rechnen (Schlüssel: Karte, Größe,
   Deckkraft-Stufe, Hover-Stand), danach je Frame nur zusammensetzen.
   Der Schatten hängt nur von der Kartengröße ab, er wird je Größe einmal
   gerechnet und geteilt.
2. **Zusammensetzen mit Ganzzahlen**: vormultipliziertes Source-over als
   `u16`-Mathe je Zeile statt `f32` je Pixel; deckendes Inneres als
   Zeilenkopie.
3. **Animierte Karte** (Appear/Dismiss skaliert 0,88 → 1): das Sprite der
   Karte in der Zielgröße skalieren statt Schatten und Ecken neu zu rechnen.
4. **Test**: Pixelvergleich alter gegen neuer Weg für Ruhe, Hover-Mitte,
   Appear-Mitte, Dismiss-Mitte, Deckkraft < 1 (Toleranz 0 außer beim
   Skalieren, dort ±1); Kosten-Test (ignoriert, lokal) mit dem Ziel < 1 ms.
   Screenshot der echten Darstellung im Dark Theme (UI-Gate 7).

Was brechen kann: Speicher (6 Sprites à ~37 000 Pixel ≈ 1 MB), Cache-
Invalidierung bei Look-Änderung aus den Settings (`[shelf]` opacity,
shadow, solid_on_hover, Kartengröße), die Settings-Vorschau teilt die
Malroutinen (`app/preview/shelf.rs`) und muss gleich bleiben.

Nicht: kein GPU-Zeichnen, keine Änderung an Timing oder Kurven.

## Daneben: Selector erscheint nach ~51 ms

Gemessen (10 Läufe `boltsnap area`, `BOLTSNAP_TIMINGS`): Aufnahme ~17 ms;
die Layer-Surfaces werden erst nach der Aufnahme angelegt
(`select_skia/mod.rs:221-305`); im Configure-Handler dann
`base_pixmap_from_image` (16-MB-Kopie in neuen Pixmap) und
`SceneCache::new` (zweiter Klon, Abdunkeln), einthreadig, ~27 ms; erster
Frame bei ~51 ms, der zweite Monitor 3,5 ms später. Ansatz (eigener Plan
vor dem Bau): Surfaces parallel zur Aufnahme anlegen, Basis und
abgedunkelte Basis direkt aus libway-Kacheln über mehrere Kerne bauen.
Schätzung ~20–25 ms bis sichtbar.
