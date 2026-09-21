# Screenshot- und Selector-Optimierung, 21. September 2026

Ausgangspunkt: `398e44e`. Optimierung von Bildkopien, Bildübergabe,
Selector-Rendering und Startaufwand, ergänzt um kompaktere Selector-Elemente
mit größerer Schrift.

## Aktiv im normalen Linux-Pfad

- Capture übernimmt vorhandene RGBA-Puffer mit `into_rgba8`.
- Capture, Selector und Shelf verwenden den tatsächlich aufgenommenen Output.
  Fokusänderungen während des Starts verschieben das eingefrorene Bild nicht auf
  einen anderen Monitor. Bekannte Änderungen von Ursprung, logischer Größe oder
  Rotation sowie das Entfernen des Zielmonitors brechen die Auswahl ab.
- Portal-Cropping berücksichtigt auch negative Desktop-Ursprünge.
- Ein Font-Auftrag läuft parallel zu Capture und Wayland-Setup. Vor der ersten
  sichtbaren Darstellung ist die Schrift installiert; kein nachträglicher
  Font-Wechsel. Kurze `fc-match`-, `gsettings`- und `hyprctl`-Aufrufe verwenden
  begrenzte Ausgabe und einen 500-ms-Timeout mit bestehender Prozessbereinigung.
  Font-Auflösung und Dateiinhalt werden weiterhin für beide Gewichte geteilt.
- Der Selector behält einen fertigen Frame. Änderungen restaurieren nur die
  betroffenen Bildbereiche, inklusive alter/neuer Umrandung, Griffe, Toolbar,
  Größenanzeige und Lupe. Identische sichtbare Zustände erzeugen keinen Commit.
- Höchstens drei SHM-Puffer, bedarfsgesteuert angelegt. Ein aktiver Puffer wird
  ausschließlich nach `wl_buffer.release` wieder beschrieben. Auch alte Größen
  zählen bis zur Freigabe gegen dieses Limit. Nach jeder Event-Dispatch-Runde
  werden wartende Frames geprüft, ohne zusätzliche Timer.
- Eine Historie von acht Frame-Änderungen repariert wiederverwendete Puffer.
  Fehlende Historie, ein neues Hintergrundbild und komplexe/großflächige Schäden
  führen zur Vollkopie. Oberflächenschaden und Pufferreparatur werden getrennt
  berechnet.
- Im normalen PNG-IPC wird der vorhandene PNG-Puffer direkt geschrieben, ohne
  ihn vorher in einen zweiten Header-plus-Payload-Puffer zu kopieren. Das
  bestehende Wire-Format bleibt kompatibel.

## Selector-UI

Toolbar- und Größenanzeigen-Schrift: **21 statt zuletzt 15 px**. Die Größenanzeige
ist 22 statt 24 px hoch. Klickflächen bleiben 24 px hoch. Äußerer Toolbar-Rand:
1 statt 3 px. Die normale Toolbar ist somit insgesamt 26 statt 30 px hoch.
Toolbar-Icons sind 18 px groß, mit 2,4-px-Strichen, runden Enden und gebogenen
Konturen. Für die Lesbarkeit bleiben seitlich am Icon und zwischen Icon und
Text je 5 px, rechts am Text 6 px und zwischen Aktionen 4 px Abstand.
Die Größenanzeige behält ihren schmalen 3-px-Textrand. Innenradien der Toolbar
folgen dem Außenradius abzüglich Rand (8 zu 9 px); ein Renderingtest prüft,
dass Hover-/Aktivflächen auch bei gebrochenen Koordinaten und allen
Layoutvarianten nicht über die äußere Kontur hinausragen.
Breiten folgen weiterhin der gemessenen Schrift. Randplatzierung, 2×2- und
Einspalten-Fallback sowie dieselben Geometrien für Darstellung und Hit-Testing
bleiben erhalten. Synthetische Darstellungen mit variablen UI-Schriften wurden
visuell geprüft. Die Schriftfamilie bleibt über `ui_font` konfigurierbar.

## Messung des Renderers

Release-Build, synthetisches Bild, fünf Aufwärm- und 100 Messframes pro Fall.
Verglichen werden der bisherige vollständige CachedOverlay-Reset samt BGRA-Kopie
und der regionale Pfad samt Schadensermittlung, Dekorationen und Reparatur von
drei abwechselnd benutzten Zielpuffern. Beide verwenden dieselben damaligen
UI-Maße mit 15-px-Schrift, vor der anschließenden Anpassung auf 21 px.
Das sind CPU-Mikrobenchmarks, keine Messungen tatsächlicher Wayland-Präsentation.

