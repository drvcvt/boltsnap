# Replay: P0-Nachweise

Stand: 20. September 2026. P0 ist in Arbeit. Der Worker ist ein ausführbarer
Medien-Prototyp; Replay-Binds, Daemon-Anbindung und Bereichsauswahl sind noch
nicht implementiert. Die Gesamtarchitektur ist noch nicht abgenommen.

## Implementierter Umfang

- Isoliertes Rust-Paket mit eigenem Lockfile und `ffmpeg-next = 9.0.0`.
- Plattformneutraler GOP-Ring mit Zeitlimit, Bytebudget, Audio-Eviction und
  geprüftem Zeitstempelbereich; Tests laufen auch im Hauptprojekt.
- Die konfigurierte Dauer ist eine harte Obergrenze des Videofensters.
  Schlüsselbild-Abstände können es verkürzen. Eine bereits zu lange aktive
  GOP verfällt; die Historie beginnt erst am nächsten Schlüsselbild neu.
- Nativer Demuxer für endliche NUT-/Matroska-Teststreams und stdin.
- Echte Paketreferenzen mit geprüftem Fehlerergebnis. `Packet::clone()` des
  Wrappers wird vermieden, weil dessen Implementierung den Payload durch
  `av_packet_make_writable` kopieren kann.
- Remux nach Matroska mit gemeinsamem A/V-Zeitursprung, Schreiblimit,
  privater temporärer Datei und atomarer Veröffentlichung ohne Überschreiben.
- Abfrage der Encoder aus genau der verlinkten FFmpeg-Bibliothek.
- Reproduzierbare Medien- und Transporttests sowie separate Linux-CI-Jobs
  für Ubuntu 22.04 und 24.04. Die neuen CI-Jobs wurden lokal nicht als
  Ubuntu-Läufe ausgeführt.

Einstieg und Befehle: [Worker-README](../src/platform/linux/replay/worker/README.md).

## Lokaler Aufbau

| Komponente | Befund |
| --- | --- |
| GPU | NVIDIA GeForce RTX 4060, 8 GiB VRAM |
| Treiber | 610.43.03 |
| FFmpeg | 8.1.2; libavformat 62.12.102, libavcodec 62.28.102 |
| Lokale Hardwareencoder | Vulkan verfügbar und synthetisch erfolgreich initialisiert; NVENC, NVDEC und VA-API im FFmpeg-Build deaktiviert |
| Capture-Quellen | wf-recorder und GSR erkennen DP-1 und DP-3; keine reale Bildschirmaufnahme für diese Messungen |
| GPU Screen Recorder | Upstream-Commit `ab9cf696d013c59c02b7e98cbf103fd7735b0bbd` separat unter einem temporären Pfad gebaut; nicht installiert |

Die synthetische Standardquelle enthält 12 Sekunden 640×360 bei 60 FPS,
H.264/libx264 ohne B-Frames mit geschlossenen Einsekunden-GOPs sowie
48-kHz-AAC-Systemtonersatz. Der Sinuston ist eine Medienfixture, kein
Nachweis funktionierender Systemtonaufnahme.

## Vollbildclip und begrenzte Historie

Ergebnis aus `tests/probe.py`:

- Letzte drei Sekunden Video aus zwölf Sekunden Quelle exportiert; neun
  ältere GOPs entfernt.
- Alle decodierten Video-Frames stimmen per Hash mit dem entsprechenden
  Quellabschnitt überein. Kein Video-Reencode.
- Alle erhaltenen komprimierten Audiopakete stimmen per Hash mit dem
  überlappenden Quellabschnitt überein. Kein Audio-Reencode.
- Vollständiges Decodieren der Ausgabe mit `ffmpeg -xerror` erfolgreich.
- Audio-Paketüberhang erhalten; Matroska-Ausgabe ist durch Paketgrenzen und
  gemeinsame Zeitverschiebung etwas länger als das reine Videofenster.
- Bei kleinerem Bytebudget wird die Historie kürzer und bleibt decodierbar.
- Vorhandene Ausgabedatei bleibt bei erneutem Speichern unverändert.
- Nicht unterstützte B-Frame-/DTS-Struktur wird abgelehnt.
- Derselbe Pfad funktioniert mit einem echten komprimierten stdin-Pipe-Stream.
- Nach Verschärfung des Zeitlimits: Endpunkt zwischen Schlüsselbildern bei
  11,5 Sekunden und Drei-Sekunden-Limit ergibt 2,5 Sekunden decodierbares
  Video; alle Frame-Hashes stimmen mit dem Quellabschnitt überein.

