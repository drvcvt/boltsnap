# Performance-Änderungen vom 20. September 2026

Die Änderungen betreffen den Linux-Pfad. Das vorhandene Arbeitsverzeichnis
enthielt bereits Replay- und Portal-Arbeit; diese wurde beibehalten. Der laufende
Daemon und die Benutzerkonfiguration wurden für diese Prüfung nicht ersetzt.

## Implementiert

- **Screenshot-Eingang:** höchstens 16 laufende oder wartende Client-Aufträge,
  davon höchstens zwei mit Bild-Payload. Die Bildreservierung erfolgt vor der
  Payload-Allokation und bleibt bis zur Verarbeitung im Eventloop bestehen.
  Auch Antwort-Threads sind auf 16 begrenzt. Bei Überlast wird die Anfrage
  abgewiesen; der Client muss sie gegebenenfalls erneut auslösen. PNG-Decoding
  hat ein 256-MiB-Allokationsbudget und maximal 64 Mi-Pixel vor RGBA-Konvertierung.
  Das ist keine Obergrenze des gesamten Daemon-RSS.
- **Selector:** einmal abgedunkelter Hintergrund und wiederverwendeter
  Arbeitspuffer. Auswahlpixel werden zeilenweise aus dem Original restauriert.
  Pixelgleichheit zu vorher ist für transparente und deckende Bilder sowie
  abgeschnittene und gebrochene Auswahlkoordinaten getestet. Die zwei
  zusätzlichen dauerhaften RGBA-Puffer benötigen jeweils rund 31,6 MiB bei 4K.
  Frame-Callbacks begrenzen weiterhin das Zeichnen; es gibt noch volle
  Oberflächenschäden und eine volle Pixelkonvertierung pro gezeichnetem Bild.
- **Video-Thumbnails:** ein Hintergrundworker mit maximal 20 wartenden Jobs.
  FFmpeg skaliert direkt auf 190×132 RGBA, mit begrenzten Threads, 10 Sekunden
  Zeitlimit und begrenzter Pipe-Ausgabe. Kein großes PNG als Zwischendatei.
  Bei voller Queue oder Decoderfehler bleibt der Platzhalter bestehen.
  Der alte Thumbnail-IPC-Pfad wird ebenfalls außerhalb des UI-Threads gelesen.
- **Replay-Vorschau:** nur die Monitor-/Session-Zuordnung wird beim Start kurz
  abgefragt, mit 100-ms-Lese- und Schreibtimeout. Freeze, Preview-Export und
  PNG-Decoding laufen im Hintergrund. REC und Escape bleiben währenddessen
  nutzbar; CLIP wird erst mit gültigem Snapshot aktiviert. Session-IDs schützen
  vor veralteten Antworten. Abbrechen startet keinen Daemon und wartet nicht
  auf die Worker-Antwort.
- **Leerlauf:** ohne Animation oder aktive Aufnahme wartet der Shelf-Eventloop
  auf Ereignisse. Laufende Aufnahmen behalten ihre 250-ms-Prüfung für Timer,
  unerwartete Recorder-Enden und die periodischen Speicherplatzprüfungen.
- **PNG-Pfad:** RGB-Konvertierung und Fenster-Randbeschnitt erfolgen vor dem
  einzigen Encoding. Der gewöhnliche Wayland-Shelf-Pfad und stdout benötigen
  keine anfängliche Capture-Zwischendatei. `last.png`, Shelf-Dateien und die
  eigene Datei für explizites Clipboard-Copy bleiben entsprechend ihrer
  bisherigen Lebensdauer bestehen. Thumbnail-Cropping vermeidet eine weitere
  Vollbildkopie.
- **Replay-Snapshots:** unveränderliche `Arc<Packet>` teilen auch die nativen
  Paketheader. Ein Snapshot kopiert weiterhin Ring-Deskriptoren, erzeugt dabei
  aber keine FFmpeg-Referenz pro Paket. Erst der Export erzeugt eigene
  `AVPacket`-Referenzen, bevor er Zeitstempel ändert. Paketbudget, Audio- und
  Videofenster, Eviction und Export-Limits bleiben erhalten.
- **Optionales Aufnahmeprofil:** `record_profile = "quiet"` begrenzt neue
  gewöhnliche Aufnahmen auf 60 fps. `"quality"` bleibt der Default mit 240 fps.
  Encoder-Qualität, Audio und Aufnahmeziele bleiben gleich. Pause/Fortsetzen
  behält das Profil der Sitzung; ungültige Profilwerte ergeben beim Start einen
  Fehler. Replay hat eine separate FPS-Einstellung. Die Benutzerkonfiguration
  wurde nicht automatisch auf `quiet` umgestellt.

## Messungen

Optimierte Rust-Harnesses mit den tatsächlichen Render-/Ring-Funktionen und
lokalen Release-Abhängigkeiten; keine Desktop-Aufnahme. Die Werte sind lokale
Mikrobenchmarks, keine Ende-zu-Ende-Latenzen oder Spiel-Frametimes.

Selector: je fünf Aufwärm- und 100 Messdurchläufe, deckendes synthetisches Bild,
bewegte Auswahl mit 75 % Breite/Höhe. Enthalten sind Hintergrundaufbau,
Auswahlwiederherstellung und RGBA→BGRA-Konvertierung; nicht enthalten sind
Toolbar, Lupe, Wayland-Übertragung und Compositor.

