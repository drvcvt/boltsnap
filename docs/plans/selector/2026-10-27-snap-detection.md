# Selector: Rechtecke erkennen und einrasten (2026-10-27)

**Wunsch (Nutzer, 27.10.):** Im Selector beim Clippen nur den YouTube-Rahmen
treffen, ohne ihn von Hand zu ziehen: „das fenster detecten … oder den
abschnitt so nur das video ohne das aussen rum“. Kanten nachschärfen „nur im
hintergrund fuer die detection“, Fenster erkennen „ist gut“. Toggle: Strg
halten rastet ein; Fenster gegen Inhalt: Mausrad wechselt die Ebene. „lass
erstmal alles erproben … damit die erste implementierung schon near perfekt
wird“.

Status: **Probephase**. Kein Code im Produkt, bis die Proben unten Zahlen
haben und das Verhalten feststeht.

## Verhalten (Entwurf, wird nach den Proben festgelegt)

- Strg halten im Selector (Aufnahme- und Screenshot-Modus): das kleinste
  erkannte Rechteck unter dem Zeiger wird hervorgehoben (Rahmen wie die
  Auswahl, gedimmt außen), daneben kurz, was es ist („Video 1362 × 765“,
  „Brave“, „DP-3“).
- Mausrad bei gehaltenem Strg: eine Ebene nach außen (Video → Fenster →
  Bildschirm) oder innen. Die Ebene bleibt, solange der Zeiger im selben
  Rechteck der äußeren Ebene bleibt.
- Klick bei gehaltenem Strg: das Rechteck wird die Auswahl (Editing, mit
  Griffen). Strg loslassen ohne Klick: nichts.
- Ohne Strg: alles wie heute.
- Kanten nachschärfen nur intern: das Video-Rechteck aus der Bewegung wird
  an die nächsten langen geraden Kanten gezogen; gezeigt wird nur das
  Ergebnis.

## Proben

| Probe | Frage | Stand |
| --- | --- | --- |
| P1 Fenster | Woher, welche sind sichtbar, wie in Surface-Koordinaten | gemessen (unten) |
| P2 Eingabe | Kommen Mausrad und Strg beim Selector an, belegt Hyprland sie | gemessen (unten) |
| P3 Bewegung | Findet ein Frame-Vergleich das Video-Rechteck, wie genau, welche Störer | gemessen (unten) |
| P4 Kanten | Wie weit rückt die Kantensuche das Bewegungs-Rechteck auf die echte Videokante | gemessen (unten) |
| P5 Zeit | Frames aus dem Puffer holen und rechnen | gemessen (unten) |
| P6 Ohne Bewegung | Was liefern Kanten allein aus einem Bild | gemessen (unten) |

**P1 (27.10., nur gelesen):** `hyprctl clients -j` liefert je Fenster `at`,
`size`, `workspace`, `monitor`, `floating`, `fullscreen`, `hidden`;
`hyprctl monitors -j` Ursprung, Größe, Skalierung, aktiven und speziellen
Workspace. Sichtbar sind die Fenster des aktiven (und speziellen) Workspaces
des Monitors. Hier: DP-3 bei 0,0 und DP-1 bei 1920,0, beide 1920 × 1080,
Skalierung 1, Fenster mit 10 px Abstand unter der Leiste (`at [10, 38]`,
`size [1900, 1032]`). Surface-Koordinaten = global − Monitor-Ursprung,
geteilt durch die Skalierung. Offen: übereinanderliegende Floating-Fenster
(Reihenfolge), Vollbild.

**P2 (27.10., Code gelesen):** SCTK liefert `PointerEventKind::Axis`
(`smithay-client-toolkit-0.19.2/src/seat/pointer/mod.rs:109`) und
`Modifiers::ctrl` (`seat/keyboard/mod.rs:268`); der Selector hat exklusive
Tastatur (`select_skia/mod.rs:298`) und wertet heute nur `alt` aus
(`input.rs:400`). Hyprland-Binds des Nutzers belegen kein Mausrad
(`~/dotfiles/hypr/hyprlua/*.lua`). Strg ist im Selector frei.

## Material für P3–P6

