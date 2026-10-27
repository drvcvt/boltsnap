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
| P3 Bewegung | Findet ein Frame-Vergleich das Video-Rechteck, wie genau, welche Störer (Cursor, Webcam-Overlay, animierte Vorschaubilder, Steuerleiste) | offen, braucht echtes Material |
| P4 Kanten | Wie weit rückt die Kantensuche das Bewegungs-Rechteck auf die echte Videokante, auch bei dunklen Rändern im Bild | offen, braucht echtes Material |
| P5 Zeit | Zweites Frame aus dem Puffer holen (Worker) und rechnen: wie lange, auf welchem Thread | offen |
| P6 Ohne Bewegung | Screenshot-Modus und pausiertes Video: was liefern Kanten allein | offen |

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

Ein laufendes YouTube-Video im Browser auf dem Replay-Display (normale
Seite, nicht Vollbild), dann ein Vollbild-Clip aus dem Puffer. Daraus im
Scratchpad Frames im Abstand 0,25 / 0,5 / 1 / 2 s, die echte
Videokante von Hand aus dem Frame gemessen als Sollwert.

## Macht nicht (in der ersten Fassung)

Keine KI-Modelle (Rechtecke findet einfache Bildverarbeitung schneller und
hardwareunabhängig), keine Browser-Anbindung, keine Erkennung von
Elementen außer Video, Fenster, Bildschirm.