| Auflösung | Vorher Median / p95 | Nachher Median / p95 |
| --- | --- | --- |
| 1920×1080 | 7,335 / 8,510 ms | 1,858 / 2,991 ms |
| 2560×1440 | 13,488 / 15,400 ms | 3,728 / 5,238 ms |
| 3840×2160 | 30,201 / 33,089 ms | 8,143 / 9,470 ms |

Snapshot inklusive letzter GOP: fünf Aufwärm- und 30 Messdurchläufe,
Einsekunden-GOPs, 512-Byte-Videopakete und rund 47 Audiopakete/s mit 128 Bytes.
Das Freigeben des Snapshots liegt außerhalb der Messzeit. Gemessen wurde der
Paket-/Ring-Teil der Freeze-Arbeit, ohne Preview-Decoding, Stream-Metadatenkopie
oder konkurrierenden Capture-Thread. Native FFmpeg-Referenzen wurden im
Vorher-Lauf wirklich erzeugt, nicht durch Dummy-Zähler ersetzt.

| Historie | Vorher Median / p95 | Nachher Median / p95 |
| --- | --- | --- |
| 60 s / 60 fps | 0,645 / 1,398 ms | 0,141 / 0,154 ms |
| 600 s / 240 fps | 21,838 / 29,493 ms | 4,916 / 6,237 ms |

Der Snapshot bleibt linear zur Paketanzahl; der Maximalfall ist nicht kostenlos.
Die Cache-Verzeichnisprüfung blieb unverändert: lokale warme Scans über
20 / 200 / 2.000 Dateien brauchten median 0,011 / 0,096 / 1,057 ms.
Langsame oder entfernte Dateisysteme wurden nicht simuliert.

Ein isolierter Calloop-Test verwendet die tatsächliche Timeout-Entscheidung
und einen einzigen Abschluss-Event nach zehn Sekunden. Er vergleicht die
bisherige 250-ms-Frist mit ereignisgesteuertem Warten: 39 periodische Rückgaben
vorher, null nachher; beide verarbeiten den Abschluss-Event. Das ist kein
Nachher-RSS- oder CPU-Nachweis des vollständig laufenden Daemons.

## Verifikation

- Hauptprojekt: `cargo fmt --check`, `cargo test --locked --all-targets`,
  `cargo check --locked --all-targets` und Release-Build.
- 240 reguläre Hauptprojekt-Tests; der zusätzliche externe FFmpeg-Test wurde
  ausdrücklich mit `cargo test --locked synthetic_video_produces_a_small_rgba_thumbnail -- --ignored`
  ausgeführt. Er prüft die Größe, rote RGBA-Pixel und Fehlerbehandlung mit einem
  synthetischen Video.
- 21 Worker-Tests, Worker-Formatierung und Worker-Release-Build.
- Worker `tests/probe.py`: Decoding, Video-/Audio-Identität, Zeit- und
  Speichergrenzen, Schutz vorhandener Dateien, Ablehnung von B-Frames, Pipe.
- Worker `tests/live.py`: eingefrorener Ausschnitt, gleichzeitige Statusabfrage,
  Exportlimit, veraltete Snapshot-ID, Vollbildclip, Audio und Decoding.
- Beschleunigter Worker-Soak mit 2.400 Wiederholungen der 12-s-Fixture:
  3.081.600 Pakete in 13,03 s, rund acht Stunden Medienzeit. 1.581.723 Bytes
  maximale Paketabrechnung bei 8.388.608 Bytes Budget; RSS-Spitze 25.048 KiB,
  nach Aufwärmen −12 KiB Veränderung. Ausgabe erfolgreich decodiert.
  Das ersetzt keinen achtstündigen Echtzeit-Capture-Test.

Lokale Messquellen und Rohwerte:
`/tmp/boltsnap-performance-after.jsj4arbx/`.
Medientests: `/tmp/boltsnap-replay-test.pw9kzw49/`,
`/tmp/boltsnap-replay-live.6f_xrkyl/`,
`/tmp/boltsnap-replay-soak.d10ubz2f/`.
Die temporären Artefakte sind nicht Bestandteil des Repositorys.

## Noch auf echter Hardware zu prüfen

Selector-Interaktion auf Wayland einschließlich mehrerer Monitore/Skalierungen,
Clipboard, Aufnahme/Pause/Fortsetzen mit `quiet`, frühes REC/Escape während
Replay-Vorschau, sowie reale Aufnahme mit Audio. Danach Spiel-Frametimes,
GPU-/CPU-Leistungsaufnahme, Lüftergeräusch und langfristige RSS-Entwicklung mit
und ohne Aufnahme vergleichen. Eine Garantie für unsichtbare Auswirkungen
oder geringere Lautstärke ist aus diesen synthetischen Tests nicht ableitbar.

`--no-dmabuf` bleibt erhalten. Der vorhandene GPU-Crop-Pfad bleibt deaktiviert,
weil der frühere Pixelvergleich fehlschlug. Die gemessenen Verbesserungen
rechtfertigen keine Änderung dieser Hardware-Kompatibilitätsentscheidungen.