Nachprüfung des harten Videolimits: 225 Hauptprojekt-Tests, 14 Worker-Tests
und der erweiterte synthetische Medientest bestanden. Linux-Check und
Worker-Clippy bestanden; Worker-Formatierung sauber. Der globale
`cargo fmt --check` meldet weiterhin die schon vorher vorhandenen
Abweichungen in `capture.rs`, `paths.rs` und `portal.rs`.

Referenzlauf vor der Verschärfung des Zeitlimits: 1.446 MiB zuletzt abgerechnete Paketbelegung, 2.001 MiB
Spitzenbelegung bei 8 MiB Budget. Das ist keine Obergrenze des gesamten RSS.
Die kurze Fixture prüft Zeitfenster und Paketidentität; unabhängiger A/V-
Impulstest, längerer Clock-Drift und Gerätewechsel stehen noch aus.

## Gefundener und umgangener Transportfehler

`write_index=0` allein reicht bei NUT nicht. Der FFmpeg-Demuxer sammelt
Syncpoints in einer zusätzlichen Struktur, auch wenn der eigene Ring
alte Pakete korrekt freigibt. Die Implementierung lässt sich im
[NUT-Demuxer](https://github.com/FFmpeg/FFmpeg/blob/n8.1.2/libavformat/nutdec.c)
nachvollziehen.

Vor der Verschärfung des Zeitlimits wurden 2400 Wiederholungen derselben Quelle durch eine echte
Pipe. Das sind 3.081.600 Pakete und etwa 8,02 Stunden Medienzeit, beschleunigt
auf rund 12,5 Sekunden Wandzeit. Beide Läufe nutzen denselben Drei-Sekunden-
Ring mit 8 MiB Budget.

| Transport | RSS am Laufende | Beobachtung |
| --- | ---: | --- |
| NUT, nur `write_index=0` | 116.092 KiB, etwa 113,4 MiB | Fortlaufendes Wachstum, abgelehnt |
| NUT, zusätzlich `syncpoints=none`, `strict=experimental` | 25.584 KiB, etwa 25,0 MiB | Nach Warmup stabil |

Beide Läufe endeten mit 1.516.256 abgerechneten Ringbytes und höchstens
2.098.286 Ringbytes. Der Unterschied liegt außerhalb des eigenen Rings.
Der Worker begrenzt außerdem die allgemeine FFmpeg-Indexreserve über
`indexmem=65536`; dies ersetzt das Deaktivieren der NUT-Syncpoints nicht.

Für den kontinuierlichen Adapter werden daher `write_index=0`,
`syncpoints=none`, `strict=experimental` und `flush_packets=1` gemeinsam
geprüft. Ein Adapter ohne diese Steuerungsmöglichkeit darf nicht einfach
denselben NUT-Pfad verwenden. Für diesen Fall bleibt Matroska mit gemessenen
Cluster-/Flush-Grenzen oder eine engere native Anbindung zu prüfen.

Das ist ein beschleunigter Allokations-/Transporttest. Daraus folgen weder
acht Stunden echte Betriebsstabilität noch ein Nachweis unsichtbarer
Aufnahmelast während eines Spiels.

## Hardware-Encoding und Crop

`h264_vulkan` initialisiert auf der lokalen RTX erfolgreich und erzeugt
eine synthetische Aufnahme mit AAC. Auch deren Drei-Sekunden-Clip lässt
sich korrekt remuxen und vollständig decodieren.

Der zunächst geprüfte Crop-Pfad lautet:

```text
Vulkan-Decode -> crop -> scale_vulkan -> Vulkan-Encode
```

Eine erste Einzelbildprobe bestand den Pixelvergleich. Die erweiterte
libx264-Fixture deckte anschließend einen Fehler auf: GPU- und CPU-Decode
des vollständigen Frames stimmen überein, der GPU-Crop weicht jedoch ab.
Bei 320×180 waren 3350 von 86400 YUV-Bytes unterschiedlich; die maximale
Abweichung betrug 208. Das ist kein akzeptabler Rundungsunterschied.

Diagnostik zeigt bei dieser Quelle 640×360 sichtbare Pixel, eine 640×368
Hardwarefläche und Decoder-Crop-Metadaten für acht untere Zeilen. Ein
Einfluss von Padding, Crop-Metadaten und Filterkoordinaten ist die aktuelle
Arbeitshypothese, noch kein vollständig bewiesener Fehlermechanismus.
`-apply_cropping 0` allein beseitigt den Fehler nicht.

Der native Filterpfad bleibt **nicht freigegeben**. `tests/probe.py --vulkan`
scheitert auf dieser Maschine bewusst am strengen Pixelvergleich. Es gibt
keinen stillen Software-Fallback und keine produktive Crop-Anbindung.
Nächster Schritt ist die Normalisierung der tatsächlichen Frame-/Crop-
Geometrie beziehungsweise ein anderer korrekt nachgewiesener Hardwarepfad.

## Noch offene P0-Gates

- Tatsächlicher GPU-Capture-Pfad mit Systemton und korrekt konfiguriertem
  kontinuierlichem Transport, einschließlich Verhalten des gewählten
  Capture-Tools bei fehlender Geräteunterstützung.
- Pixelkorrekter Hardware-Crop für gepaddete Frames und weitere Rechtecke.
- Endpunkt-/Flush-Latenz im warmen Zustand, inklusive Video und Audio.
- Wiederholte A/B-Messung gegen Capture allein und natives Tool-Replay,
  anschließend unter CPU-/GPU-Spielelast.
- Reale Zwei-/Achtstundenläufe und weitere Hardware; der beschleunigte
  Transporttest ersetzt diese Nachweise nicht.
- Tatsächliche Builds mit den älteren FFmpeg-Versionen der CI-Matrix.
- Produktionstauglicher Controller mit begrenzten Queues, Snapshot-
  Reservierungen, Abbruch und getrennten Ingest-/Exportpfaden.

Die Vorversuche rechtfertigen noch keine Integration eines vermeintlich
fertigen Replay-Services in den UI-Daemon.

## Linux service and shelf integration, 2026-09-20

Implemented a continuous worker plus shelf-owned supervisor, tray start/stop and
fullscreen save, the selector Clip button, frozen preview and crop-to-shelf.
Alt+Shift+Print is installed in the local Hyprland Lua configuration. The tray
menu was read back from the running daemon over D-Bus.

Validation after integration:

- Main crate: 233 tests passed; Linux check and release build passed.
- Worker: 20 tests passed; fmt and Clippy with warnings denied passed.
- Finite media regression: decode, packet/frame identity, GOP/time and byte
  bounds, no overwrite, rejected B-frames and stdin transport passed.
- Live synthetic H.264 Vulkan export: last-frame preview pixels, 60 FPS metadata,
  decoded frame counts, dimensions, audio, frozen interval, concurrent status
  and ingest, export exclusion and stale selection checks passed.
- Real shelf daemon with a synthetic capture executable: start returned while
  capture continued, fullscreen and cropped saves were acknowledged by the
  shelf, binary preview forwarding worked, and Stop reaped the capture/worker.
  This checks service ownership and shelf integration, not real screen capture.

Relevant local artifacts:

- `/tmp/boltsnap-replay-test.mtp8kw0j/` (finite regression).
- `/tmp/boltsnap-replay-live.v5kvb1x2/` (Vulkan, preview, FPS/frame count).
- `/tmp/boltsnap-replay-supervisor.lk8nphjd/report.json` (shelf integration).

The stricter FPS check found missing frame-rate metadata in the remuxer: packet
identity was correct, but Matroska reported 62.5 instead of 60 FPS. Stream frame
rate is now retained through snapshots and copied into muxer metadata. A 64×64
encoder smoke test also rejected this GPU's valid Vulkan encoder; the fixture
now uses 320×180 and the live test runs the encoder probe before capture.

GPU Screen Recorder 6.1.2 and its helper were built and copied beside the local
Boltsnap binary without elevated file capabilities. Starting inside the desktop
session reached Polkit authentication, but no KMS authorization completed within
the startup timeout. The production buffer remains stopped. Native capture,
real-time soak and game frame-time measurements therefore remain unverified.
The temporary KMS socket was moved to Trash with `safe-rm`.

Root `cargo fmt --check` still reports three pre-existing formatting differences
in `capture.rs`, `paths.rs` and `portal.rs`. Those unrelated edits were preserved.