Der Nutzer wollte nicht gestört werden. Der Replay-Puffer nimmt DP-3 auf
(dort lief nur ein Terminal), das Video lief auf DP-1 (Crunchyroll in Brave,
eingebettet, nicht Vollbild). Daher sechs stille Vollbild-Aufnahmen
`boltsnap full --no-copy -o -` (ohne Shelf, ohne Zwischenablage) im Abstand
von ~1 s, DP-1 ausgeschnitten, nur im Scratchpad (`scratchpad/snap/`, nicht
ins Repo: fremdes Bildmaterial). Sollwert: die Videokante pixelweise aus dem
Bild gelesen (Seite links/rechts/unten `#000`, oben die Kopfleiste `#272727`):
erste Videospalte 382, letzte 1701, erste Zeile 138, letzte 879, also
`(382, 138)–(1702, 880)` exklusiv, 1320 × 742.

**P3 Bewegung:** Viertelauflösung, Grauwert, |Differenz| > 6 über
aufeinanderfolgende Paare vereint, Zeilen/Spalten mit > 5 % Bewegung:
`(380, 136)–(1704, 880)`, bis 2 px daneben (Viertel-Raster). Befund: drei
Aufnahmen in 1 s waren pixelgleich (Anime hält Einstellungen mehrere
Sekunden), ein Paar von fünf hatte nur 9 156 geänderte Pixel. Bewegung
braucht also mehrere Frames über ≥ 2 s und versagt bei Standbild und
Pause.

**P4 Kanten nach Bewegung:** stärkste lange Kante je Seite in ±12 px um das
Bewegungs-Rechteck (mittlerer Gradient über die Gegenrichtung):
`(382, 138)–(1702, 880)`, **exakt** der Sollwert.

**P6 Kanten allein (ein Bild):** vom Zeiger in vier Richtungen bis zur
ersten Kante, die über die Breite/Höhe des bisher gefundenen Rechtecks zu
≥ 90 % durchläuft (Gradient > 20); Start ±6 px um den Zeiger, wiederholt bis
stabil. Zeiger in der Mitte, an den Untertiteln, 12 px von der oberen linken
Ecke: alle drei **exakt** `(382, 138)–(1702, 880)`. Zwei Fehlversuche
vorher, festgehalten: mit fester Spanne ±150 px nahe am Rand (Spanne ragt
übers Video) → Seitenrand statt Videokante; Start mit dem ganzen Bild → das
Fenster (das ist die Fenster-Ebene, nicht die innere). Außerhalb des Videos
(Seitentitel, Kopfleiste) liefert es Seitenbereiche bzw. das Fenster, dort
ist das Fenster aus Hyprland die richtige Ebene.

**P5 Zeit:** Python/numpy: Kantenmasken für 1920 × 1080 ~20 ms einmal je
Bild, Suche ~27 ms je Punkt (Python-Schleifen; in Rust ist die Suche ein
paar hundert Speicherzugriffe, Schätzung < 1 ms, live beim Bewegen).
Bewegung braucht Frames aus dem Puffer: letzte Sekunde 1080p60 auf 4
kleine Frames dekodiert 0,60 s (Software, `-threads 2`) bzw. 0,55 s
(NVDEC), synthetische Quelle aus der C2-Probe, inklusive ffmpeg-Start.

## Ergebnis der Proben

Kanten allein treffen das Video aus einem Bild exakt, sofort, in jedem
Modus (auch Screenshot und pausiert). Bewegung kostet ~0,5 s und mehrere
Sekunden Material und ist nur nötig, um ein Rechteck als „Video“ zu
benennen oder wenn das Bild selbst lange gerade Kanten hat. **Vorschlag
erste Fassung:** Ebenen = Kanten-Rechteck um den Zeiger (innen), Fenster
aus Hyprland, Bildschirm; Bewegung erst, wenn Kanten in der Praxis
danebenliegen. Tests mit synthetischen Fixtures (Seite, eingebettetes
Video, Kopfleiste, Untertitel-Kontrast), die echten Aufnahmen nur zum
Nachsehen im Scratchpad.

## Macht nicht (in der ersten Fassung)

Keine KI-Modelle (Rechtecke findet einfache Bildverarbeitung schneller und
hardwareunabhängig), keine Browser-Anbindung, keine Erkennung von
Elementen außer Video, Fenster, Bildschirm.

## Erste Fassung: Bauplan (28.10., Nutzer: „ja dann fang an mit dem bauen“)

**Verhalten (fest):**
- Strg halten, Selector in beiden Modi, mit oder ohne Auswahl: das Rechteck
  der aktuellen Ebene unter dem Zeiger erscheint so, wie es als Auswahl
  aussähe (Ring, außen gedimmt, Größenanzeige), ohne Griffe; die
  Aufnahme-Leiste blendet aus, solange es eine Vorschau ist. Rahmen und
  Auswahl springen ohne Animation (auf der „Ohne Animation“-Liste des
  Selectors: Mitziehen, Rahmen, Griffe).