| Kleiner Drag | Vollbild Median / p95 | Regional Median / p95 | kopierte Bytes/Frame vorher → nachher |
|---|---:|---:|---:|
| 1920×1080 | 2,308 / 3,362 ms | 0,664 / 0,729 ms | 8.294.400 → 323.055 |
| 2560×1440 | 4,112 / 4,999 ms | 0,948 / 1,035 ms | 14.745.600 → 428.700 |
| 3840×2160 | 8,281 / 9,734 ms | 1,445 / 1,518 ms | 33.177.600 → 638.453 |

Bei 4K: Hover 8,405 → 1,400 ms, Lupe 8,439 → 1,409 ms,
große Positionssprünge 8,429 → 4,686 ms, jeweils Median. Der kleine Drag
benötigt damit in diesem Test rund 83 % weniger CPU-Zeit pro Frame und 98 %
weniger Bytes in der BGRA-Pufferkopie. Die einmaligen Hintergrund-/Dimmed-/Frame-
Allokationen verschwinden dadurch nicht. Drei 4K-SHM-Puffer belegen zusammen
rund 95 MiB; der übliche Zweipufferfall rund 63 MiB, zusätzlich zu den Bildern
und Render-Caches. Das sind berechnete Puffergrößen, keine gemessenen RSS/PSS-Werte.

Reproduzierbar ohne Desktop-Zugriff:

```sh
cargo run --release --example selector_bench -- /tmp/boltsnap-selector-ui.png
```

## Rohbild-IPC: implementiert, standardmäßig deaktiviert

Der neue Linux-Pfad übergibt eng gepackte RGB8-Pixel als versiegeltes `memfd`
über `SCM_RIGHTS`. Der Daemon erstellt direkt das kleine Thumbnail und genau
**ein** PNG. Die Implementierung ist vorhanden, besteht die unten genannten
Tests und lässt sich für weitere Messungen mit `BOLTSNAP_RAW_TRANSFER=1`
explizit benutzen. Der normale Weg bleibt PNG, weil der Prototyp die geplante
Abnahmegrenze nicht erfüllt hat.

Nach zwei Vorläufen bestätigt ein zusätzlicher Lauf ohne parallel gestartete
Builds, Tests oder andere Benchmarks bei 4K:

| Bild | bisheriges PNG Median / p95 | Rohbild Median / p95 | Entscheidung |
|---|---:|---:|---|
| gut komprimierbare Flächen/Gradienten | 31,938 / 36,242 ms | 37,039 / 38,877 ms | Verschlechterung |
| detailreiches Rauschen | 104,300 / 120,123 ms | 99,518 / 113,626 ms | begrenzter Gewinn |

Die gut komprimierbaren 4K-Bilder waren auch in beiden Vorläufen langsamer:
32,436 → 36,142 ms und 33,458 → 40,055 ms, jeweils Median. Damit rechtfertigt
die Dekodierersparnis keine generelle Aktivierung. Das Befüllen des
Rohbild-memfd und der größere Datensatz haben ebenfalls Kosten.

Der Benchmark umfasst Encoding, lokale Socket-Übertragung/FD-Aushandlung,
Mapping bzw. Decoding und Thumbnail. Er umfasst keine Dateischreibvorgänge,
Clipboard-Bereitschaft oder sichtbare Shelf. Die PNG-Referenz enthält die
vorherige zusammenhängende Request-Allokation; der jetzt aktive PNG-Pfad spart
auch diese Kopie ein. Die Vorläufe überschnitten sich teilweise mit anderer
Verifikationsarbeit; maßgeblich ist der anschließende separate Lauf. Für eine
künftige Aktivierung fehlen belastbare vollständige Messungen einschließlich
Ressourcen.

```sh
cargo run --release --example image_transfer_bench
```

Eigenschaften des optionalen Pfads:

- Dieselben Limits wie bisher: 16 Client-Aufträge, zwei Bildaufträge; Reservierung
  vor READY, beibehalten bis zur Verarbeitung des vorbereiteten UI-Events.
- Gleiche UID über `SO_PEERCRED`, exakt ein FD, `CLOEXEC`, begrenzte Header und
  positive/geprüfte Maße bis 64 Mi-Pixel. Keine vom Client gewählten Dateipfade.
- Unveränderliche Länge und Daten vor dem read-only Mapping. Benötigt
  `F_SEAL_WRITE`, `F_SEAL_SHRINK`, `F_SEAL_GROW`, `F_SEAL_SEAL`;
  `F_SEAL_FUTURE_WRITE` allein reicht ausdrücklich nicht.
