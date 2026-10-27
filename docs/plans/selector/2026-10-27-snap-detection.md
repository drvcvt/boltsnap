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
