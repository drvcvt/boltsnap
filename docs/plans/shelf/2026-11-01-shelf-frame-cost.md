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

Jeder Shelf-Frame mit 6 Karten deutlich unter 4,2 ms (240 Hz), angestrebt
1–2 ms, also Animationen in voller Bildrate; **Pixel identisch** zu heute.

## Plan (Bau-Stand nach dem Lesen von `shelf_card/paint.rs`)

Ganze Sprites je Karte verworfen: auf die Karte kommen danach Hover-
Buttons, Video-Badge und Spinner, und bei einer Appear-Animation rutschen
alle Karten, ein Sprite müsste ständig neu. Stattdessen nur das Teure und
Unveränderliche cachen, die Mal-Logik bleibt:

1. **Schattenmaske je Kartengröße** (Schlüssel: Breite, Höhe, `scale`,
   Nachkommaanteil der Schattenposition): je Pixel der Umgebung der
   Schattenwert `ALPHA · smoothstep(ramp)` und ob er gedithert wird; beim
   Malen nur noch `dithered` an der Bildschirmposition und `blend_px` wie
   heute. Pixel, die die Karte selbst danach überschreibt (`cov > 0` im
   eigenen Rechteck, Schatten gibt es nur für ruhende Karten), fehlen in
   der Maske: dort ist heute schon nichts vom Schatten zu sehen.
2. **Eckenmaske je Kartengröße** (Breite, Höhe, Radius): `rr_coverage` je
   Pixel einmal; beim Malen für `cov == 1` eine 256er-Tabelle
   `round(v · a)` je Deckkraft `a` (dieselbe Rechnung, vorher erledigt),
   für den Rand die heutige Formel mit dem Maskenwert.
3. Caches thread-lokal (Overlay-Thread und Settings-Vorschau malen
   getrennt), höchstens 32 Einträge, dann geleert.
4. **Tests**: die heutigen Funktionen als Referenz im Test-Modul;
   Pixelgleichheit (Toleranz 0) für Ruhe, Hover-Mitte, Appear-Mitte,
   Dismiss-Mitte, Deckkraft 0,8 und 1, Vorschau-`scale` 1,25; Kosten-Test
   (ignoriert, lokal) mit Median und p95; Screenshot der echten Darstellung
   (UI-Gate 7, `shadow_shot`).

Was brechen kann: die Settings-Vorschau teilt `draw_shelf` (der Test mit
`scale` 1,25 deckt sie), Speicher je Maske ~150 KB.

Nicht: kein GPU-Zeichnen, keine Änderung an Timing, Kurven oder Aussehen.

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