- Ebenen, innen nach außen, jede enthält den Zeiger: Kanten-Rechteck aus dem
  Bild des Selectors, sichtbare Fenster aus Hyprland (kleinstes zuerst),
  der Bildschirm unter dem Zeiger. Fast gleiche (≤ 4 px an jeder Kante)
  zählen einmal.
- Mausrad hoch bei gehaltenem Strg: eine Ebene nach außen, runter: nach
  innen, an den Enden bleibt es stehen. Ändert sich die Liste der Ebenen
  (anderes Fenster, anderes Rechteck), beginnt es wieder innen.
- Klick bei gehaltenem Strg: das Rechteck wird die Auswahl (Editing, Griffe,
  Leiste); im Sofort-Modus (`--instant`) wird es gleich genommen.
- Strg loslassen: Vorschau weg, die bisherige Auswahl (oder keine) wie vorher.
- Ohne Bild (Aufnahme ohne Replay-Puffer): nur Fenster und Bildschirm.
  Ohne Hyprland: nur Kanten und Bildschirm.
- Ein Kanten-Rechteck unter 48 × 48 px (ein Buchstabe, ein Icon) zählt nicht.

**Bild:** `Selector::image` (Screenshot, oder das eingefrorene Replay-Frame im
Aufnahme-Modus, `select_skia/mod.rs:339`), in Bildpixeln; Zeiger und
Ergebnis über das Verhältnis Bild/Surface umgerechnet. Die Kanten werden
beim ersten Strg aus dem Bild gebaut und neu, wenn das Bild wechselt.

**Dateien:**
- `src/selector/snap.rs` (plattformneutral): `Edges::new(&RgbaImage)` —
  Helligkeit, Kantenflag |Δ| > 20 zwischen Nachbarn, je Spalten- und
  Zeilengrenze eine Präfixsumme (`u16`), damit „läuft die Kante zu ≥ 90 %
  über die Spanne“ O(1) ist; `Edges::around(point) -> Option<(x0, y0, x1,
  y1)>` wie Probe P6 (Start ±6 px, wiederholt bis stabil, höchstens 8
  Runden; das ganze Bild oder < 48 px ist `None`); `levels(inner, windows,
  screen, point) -> Vec<Rect>`.
- `src/platform/linux/select_skia/snap.rs`: Zustand (`Snap`: Kanten mit
  Bild-Kennung, Fenster einmal je Selector aus `hypr::json("clients")` und
  `("monitors")`, Ebenen, Index, Strg), `Selector`-Methoden für Strg,
  Mausrad, Klick und die gezeigte Auswahl; Fensterliste aus dem JSON als
  reine Funktion (sichtbar = `mapped`, nicht `hidden`, Workspace aktiv oder
  speziell auf seinem Monitor; in Surface-Koordinaten, auf die Surface
  beschnitten).
- `input.rs`: `update_modifiers` (Strg), `Axis`, Druck mit Strg.
- `present.rs`: gezeigte Auswahl = Vorschau, falls aktiv; `mod.rs`
  `sync_bar`: keine Leiste während der Vorschau.
- README (Selector-Bedienung), `docs/codebase.md`.

**Was brechen kann:** Strg-Klick darf nicht zusätzlich eine Zeichnung
beginnen (Druck verbrauchen); Leiste und Hinweis bei Vorschau; mehrere
Monitore im Screenshot-Modus (Bildschirm-Ebene = Monitor unter dem Zeiger);
Speicher der Präfixsummen (3840 × 1080: 2 × 8,3 MB, einmal je Selector).

**Tests:** synthetische Seite wie Probe (Kopfleiste, schwarzer Grund,
eingebettetes „Video“ mit Rauschen, Untertitel-Kasten mit Schrift-Kanten):
Mitte, an den Untertiteln, 12 px von der Ecke → exakt; Text außerhalb →
`None`; Vollbild-Video → `None`; Kante dunkel auf dunkel → kein falsches
kleines Rechteck. `levels`: Reihenfolge, Duplikate, nur mit Zeiger.
Fensterliste aus festem JSON (zwei Monitore, versteckte, anderer Workspace,
speziell, über den Rand). Ebenenwechsel und Rücksprung bei neuer Liste.
Zeitmessung `Edges::new` 1920 × 1080 im Test ausgegeben.