- Empfangene FDs gehören sofort einem `OwnedFd`, auch bei zurückgewiesenen
  Zusatz-FDs oder abgeschnittenen Ancillary-Daten. Die Linux-Verträge sind in
  [unix(7)](https://www.man7.org/linux/man-pages/man7/unix.7.html),
  [recvmsg(2)](https://www.man7.org/linux/man-pages/man2/recvmsg.2.html) und
  [F_GET_SEALS](https://man7.org/linux/man-pages/man2/F_GET_SEALS.2const.html)
  beschrieben.
- Ein alter Daemon kann vor READY den nicht mutierenden Handshake schließen;
  dann folgt der vorhandene PNG-Pfad. Überlast erzeugt einen Fehler. Nach
  möglicher FD-Übergabe gibt es keinen automatischen Retry oder Fallback.
- ACK erst nach Dateivorbereitung und Shelf-Modellaufnahme. Ein verlorenes ACK
  meldet den unklaren Annahmestatus. Clipboard-Erfolg bezeichnet weiterhin nur
  den gestarteten Helper, nicht nachgewiesene Clipboard-Bereitschaft.
- Eigenständige temporäre Clipboard-Datei und atomar aktualisiertes `last.png`.
  Unveröffentlichte Shelf-Dateien werden beim Verwerfen des Jobs aufgeräumt.
  Eine erfolgreiche Aktualisierung von `last.png` wird bei späteren Fehlern
  nicht zurückgerollt, damit keine neuere Aufnahme überschrieben wird.

## Messpunkte und bewusst offene Punkte

`BOLTSNAP_TIMINGS=1 boltsnap area` schreibt numerische Request-ID, Prozess-ID,
monotone Zeit und Stufendauer auf stderr. Der neue Daemon übernimmt die ID bei
PNG und RGB aus dem Header. Seine Zeilen erscheinen in seinem stderr/Journal.
Messpunkte umfassen Prozesseintritt, Ziel, Wayland-Verbindungen, Capture,
Font-Auftrag, Configure, ersten Selector-Commit, Auswahl/Crop, RGB/Trim,
Encoding/Transport, Shelf-Vorbereitung, Modell-ACK und Shelf-Commit.
Ohne Variable bleiben die Messpunkte still; keine Hintergrundmessung oder
zusätzliche Idle-Timer. Keine Pixel, Fenstertitel, Ausgabennamen oder Pfade im Log.
`capture_wayland_total` enthält auch die Zeit, die der Nutzer auswählt.
Commit und Frame-Callback sind kein Presentation-Time-Nachweis.

Nicht aktiviert/implementiert:

- Der direkte `screenshot_single_output`-Shortcut. libwayshot 0.7.3 liefert dort
  ein Bild vor der allgemeinen Transformation/Komposition; Screencopy-Flags
  werden nicht nach außen gegeben. Der aktivierte Pfad bleibt deshalb der
  bisherige Normalisierungspfad. Echte Orientierung-/Y-Invert-/Skalierungstests
  fehlen für eine Freigabe des Shortcuts.
- Ein dauerhaft laufender Capture-Prozess oder eine eigene Wayland-Library.
  Ohne gemessenen Startzeitvorteil rechtfertigen sie ihre Komplexität und
  zusätzlichen dauerhaften Ressourcen nicht.
- Hardwaretests mit Rotation, fractional scaling, Hotplug, KWin-Portal und X11;
  tatsächliches Compositor-Timing sowie CLI+Daemon-CPU/RSS/PSS unter Spielelast.
  Die synthetischen Ergebnisse beweisen keine geringere Lüfterlautstärke.

## Verifikation

- `cargo fmt --check`, `cargo check --locked --all-targets`,
  `cargo test --locked --all-targets`, `cargo build --release --locked`.
- 252 reguläre Hauptprojekt-Tests; zusätzlicher externer FFmpeg-Thumbnail-Test
  ausdrücklich mit `--ignored` ausgeführt. Die Beispiel-Binaries führen unter
  `--all-targets` zusätzlich ihre eingebundenen Modul-Tests aus.
- Pixelvergleich regional/vollständig über 480 Zustände mit Farbmuster,
  Transparenz, Randpositionen, Hover/Toggles, Lupe, drei Pufferständen,
  zusammengefassten Frames und abgelaufener Historie.
- Protokoll-/FD-Tests einschließlich Fragmentierung, gemischter Limits,
  unveränderlicher Daten, fehlender/zusätzlicher/trunkierter FDs und FD-Leaks,
  Legacy-EOF, verlorenem ACK, falschem Stride/Größe, PNG-Pixelidentität,
  Fehlern beim Publizieren/Clipboard-Start und Datei-Ownership.
- 21 Replay-Worker-Tests, Worker-Formatprüfung, synthetische `probe.py` und
  `live.py` bestanden. Kein produktiver Recorder oder laufender Daemon wurde
  für die Verifikation ersetzt.

Das Windows-Backend wurde nicht geändert. Die automatisierten Tests ersetzen
nicht die oben aufgeführten Hardware- und Compositor-Prüfungen.