**Macht nicht:** keine Namen am Rechteck (die Größe steht ohnehin da), keine
Bewegungserkennung, keine Anpassung während die Auswahl schon gezogen wird.

**Live (mit Okay, nicht während der Nutzer schaut):** Strg im Selector über
einem eingebetteten Video, Mausrad, Klick, Clip.

**Stand 28.10.:** gebaut. Abweichungen vom Bauplan, beim Testen gefunden:
- Ein Punkt auf einem Buchstaben (Untertitel als Blockschrift) ergab nur den
  Buchstaben (< 48 px, also nichts). Jetzt sucht `innermost` dann erneut von
  dem kleinen Rechteck aus, um `MIN_SIDE` vergrößert, bis eins groß genug
  ist: die kurzen Kanten der Schrift zählen über die größere Spanne nicht.
- Ein Punkt auf einer scharfen Form im Bild (Test: „Lampe“ 60 × 220) findet
  zuerst die Form. Das ist eine echte innere Ebene; deshalb liefert
  `Edges::around` jetzt eine Kette: innerstes Rechteck, dann je das
  nächste, das es umschließt (Suche jenseits seiner Kanten), höchstens 6.
  Mausrad nach außen erreicht so das Video.
- Die Vorschau wird vor der normalen Zeigerbewegung aktualisiert, nicht an
  ihrer Stelle, damit Ziehen mit gehaltenem Strg weiterläuft.
Tests: `selector::snap` (Video exakt von Mitte, Untertitel, 12 px neben der
Ecke, unterer rechter Ecke; Lampe → [Lampe, Video]; Text und Vollbild ohne
inneres Rechteck; dunkle Kante schneidet nie ins Video; `levels`),
`select_skia::snap` (Fensterliste aus festem JSON: Workspace aktiv/speziell,
versteckt, ungemappt, Monitor-Ursprung, Beschnitt). Zeiten im Debug-Build:
Kanten für 1920 × 1080 187 ms (einmal beim ersten Strg), fünf Suchen
0,66 ms. Release nicht gemessen (live nachholen). `cargo test` 161 + 410
grün, fmt sauber; clippy `-D warnings` scheitert an 33 Stellen, die der neue
Toolchain (1.98.1, Lint `chunks_exact_to_as_chunks`) im übrigen Repo
findet, keine in den neuen Dateien; behoben im eigenen Commit `1efa6b2`
(`as_chunks::<N>()` statt `chunks_exact(N)`, ein `match` als `?`), clippy
`-D warnings` wieder still, Tests unverändert grün.
Offen: live (Strg über einem eingebetteten Video, Mausrad, Klick, Clip).

## Nachtrag 28.10.: Fenster kam nie (Nutzer, live)

O-Ton: „detected es oft den fensterrand von termo nicht richtig und das
ganze fenster hat es nicht ein mal selected“.

**Gemessen** (ignorierter Test `select_skia::snap::tests::snap_probe` auf
einer stillen Aufnahme von DP-3 mit `hyprctl -j clients/monitors`): termo
ist halbtransparent, seine Seiten haben gegen die Lücke keine Kante über
90 %. Die Suche fand stattdessen die waagerechten Linien im Terminal und
lieferte Streifen bis an den Bildschirmrand, z. B. `(0, 28)–(1910, 608)`,
`(0, 0)–(1920, 609)`: fünf solche Ebenen vor dem Fenster (`(10, 38) 1900 ×
1032`, Ebene 4), alle über den Fensterrand hinaus.

**Fix:** ein Kanten-Rechteck zählt nur mit echten Kanten an allen vier
Seiten (`Edges::closed`; ein Streifen zwischen zwei Linien hat zwei), auch
für jede äußere Stufe der Kette; und ein Fenster oder der Bildschirm ersetzt
ein Kanten-Rechteck, das ihm gleicht (≤ 4 px), statt umgekehrt. Ergebnis auf
derselben Aufnahme: oben und Mitte `[Fenster, Bildschirm]`, im unteren
Bereich `[(10, 889) 1900 × 181 (Eingabe und Statuszeile), Fenster,
Bildschirm]`. Tests: `bands_between_lines_across_the_window_are_no_rectangle`,
`levels` mit einer Fensterlesung 2 px innen → das Fenster. `cargo test`
161 + 411 grün, clippy `-D warnings` still.
