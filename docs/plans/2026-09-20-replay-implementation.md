# Replay-Clipping: vollständiger Implementierungsplan

## Implementierungsstand nach der Bedienungsentscheidung

Die folgenden Abschnitte bleiben der ursprüngliche Gesamtplan. Implementiert ist
jetzt ein erster Linux-/Hyprland-Pfad mit `start`, `stop`, `status`, `save`, Tray,
**Clip neben REC** für die markierte Region und Vollbild über **Alt+Shift+Print**.
Die Auswahl verwendet den letzten Frame eines beim Öffnen eingefrorenen Clips.
Die Nutzungsdokumentation in [docs/replay.md](../replay.md) beschreibt den
aktuellen Vertrag; weiter unten geplante Befehle sind noch keine verfügbare API.

Bewusste Abweichungen: abstrakte Unix-Sockets mit Same-UID-Prüfung statt
Dateisockets; ein laufender Export ohne Jobverwaltung; CPU-Crop plus
Hardware-Encoding statt des noch fehlerhaften GPU-Crops; Stop beendet auch
unfertige Exporte. Encoderfamilien NVENC/VAAPI/Vulkan haben Adapter, andere
installierte Encoder noch nicht. Echte Capture-, Spielelast- und AMD-/Intel-Tests
sowie Release-Paketierung bleiben offen. Der Gesamtplan ist damit nicht als
vollständig abgenommen zu verstehen.


Stand: 20. September 2026. Status: Umsetzung freigegeben, P0 in Arbeit.

Der Maintainer hat die Umsetzung freigegeben. Der isolierte Medienworker und die ersten synthetischen Vorversuche sind implementiert. Ergebnisse und verbleibende P0-Gates stehen in [docs/replay-benchmarks.md](../replay-benchmarks.md). Eine bestandene Gesamt- oder Performance-Abnahme steht noch aus. Bestehende uncommittete Änderungen bleiben erhalten; keine automatischen Commits oder Veröffentlichungen.

## 1. Verbindliches Verhalten

- Replay zeichnet genau einen gewählten Bildschirm und Systemton auf. Der Bildschirm bleibt für die Sitzung fest gewählt, unabhängig vom Fokus. Kein Mikrofon im Standard.
- Die Videohistorie ist rollierend auf die eingestellten N Sekunden begrenzt, standardmäßig 60 Sekunden. Neue Frames verdrängen die ältesten Abschnitte; die Historie wächst nicht mit der Laufzeit.
- Ein Bind speichert die letzten eingestellten N Sekunden als vollständiges Video.
- Ein zweiter Bind sichert denselben Zeitraum sofort und öffnet danach die Bereichsauswahl auf einem Standbild dieses gesicherten Videos. Der Export enthält tatsächlich nur den gewählten Ausschnitt plus Systemton.
- Auswahl und Export verschieben den gesicherten Zeitraum nicht. Der laufende Replay-Puffer bleibt aktiv.
- Ein normaler Clip kopiert die bereits komprimierten Daten in einen fertigen Container. Ein Bereichsclip muss das Video neu encodieren; dafür läuft keine zweite dauerhafte Aufnahme.
- Vordergrund-Performance hat Vorrang vor Exportgeschwindigkeit. Explizite Auflösung, FPS, Quelle und Encoder werden nie heimlich geändert. Software-Encoding ist eine bewusste Wahl.
- Kein dauerndes Schreiben der Replay-Historie auf SSD. RAM enthält komprimierte Pakete; temporäre Dateien entstehen erst für angenommene Clip-Aufträge.
- Replay startet zunächst explizit. Ein optionaler Autostart setzt eine gültige Quellenwahl voraus.

Der erste vollständige Funktionsumfang gilt für Linux auf den bereits geeigneten Wayland-Compositors. Encoderabdeckung ist keine Zusage zusätzlicher Compositor-Unterstützung. Der Screenshot-Portal-Fallback ist kein fertiger Screencast-Pfad. Windows bleibt eingefroren; X11-, Portal-/PipeWire-Capture und zusätzliche Plattformen werden nicht beiläufig ergänzt.

## 2. Architektur und Verantwortlichkeiten

```text
CLI / zwei Desktop-Binds
          |
Linux-Daemon: Replay-Supervisor, Status, Selector, Shelf
          | private Steuerverbindung
          v
boltsnap-replay-worker
          |
Capture-/Encode-Adapter: gewählter Output + Systemton
          | komprimierter Stream, begrenzte Pipe
          v
libavformat-Demux -> begrenzter Paketring -> unveränderlicher Snapshot
                                               |              |
                                          Remux-Export    Standbild
                                                              |
                                                         Bereichswahl
                                                              |
                                                   begrenzter Crop-Export
```

Der Linux-Daemon besitzt einen kleinen Supervisor. Der Medienworker besitzt Recorder-Kindprozess, Demuxer, Ring und Aufträge. Blockierende Medienarbeit läuft weder im Wayland-Eventloop noch im Socket-Reader. Ingest und Steuerung bleiben bedienbar, während Exporte laufen.

### Prozess- und Abhängigkeitsgrenze

- Separates Rust-Paket unter `src/platform/linux/replay/worker/`, eigenes Manifest und Lockfile, Binärname `boltsnap-replay-worker`. Kein Umbau des gesamten Projekts zum Workspace nötig.
- Nur dieses Paket verlinkt libavformat/libavcodec/libavutil und benötigte Filterbibliotheken. Ausgangspunkt ist `ffmpeg-next` mit eng gekapselten zusätzlichen FFI-Aufrufen, wo dessen API nicht reicht. Die im Vorversuch geprüfte Version wird festgesetzt.
- Kleine reine Datentypen und Policies unter `src/replay/` werden von Hauptprogramm und Worker als Quellmodule eingebunden. Keine Abhängigkeit des Workers auf das gesamte Boltsnap-Paket und keine nativen FFmpeg-Typen im gemeinsamen Vertrag.
- Hauptprogramm startet ohne Worker, FFmpeg und Capture-Tool weiter für Screenshots. Fehlende Replay-Komponenten ergeben eine gezielte Diagnose erst bei Replay-Nutzung.
- Worker und CLI sprechen ein versioniertes Protokoll. Ein inkompatibler Worker wird vor Aufnahmebeginn abgelehnt.
- Für die Distribution getrenntes optionales Worker-Paket mit ermittelten Bibliotheksabhängigkeiten. Ein einzelner dynamischer FFmpeg-Build wird nicht als distributionsübergreifend ABI-kompatibel ausgegeben. Keine automatischen Treiber-/Codec-Downloads.

### Capture-Adapter

1. GPU Screen Recorder als zuerst zu vermessender GPU-Pfad, ohne seinen internen Replay-Modus. Wiederverwendung von Capture und Encoding, eigene Kontrolle der Historie.
2. wf-recorder als Adapter mit frei wählbarem kompatiblem FFmpeg-Encoder. Eigene Replay-Profile; bestehendes Recording mit `--no-dmabuf` und dessen aktuellem FPS-Verhalten bleibt unverändert.
3. wl-screenrec ersetzt bei nachgewiesenem Vorteil einen VA-API-Pfad. Kein dritter dauerhafter Adapter allein wegen seiner Verfügbarkeit.
4. Falls eine geforderte Encoderfamilie keine passenden Frames von diesen Tools bekommt: enger nativer Capture-/Frame-Import-Adapter im Worker, der dieselbe Ring-Schnittstelle nutzt. Kein zweiter vorgeschalteter Encode. Erst nach dokumentiertem Fehlschlag der vorhandenen Wege bauen.

Interner Transport zuerst NUT mit deaktiviertem Indexaufbau und begrenztem Flush-Verhalten prüfen, danach Matroska mit begrenzter Cluster-Latenz. Endgültige Wahl in P0 anhand tatsächlicher Metadaten-, Speicher- und Latenztests. Keine eigene Containerimplementierung, kein Polling einer unfertigen normalen MP4-Datei. Komprimierte Pipe-Kopien und Mux/Demux sind messbarer Aufwand, kein behauptetes vollständiges Zero-Copy.

### Verbindliche Regeln für die Codearchitektur

Die Dateiliste ist keine Aufforderung, vorab ein Framework zu bauen. Zuerst einen vollständigen vertikalen Pfad umsetzen: Start, begrenzte Historie, Snapshot, gültiger Vollbildclip. Weitere Adapter und Crop ergänzen diesen Pfad. Neue Traits oder generische Infrastruktur brauchen mindestens zwei konkrete Nutzer mit tatsächlich gemeinsamer Semantik. Statische Adapterauswahl mit Enum und kleinen Funktionen reicht zunächst; kein Plugin-System, kein allgemeiner Eventbus und keine neue Async-Runtime allein für Replay.

**Ownership und Nebenläufigkeit:**

- Genau ein Controller im Worker verändert Sitzungszustand, Ring und RAM-Reservierungen. UI und Supervisor spiegeln bestätigte Zustände und erfinden keine zweite Aufnahme-Wahrheit.
- Ein separater Ingest-Thread besitzt den Demux-Kontext und dessen blockierende Eingabe. Er übergibt Pakete über eine nach Bytes und Anzahl begrenzte Queue. Der Controller verarbeitet begrenzte Batches und bedient dazwischen Stop-/Snapshot-Anfragen. Ein hängender Demuxer darf Steuerung nicht aushungern.
- Export erhält ausschließlich einen unveränderlichen Snapshot und eine eigene Reservierung. Ein Export-Thread besitzt seine Decode-/Filter-/Mux-Kontexte. Keine FFmpeg-Kontexte threadübergreifend gleichzeitig benutzen und keine ungeprüften `unsafe impl Send/Sync` ergänzen.
- Keine Datei-I/O, GPU-Wartezeit, Prozesswartezeit oder Standbild-Decodierung unter einer Ring-/UI-Sperre. Kein globales `Arc<Mutex<ReplayEverything>>`. Gemeinsam referenzierte Paketdaten bleiben unveränderlich; ihre Lebensdauer ist explizit.
- Jede Queue hat Kapazität, Besitzer und dokumentiertes Verhalten bei Rückstau/Disconnect. Kontrollnachrichten dürfen nicht hinter unbegrenzt vielen Medienpaketen warten. Ein gemeinsames Wakeup/Wait-Verfahren vermeidet Busy-Polling und Sleep-Schleifen.
- Worker kontrolliert Paket-RAM, Supervisor koordiniert temporäre Plattenreservierungen mit den bestehenden Aufnahmegrenzen. Keine mehrfach unabhängig vergebenen Reservierungen für denselben freien Platz. Ressourcen werden genau einmal bei Terminalzustand beziehungsweise bestätigtem Besitzwechsel freigegeben.

**Schnittstellen und Abhängigkeiten:**

- Abhängigkeitsrichtung: CLI/UI -> Plattform-Fassade -> Supervisor/Medienworker -> Capture-/Codec-Adapter. Reine Verträge und Berechnungen liegen darunter und importieren keine dieser Implementierungen zurück.
- Ein kleiner gemeinsamer Moduleinstieg bindet die tatsächlich geteilten Quellen ein. Keine kopierten Typdefinitionen, keine verstreuten relativen `#[path]`-Importe und keine implizite Abhängigkeit auf `crate::config` oder Shelf aus den gemeinsamen Modulen. CLI-spezifischer Code wird nicht in den Worker hineingezogen.
- Adapter beschreiben Capture-Start und nachgewiesene Fähigkeiten. Clipfenster, Quotas, Jobzustände und Dateifertigstellung existieren jeweils nur einmal im gemeinsamen Ablauf. Kein eigener Replay-Ring je Hersteller.
- Wire-Nachrichten, interne Zustände und UI-Anzeige bleiben getrennt. Wire-Strings werden an der Grenze validiert, intern tragen Enums und Typen die Bedeutung. Fehler behalten Code, betroffene Stufe und Ursache; Text entsteht an der Anzeigegrenze.
- Native Handles/Pointer bleiben in kleinen RAII-Wrappers. Jeder notwendige `unsafe`-Block dokumentiert Ownership, Lebensdauer und die konkret eingehaltene FFmpeg-Vorbedingung. `Drop` räumt nichtblockierende Ressourcen auf; begrenztes Prozess-/Thread-Shutdown erfolgt explizit.
- Codekommentare sind kurz, sachlich und beschreibend. Nur nicht offensichtliche Bedingungen, Einheiten, Ownership oder notwendige Sicherheitsinvarianten kommentieren. Keine Nacherzählung des Codes, Floskeln, dekorativen Überschriften oder Implementierungstagebücher. Ausführliche Architekturbegründungen gehören in die Dokumentation.

**Kosten im laufenden Betrieb:**

- Pro Paket nur erforderliche Referenz-/Metadatenarbeit. Keine JSON-Erzeugung, Prozessabfragen, Dateisystemscans, neuen Threads oder Logs pro Frame/Paket.
- Ring organisiert abgeschlossene Abschnitte und laufende Summen. Kein vollständiges Durchlaufen der Historie bei jedem eingehenden Paket. Snapshot-Erzeugung referenziert vorhandene Abschnitte; sie kopiert keine Videonutzdaten und hält den Controller nicht für Disk-Export fest.
- Status liest akkumulierte Zähler mit begrenzter Aktualisierungsrate. Wiederholte Fehler werden zusammengefasst; Logqueues und Logs dürfen ebenfalls nicht unbeschränkt wachsen.
- Optimierungen folgen Messungen. Kein eigener Allocator, lockfreier Container, Shader oder Containerparser ohne belegte Notwendigkeit. Bewährte Standardbibliothek und vorhandene native APIs zuerst.

**Review vor Abschluss jedes Arbeitspakets:** Ist eindeutig, wer jede Ressource besitzt? Kann langsame Disk, ein hängender Encoder oder eine verschwundene UI den Capture-/Kontrollpfad blockieren? Gibt es doppelte Zustands-/Budgetlogik oder eine Abstraktion ohne aktuellen Nutzer? Lässt sich ein Fehlerpfad mit einem kleinen deterministischen Test prüfen? Ein Paket mit ungeklärter Antwort ist architektonisch noch nicht fertig.

## 3. Konkrete Dateigrenzen

Die Namen sind die geplanten Implementierungsorte. Kleine eng zusammengehörige Module dürfen zusammengelegt werden; die Verantwortungsgrenzen bleiben bestehen.

| Datei / Bereich | Änderung und Verantwortung |
| --- | --- |
| `src/main.rs` | Replay-Befehle früh an einen eigenen typisierten Parser delegieren; vorhandene Screenshot-/Recording-Argumente erhalten |
| `src/replay/mod.rs`, `cli.rs`, `types.rs` | Portable Befehle, IDs, Status, Konfiguration und Fehlercodes |
| `src/replay/protocol.rs` | Versionierte Replay-Nachrichten und begrenztes Framing, ohne native Handles |
| `src/replay/policy.rs`, `geometry.rs` | Zeitfenster, Reservierungen, Queue-Regeln, Pixel-/Output-Transformationen |
| `src/lib.rs` | Reine Replay-Module für plattformneutrale Tests zugänglich machen |
| `src/config.rs` | Validierte `[replay]`-Sektion, bestehende Schlüssel und unbekannte TOML-Werte bewahren |
| `src/platform/mod.rs` | Schmale Replay-Plattform-API; klare Nichtverfügbarkeit auf anderen Plattformen |
| `src/platform/linux/replay/mod.rs`, `supervisor.rs`, `ipc.rs` | Worker-Lebenszyklus, eigener Replay-Socket, sichere Prozessaufrufe, Eventweitergabe |
| `src/platform/linux/replay/worker/Cargo.toml`, `Cargo.lock`, `src/main.rs` | Eigenständig baubarer Medienworker und Capability-Handshake |
| Worker `adapter.rs`, `capabilities.rs`, `profiles.rs` | Tool-/Geräteerkennung, Encoderwahl, konkrete Capture-Kommandos und Probe-Ergebnisse |
| Worker `media.rs`, `ring.rs`, `snapshot.rs` | Native Paketverwaltung, Demux, Schlüsselbildgrenzen, reservierte Snapshots |
| Worker `export.rs`, `crop.rs`, `audio.rs` | Remux, GPU-/CPU-Crop, gemeinsamer A/V-Zeitbezug und Quellenwechsel |
| `src/platform/linux/paths.rs` | Private Replay-Verzeichnisse, Worker-Auflösung, passende Container-Endungen |
| `src/platform/linux/select_skia/mod.rs` | Vorgegebenes Videostandbild und aufgezeichneten Output als Auswahlquelle akzeptieren |
| `src/platform/linux/shelf/mod.rs` | Supervisor anbinden, kleine Replay-Events, fertige Clips hinzufügen |
| `src/platform/linux/tray.rs` und zuständiges Statusmodell | Start/Stop, Zustand, Fehler, laufende Exportaufträge |
| `tests/` und Worker-Tests | Portable Policy-Tests, Medienfixtures, Prozess- und Fehlerfalltests |
| `docs/replay.md`, `docs/replay-benchmarks.md` | Nutzung, Bind-Beispiele, Grenzen, reproduzierbare Messergebnisse |
| `.github/workflows/ci.yml`, `.github/workflows/release.yml`, Paketmetadaten | Separater Linux-Worker-Build, Bibliotheksmatrix, optionale Auslieferung |

Vor jeder Codeänderung den gesamten betroffenen Aufrufpfad erneut lesen. Besonders `capture.rs`, `paths.rs`, Linux-Shelf, Cargo-Dateien und README haben bereits fremde Änderungen im Arbeitsbaum.

### Integration ohne Veränderung des Windows-Backends

Das vorhandene `Request`-Enum in `src/protocol.rs` wird auch im Windows-Shelf vollständig gematcht. Deshalb bleibt dieses Legacy-Protokoll unverändert. Replay erhält eigene portable Nachrichten und unter Linux den Socket `boltsnap-replay.sock` im plattformseitig ermittelten Runtime-Verzeichnis. Der vorhandene Daemon-Startmechanismus wird wiederverwendet.

Der zusätzliche Socket ist ein Kontrollkanal, kein Videotransport. Er gehört demselben Benutzer, liegt in einem privaten Verzeichnis und hat begrenzte Nachrichtengrößen und Leserzeiten. Worker-Steuerung erfolgt über eine geerbte private Verbindung. Keine beliebigen von Clients gelieferten Prozess-IDs signalisieren. Status-Watcher haben begrenzte Queues und dürfen Ingest nicht blockieren.

Das Hauptprogramm dispatcht Replay über die schmale Plattform-API. Ein kleiner Nicht-Linux-Fehlerpfad liegt an dieser Grenze; keine neuen Implementierungen unter `src/platform/windows/`.

## 4. Bedienung und Konfiguration

Geplante CLI:

```text
boltsnap replay start [--output NAME]
boltsnap replay stop
boltsnap replay status [--json]
boltsnap replay watch --json
boltsnap replay save [--seconds N] [--destination shelf|disk]
boltsnap replay crop [--seconds N] [--destination shelf|disk]
boltsnap replay jobs [--json]
boltsnap replay retry JOB_ID
boltsnap replay cancel JOB_ID
boltsnap replay encoders [--json]
boltsnap replay doctor [--probe]
```

Desktop-Binds führen `replay save` beziehungsweise `replay crop` aus. Keine neue globale Hotkey-Bibliothek erforderlich. Ohne laufenden Puffer geben Save/Crop einen eindeutigen Fehler zurück; sie starten keine Aufnahme, deren Vergangenheit ohnehin fehlt.

`start` ist für eine identische aktive Sitzung idempotent. Änderungen an Quelle/Codec/FPS erfordern einen kontrollierten Neustart mit neuem Sitzungsschlüssel; kein Wechsel mitten in einem Snapshot. `stop` beendet neue Aufnahme und verwirft nur die ungesicherte Historie. Bereits angenommene Aufträge bleiben gültig und können fertig werden oder einzeln abgebrochen werden.

Normales Speichern bestätigt nach Sicherung des Snapshots die Job-ID. Fertigstellung und Zielpfad kommen über Status/Watch. Crop sichert zuerst den Snapshot, wartet dann auf das Standbild und öffnet den Selector. Escape verwirft nur diesen Auftrag. Pro Sitzung maximal eine offene Auswahl; ein zweiter Crop-Aufruf meldet `selection_busy` ohne neue Reservierung.

Geplante Konfiguration, Ausgangswerte für die Messphase:

```toml
[replay]
autostart = false
duration_seconds = 60
output = ""                 # vor Start explizit wählen
fps = 60
resolution = "native"
encoder = "auto"
codec = "auto"
quality = "balanced"
container = "auto"
audio = "system"
memory_mib = 512
temp_quota_mib = 2048
max_jobs = 2
export_concurrency = 1
destination = "shelf"
```

- `duration_seconds`: 1 bis 600, pro Save höchstens die konfigurierte Historie. Übliche Binds können 30 oder 60 Sekunden anfordern.
- `fps`: positive ganze Zahl, initial 1 bis 240; zusätzlich reale Geräte-/Auflösungsgrenzen prüfen. 120/144/240 sind explizite Profile, kein pauschales 240-FPS-Dauerduplizieren eines ruhigen Desktops.
- `memory_mib`: 64 bis 4096 mit zusätzlicher Plausibilitätsprüfung gegen verfügbaren RAM. 512 MiB ist ein Startwert für den lokalen 16-GiB-Rechner, keine Garantie für 60 Sekunden bei jeder Qualität.
- `max_jobs`: 1 bis 8, initial 2 inklusive Auswahl/Queue/Export. Reservierungsprüfung kann schon unter dieser Anzahl ablehnen. Crop-Concurrency bleibt zunächst auf 1 begrenzt.
- Native Auflösung standardmäßig; abweichende Größe explizit. Qualität als benanntes Profil oder encoderspezifische Optionen, ohne erfundene universelle CQ-Skala.
- Optionale `device`, `backend`, `encoder_options` und `export_encoder` werden typisiert/validiert. Prozessaufrufe mit Argumentlisten, nie Shell-Interpolation. Optionen dürfen Quelle, Ausgabepfad und Steuerkanal nicht überschreiben.
- Unbekannte Replay-Schlüssel und ungültige explizite Werte führen bei Replay-Nutzung zu einer konkreten Diagnose. Sie machen vorhandene Screenshots nicht unbenutzbar.
- Standardziel Shelf nutzt die vorhandene temporäre Video-Karte mit späterem Speichern. `disk` übernimmt atomar in das konfigurierte Aufnahmeverzeichnis. Dateiendung folgt dem wirklichen Container.

## 5. Encoderwahl, Qualität und Dateigröße

Erkennung erfasst **Backend-Build + Encoder + Gerät + Frameformat + Codec + Container**. Die Ausgabe von irgendeinem `ffmpeg -encoders` reicht nicht, um die Fähigkeiten eines separat gebauten Capture-Tools zu behaupten.

| Familie | Geplanter Aufnahmeweg | Geplanter echter Crop-Weg |
| --- | --- | --- |
| NVIDIA / NVENC | Geprüfter GSR-Pfad; weitere Namen über kompatiblen Adapter | Hardwaredecode und unterstützter CUVID-Crop oder geprüfter GPU-Filter, anschließend NVENC |
| AMD / VA-API | Geprüfter DMA-BUF-/VA-API-Pfad | VA-API-Decode, Crop-Rechteck durch VPP materialisieren, VA-API-Encode |
| Intel / VA-API, QSV | VA-API zunächst; QSV mit nachgewiesenem Geräte-/Frame-Import | VA-API-VPP beziehungsweise `vpp_qsv`, danach passender Encoder |
| AMF | Wenn Linux-Build, Treiber und Frameversorgung die Kombination tatsächlich unterstützen | Geprüfter Hardwarepfad oder offengelegter CPU-Filter mit Hardware-Upload |
| Vulkan Video | Nur mit erfolgreich initialisierter Encode-Queue und benötigten Erweiterungen | Nachgewiesener Decode-/Filter-/Encode-Pfad; kein aus dem Namen abgeleitetes Interop-Versprechen |
| Weitere Hardware-/Softwareencoder | Freie Encoder-Namen mit passender Frameversorgung und bestandener Probe | Kompatibler Exportencoder; Softwarepfad explizit und gedrosselt |

Keine Namens-Allowlist als allgemeine Zulassungsgrenze. Bekannte Familien bekommen abgestimmte Profile, unbekannte Encoder ihre tatsächlichen Optionen. Ein installierter Encoder kann wegen fehlender Hardware, inkompatibler Frames oder Container ungeeignet sein; der Status nennt diese konkrete Stufe.

Prüfstufen: gefunden, initialisierbar, Format/FPS/Auflösung unterstützt, kurzer synthetischer Encode, tatsächlicher Capture-Pfad, decodierbarer Remux, Crop. Lange Belastungstests laufen nur im Diagnose-/Abnahmeverfahren. Ergebnisse werden nach Gerät, Treiber, Tool-/Bibliotheksversion und Aufnahmeparametern gecacht.

Auto bevorzugt getestete Hardware auf dem Capture-Gerät. AV1 wird bei geeigneter Hardware und bestandenen Qualitäts-/Lasttests bevorzugt, danach HEVC beziehungsweise H.264 entsprechend Kompatibilitätsprofil. Die Reihenfolge ist kein Ersatz für Messungen. Ein manuell angeforderter Encoder wird nicht still ersetzt. Andere GPU, CPU-Readback oder Software-Fallback werden vor Start als gewählter Pfad ausgewiesen.

Profile optimieren Qualität und Dateigröße innerhalb des Performance-Budgets. Keine pauschale Bitrate für alle Auflösungen oder identischen Qualitätszahlen zwischen Encodern. Keyframe-Abstand zunächst höchstens 1 Sekunde; B-Frames zunächst deaktiviert, sofern der Encoder das erlaubt. Kompressionsverlust dieser Latenzentscheidung wird vermessen. Komplexere Referenzstrukturen kommen erst nach vollständigem Nachweis korrekter Clip-Grenzen.

MP4 für nachgewiesen kompatible Kombinationen, sonst Matroska. Audiocodec passend zum Container, initial AAC bei Verfügbarkeit. Audio wird beim Export kopiert, soweit die Zeitgrenzen und Container es erlauben. Crop erzeugt genau eine zusätzliche Video-Generation. Optionales Nachkomprimieren kompletter Clips ist kein Bestandteil des Standardpfads.

## 6. Ring, Zeitpunkt und Speichergarantie

### Ring und Budget

- Ein Encode pro Video-/Audiostream. Der Ring hält refcounted komprimierte Pakete, Side-Data, Streamparameter und rationale Zeitbasen. Keine Rohbildhistorie.
- Paketdaten werden nur einmal gezählt, solange Snapshots und Ring dieselben Referenzen teilen. Für späteres Überaltern reserviert ein angenommener Snapshot seine künftige zusätzliche Belegung.
- Budget umfasst Ring, noch lebende Snapshot-Daten, Side-Data, Verwaltungsaufwand und begrenzte Eingangs-/Exportqueues. Decoderflächen, Encoderpuffer und Prozess-RSS werden separat gemessen und begrenzt, nicht fälschlich als Teil einer exakten Paketbyte-Garantie ausgegeben.
- Ganze sicher decodierbare Abschnitte entfernen. Niemals einzelne benötigte Referenzpakete herauswerfen. Keyframe-Flag allein reicht bei offenen GOPs nicht als Beweis; unterstützte Profile und Decode-Tests legen sichere Schnittpunkte fest.
- Sobald ein Abschnitt vor `Ende - N` beginnt, wird er entfernt. Überschreitet schon die aktive GOP das Zeitfenster, verfällt sie samt Audio; bis zum nächsten sicheren Einstiegspunkt gibt es keine speicherbare Historie.
- Ist eine GOP oder ein Paket größer als das zulässige Budget, kontrollierter Fehler statt unbegrenzter Sonderallokation. Wiederholter Rückstau macht die Sitzung fehlerhaft; keine beliebigen Paketverluste kaschieren.
- Historie darf bei Byte-Druck kürzer als N werden und meldet dies. Auflösung, FPS und Qualität werden dadurch nicht still angepasst.
- Vor Snapshot-Annahme RAM und temporären Speicher reservieren. Initial konservativ genug Reserve für historischen Snapshot plus weiterlaufenden Ring vorhalten; genaue gemeinsame Referenzen dürfen später nachgewiesen effizienter reserviert werden.
- Angenommene Crop-Snapshots zügig asynchron in eine begrenzte private Quelldatei remuxen. Bei langsamer/voller Disk bleiben zugesagte Daten erhalten, neue Aufträge werden abgelehnt und Live-Historie wird nötigenfalls kürzer. Kein unbegrenztes Queue-Wachstum.
- Quota für temporäre Quellen, unfertige Exporte und fertige temporäre Replay-Karten gemeinsam abrechnen. Bestehende Recording-Quota und freien Plattenplatz zusätzlich berücksichtigen; zwei unabhängige Prüfungen dürfen denselben freien Platz nicht doppelt reservieren.

Speicherbedarf als Größenordnung: 40 Mbit/s benötigen für 60 Sekunden ungefähr 300 MB reine Videopakete; Audio, Verwaltungsaufwand und Aufträge kommen hinzu. Maximale FPS, höchste Qualität, kleinste Datei und beliebig kleiner RAM sind keine gleichzeitig garantierbaren Größen.

### Fester historischer Endpunkt

1. CLI meldet den Auftrag; der Worker verarbeitet ihn im geordneten Kontrollpfad.
2. Worker fixiert den neuesten bereits vollständig verfügbaren, sicher decodierbaren Video-Endpunkt seiner aktuellen Sitzung. Keine später eintreffenden Frames an diesen Auftrag anhängen.
3. Start ist der erste verfügbare sichere Random-Access-Punkt bei oder nach `Ende - N`. Das Videofenster überschreitet N nicht; der Schlüsselbild-Abstand kann den Clip verkürzen. Die tatsächliche Dauer steht im Ergebnis. Exakt framegenaue Starts würden zusätzliches Encoding erfordern und sind kein stilles Verhalten.
4. In der Aufwärmphase wird die verfügbare kürzere Dauer gespeichert und ausgewiesen. Ohne decodierbaren Startpunkt noch kein erfolgreicher Clip.
5. Snapshot erhält `session_id`, `snapshot_id`, Anfang/Ende, Output-Geometrie und Codec-Metadaten. Erst nach erfolgreicher Reservierung und Sicherung folgt die Annahmebestätigung.

Der Endpunkt ist wegen Capture-/Encode-/Mux-Verzögerung nicht exakt der physische Tastendruck. Diese Verzögerung wird gemessen und im Diagnosemodus sichtbar. Eingangszeit an der Pipe darf nicht als bewiesene Capture-Zeit ausgegeben werden. Bei später erlaubten B-Frames muss die Endgrenze alle Decode-Abhängigkeiten erfüllen; keine referenzlosen Endframes exportieren.

Audio und Video verwenden einen gemeinsamen Zeitursprung, ihre rationalen Zeitbasen und erhaltene Codec-Priming-/Skip-Metadaten. Kein unabhängiges Nullsetzen beider Streams. Audio-Paketgrenzen können einen kleinen dokumentierten Überhang erzeugen. Clock-Drift wird in der laufenden Audiopipeline kontrolliert ausgeglichen, ohne das Clipfenster nachträglich zu verschieben.

Ruhiger Desktop/VFR braucht gültige Frame-Dauern, periodisch sichere Einstiegspunkte und einen definierten aktuellen Endpunkt. Unterdrückte identische Frames sind erlaubt, eingefrorene Zeitachsen und jahrelange GOPs nicht. Messung entscheidet, ob ein unterstützter Adapter das zuverlässig liefert.

## 7. Bereichsauswahl und Export

- Aus dem fixierten Snapshot den letzten darstellbaren Frame decodieren. Nur dieses einzelne Standbild zur UI übertragen, mit vorab geprüfter Größenbegrenzung und Quelldimensionen.
- `run_select_record` beziehungsweise aktuelle Screenshot-Auswahl nicht als Live-Aufnahmeersatz verwenden: deren Fokus-Output und neue Aufnahme können inzwischen anders sein.
- Neue Selector-Eingabe enthält explizit Standbild, aufgezeichneten Output, Rotation, Skalierung und Pixelgeometrie. Ergebnis ist ein überprüftes Rechteck in Videopixeln.
- Negative Monitorpositionen, fractional scaling, Portrait/Rotation und Letterboxing rein mathematisch abbilden und testen. Aktuelle Output-Geometrie nie rückwirkend auf ältere Frames anwenden.
- Chroma-/Encoderalignment: nach innen ausrichten und das tatsächliche Rechteck sichtbar darstellen. Falls Padding notwendig ist, nur Randpixel des gewählten Bereichs oder Schwarz verwenden; keine Pixel außerhalb der Auswahl dazunehmen. Zu kleine Rechtecke klar ablehnen.
- Vollbildauswahl nutzt den normalen Remuxpfad. Andere Rechtecke durch Decode, Crop und Encode materialisieren. Keine reine Container-Crop-Metadatenlösung, bei der der vollständige Desktop im Video erhalten bleibt.
- Audio des gleichen Snapshots bleibt erhalten. Ausgabeauflösung ist die Ausschnittgröße, mit offen ausgewiesenem notwendigem Padding.
- Vor Export Pfadfähigkeit und Sitzung prüfen. Falls der Aufnahmeencoder keinen passenden Crop-Pfad hat, den eingestellten kompatiblen Exportencoder verwenden. Kein stiller CPU-Wechsel unter Last.
- Ein Export gleichzeitig, niedrige CPU-/I/O-Priorität und zusätzlich begrenzte GPU-Zulieferung. `nice` allein begrenzt keine GPU-Auslastung. Frameweise pacing, kleine Hardwarequeues und Lastpausen vorsehen; sichere Mindestfunktion ohne herstellerspezifische GPU-Telemetrie.
- Voll ausgelastetes Spiel darf Crop-Export verzögern. Gesicherte Quelle bleibt vorhanden, Status zeigt `waiting_for_budget`. Kein Versprechen sofortiger Fertigstellung bei unsichtbarer Zusatzlast.
- Ausgabe zuerst in exklusive temporäre Zieldatei. Mux-/Flush-/Close-Fehler prüfen, Metadaten und repräsentative Decode-Punkte validieren, anschließend ohne Überschreiben übernehmen. Bei Ziel auf anderem Dateisystem dort temporär kopieren und erst dann veröffentlichen.
- Shelf erhält ausschließlich erfolgreich fertiggestellte Clips. Fehlgeschlagene Exporte behalten eine wiederholbar nutzbare Quelle innerhalb der Auftragsquote; Retry oder Cancel geben Ressourcen kontrolliert frei.

Der aktuelle Clip enthält die später geöffnete Auswahl nicht. Da die Bildschirmaufnahme weiterläuft, können spätere Clips die Auswahloberfläche enthalten. Universelles Ausschließen eigener Overlays ist keine gesicherte Eigenschaft der Capture-Backends und darf nicht durch heimliches Pausieren mit Zeitlücken simuliert werden.

## 8. Zustände, Protokoll und Fehlerfälle

Sitzungs-Lebenszyklus: `stopped -> starting -> buffering -> ready`, zusätzlich `stopping`, `failed`. `ready` bedeutet decodierbare Historie vorhanden; `available_duration_ms` zeigt separat, ob die gewünschte Dauer erreicht ist. Gesundheit wird orthogonal als `healthy` oder `degraded` mit konkreten Gründen geführt: Eine bereite Sitzung kann zugleich Tonprobleme haben. Verkürztes Fenster, fehlender Ton oder überschrittenes FPS-Ziel sind sichtbar. Keine widersprüchlichen Bool-Kombinationen für Lebenszykluszustände.

Auftrag: `accepted -> securing -> awaiting_selection/queued -> exporting -> completed`, alternativ `waiting_for_budget`, `failed`, `cancelled`. Bei `accepted` sind Snapshot und RAM-Reserve bereits gesichert; `securing` bezeichnet die anschließende Dateisicherung. Ein Auftrag bleibt unabhängig von einem späteren Sitzungswechsel. Fehler darf der UI-Status nicht als Erfolg kaschieren. Zustandswechsel erfolgen zentral als explizite Übergänge, inklusive Freigaben und genau einem Terminalereignis pro Versuch. Retry beginnt nur mit validierter erhaltener Quelle und neu reservierten Ressourcen einen neuen Versuch derselben Job-ID.

Gemeinsame Nachrichten: `Start`, `Stop`, `Status`, `Watch`, `Save`, `PrepareCrop`, `CommitCrop`, `RetryJob`, `CancelJob`, `ListJobs`, `Capabilities`. Jede Mutation hat eine Request-ID; Wiederholung nach Transportfehler gibt denselben Auftrag zurück statt doppeltem Clip. Session-/Snapshot-IDs verhindern verspätete Rechtecke für falsche Aufnahmen. Auch Deduplizierung und abgeschlossene Jobhistorie haben feste Grenzen. Deduplizierung gilt innerhalb der ausgehandelten Worker-Instanz und ihres ausgewiesenen Wiederholungsfensters. Nach Neustart oder Ablauf gibt es keine automatische Wiederholung einer ungewissen Mutation; zuerst Job-/Recovery-Status abgleichen.

Status enthält tatsächlichen Encoder, Codec, Adapter, Gerät, Ausgabeformat, FPS, Quellgröße, Historienlänge, RAM-Belegung/Reservierung, Disk-Belegung/Reservierung, Aufträge, verworfene Frames und letzte konkrete Fehlerursache. Diagnose unterscheidet angefragte und beobachtete Werte. Keine dauernden Vorschau-Decodes zur Statusanzeige.

Antworten bestätigen innerhalb der vorhandenen Größenordnung des IPC-Timeouts nur Annahme/Zustand. Export läuft asynchron. Watch koalesziert Statusänderungen, behält Terminalereignisse oder erzwingt bei Rückstau einen vollständigen Resync.

| Ereignis | Festgelegtes Verhalten |
| --- | --- |
| Worker-/Recorder-Absturz | Daemon/Shelf bleiben aktiv; Sitzung fehlerhaft, offene Aufträge nach vorhandener Quelldatei als rettbar oder verloren kennzeichnen |
| Startfehler / fehlender Encoder | Konkrete Capability-Stufe melden, kein automatisches Software-Encoding |
| Mehrfacher Start / zwei Clients | Ein Supervisor-Owner, idempotente Startentscheidung und sichere Socket-Besitzprüfung |
| Monitor entfernt, Größe/Rotation geändert | Aufnahme kontrolliert beenden, neue Session nötig; alte Snapshots behalten alte Geometrie |
| Audio-Ausgabegerät gewechselt | System-Default verfolgen; Streamwechsel kontrolliert mit neuem Zeitsegment oder neuer Session, keine Mikrofonquelle einsetzen |
| Audio fehlt / Adapter kann Wechsel nicht nahtlos | Sichtbar `degraded` oder kontrollierter Neustart; keine behauptete lückenlose Tonspur |
| Suspend oder Session-Lock | Über bestehende System-/Session-Signale stoppen/pausieren, danach neue Zeit-Epoche; keine Lock-Screen-Aufnahme |
| Daemon-Ende | Worker und eigene Kinder begrenzt herunterfahren und reap; keine verwaisten Recorder |
| Speicher-/Diskquote erreicht | Neue Jobs ablehnen, keine unbegrenzte Allokation und keine still verlorenen angenommenen Clips |
| Job abgebrochen | Nur dessen Ressourcen freigeben, Replay und andere Aufträge weiterlaufen lassen |
| Alte Temporärdateien nach Neustart | Besitz/Manifest prüfen, Wiederherstellung anbieten; keine fremden Dateien oder gespeicherten Clips bereinigen |
| Bestehende normale Aufnahme aktiv | Beide Zustände getrennt halten; Hardware-Sessionlimit vor Replay-Start prüfen, bei Konflikt Replay klar ablehnen |
| HDR-/Farbraum nicht durchgängig unterstützt | Vor Start klar ablehnen oder explizites SDR-Profil anbieten; keine falschen Farben still als HDR speichern |

Session-Lock-Tests verwenden simulierte Signale beziehungsweise vom Nutzer selbst durchgeführte Abläufe. Hyprlock niemals durch Agenten starten, aufrufen, signalisieren, schließen oder abfotografieren.

## 9. Arbeitspakete und Reihenfolge

Jedes Paket endet mit prüfbaren Ergebnissen. Kein nächstes Paket darf fehlende Nachweise seines Vorgängers durch Annahmen ersetzen. Einzelne Hardwarevarianten dürfen als ungetestet ausgewiesen bleiben; vollständige Herstellerabdeckung ist damit noch nicht abgenommen.

### P0: technische Nachweise und festes Build-Fundament

**Dateien:** Worker-Grundgerüst, reproduzierbarer Diagnose-Harness, `docs/replay-benchmarks.md`. **Abhängigkeit:** Implementierungs-Go.

- [ ] Aktuelle Plattform/Versionen und vorhandene Tools erfassen; Capture-Fähigkeiten des Compositors prüfen.
- [ ] Isolierten Worker mit festgesetzter Rust-FFmpeg-Anbindung bauen. Hauptbinary ohne Medienbibliotheken starten. Baseline mit Ubuntu-22.04-FFmpeg und aktueller FFmpeg-Version prüfen; neuere Features capability-gaten.
- [ ] GSR-/wf-recorder-Transport inklusive Systemton und HDR-/Codec-Metadaten testen. NUT zuerst, Matroska nur bei konkretem Hindernis; Flush, Startup-Probe, EOF und Mehrstunden-Speicherwachstum messen.
- [ ] Aufnahme ohne CPU-Rohbildtransfer auf lokalem GPU-Pfad nachweisen; versteckte GPU-/CPU-/Cross-GPU-Kopien erfassen.
- [ ] Capture allein, toolinternes Replay und Capture mit eigenem Demux/Ring im wiederholten A/B-Test vergleichen.
- [ ] Ein historisches Vollbildvideo und einen echten Hardware-Crop mit synchronem Testton erzeugen und decodieren.
- [ ] Ergebnis festhalten: genaues Backend-Kommando, Bibliotheksversionen, Transport, unterstützte Profile, Last und Restprobleme.

**Gate:** Bei relevantem Zusatzaufwand, unbeschränkten Transportpuffern oder unbrauchbarer Endpunktlatenz erst den Transport/Adapter korrigieren. Falls nur native Integration die Ziele erreicht, diese begrenzt nachweisen, bevor große UI-Arbeit beginnt. Keine Freigabe der Architektur allein aufgrund eines abspielbaren Einzelclips.

### P1: portable Verträge, Konfiguration und CLI

**Dateien:** `src/replay/*`, `src/config.rs`, `src/main.rs`, `src/lib.rs`, Plattformgrenze. **Abhängigkeit:** P0-Vertrag.

- [ ] Typisierte Commands, Session-/Job-IDs, versionierte Nachrichten, Zeit-/Byte-Einheiten und Fehlercodes definieren.
- [ ] CLI und `[replay]` validieren; vorhandene Aufnahme-/Screenshot-Parsingtests unverändert bestehen lassen.
- [ ] Pure Reservierungs-/Fenster-/Geometrie-Policy implementieren, ohne FFmpeg oder OS-Typen.
- [ ] Zustandsübergänge und Ressourcenbesitz als kleine explizite Modelle implementieren; Lebenszyklus und Gesundheitsmeldungen getrennt halten.
- [ ] Grenzwerte, Überläufe, ungültige Rechtecke, Unknown-Version und Windows-Nichtverfügbarkeit testen.

**Abnahme:** Dieselben reinen Tests laufen auch außerhalb Linux. Legacy-Request-Protokoll und Windows-Backend unverändert.

### P2: Supervisor, Prozessbesitz und Capability-Erkennung

**Dateien:** Linux-Replay-API, IPC, Supervisor, Worker-Adapter/Capabilities, Linux-Pfade. **Abhängigkeit:** P1.

- [ ] Eigenen privaten Socket, Daemon-Anbindung, Worker-Handshake, Timeout und begrenzte Eventqueues bauen.
- [ ] Capture-Child ausschließlich unter Worker-Besitz starten, Logs drainen, stdout/Medienkanal trennen und Exit zuverlässig behandeln.
- [ ] Encoder-/Geräte-Discovery, kurze Probes und Cache mit sauberer Invalidierung implementieren.
- [ ] Fehlende Tools, gescheiterte Initialisierung, hängendes Kind, Doppelstart, Socket-Race und Worker-Absturz testen.
- [ ] Queue-Besitz, Abbruch und begrenztes Shutdown bei blockierter Medien-I/O testen; Controller muss weiter Status und Stop bedienen können.

**Abnahme:** Start/Stop/Status funktionieren; absichtlicher Worker-Fehler beschädigt weder Shelf noch normale Aufnahme. Keine orphan Prozesse.

### P3: begrenzter Replay-Ring mit Audio

**Dateien:** Worker-Media/Ring/Audio und portable Policy. **Abhängigkeit:** P2.

- [ ] AVPacket-Ownership, Streamparameter, Side-Data, PTS/DTS und abgeschlossene sichere Abschnitte implementieren.
- [ ] Zeit- und Byteeviction, Reservierungen, große Pakete, Rückstau und langsame Verbraucher behandeln.
- [ ] Systemton, Drift, Stille, VFR, Startup/Warmup und neue Zeit-Epochen implementieren.
- [ ] Synthetische Streams mit sichtbarem Framezähler und synchronem Audioimpuls verwenden.

**Abnahme:** Mehrstundenlauf ohne monotones Speicherwachstum; deterministische Byteobergrenze; Historie bleibt decodierbar oder meldet klar fehlende Bereitschaft.

### P4: Snapshot und normaler Clip

**Dateien:** Worker-Snapshot/Export, portable Job-Policy, Linux-Shelf-Events. **Abhängigkeit:** P3.

- [ ] Historischen Endpunkt atomar fixieren, Ressourcen reservieren, Request-ID deduplizieren.
- [ ] Vollbild per Stream-Copy exportieren, tatsächliche Dauer und Grenzen zurückgeben.
- [ ] Sichere Dateifertigstellung und separate `ReplayClipReady`-Events zur vorhandenen Video-Karten-Erzeugung anbinden.
- [ ] Bestehendes Recording-Finalize-Event nicht missbrauchen: dessen Handler löscht Recording-Zustand und wäre für Replay falsch.
- [ ] Wiederholte Binds, Stop während Export, voller Datenträger, Namenskollisionen, Prozessfehler und abgebrochenen Client prüfen.

**Abnahme:** Kein Video-Reencode bei Normalclips; Auftrag verändert laufende Aufnahme nicht; kein Erfolg vor erfolgreich fertiggestellter Datei.

### P5: rückwirkender Selector

**Dateien:** Standbild-Export, Linux-Selector, portable Geometrie, CLI-Crop-Flow. **Abhängigkeit:** P4.

- [ ] Letzten Frame des Snapshots decodieren, Größenlimit prüfen, recorded Output und Geometrie übergeben.
- [ ] Selector gezielt um Bildquelle/Modus ergänzen, keine allgemeine Selector-Neuentwicklung.
- [ ] Auswahl in Video-Pixel abbilden, Alignment sichtbar machen, Escape und stale IDs behandeln.
- [ ] Mit absichtlich langer Auswahl testen: gespeicherter Zeitraum bleibt der des Bind-Aufrufs.

**Abnahme:** Fractional scaling, Rotation, negative Monitorpositionen und Fokuswechsel liefern den richtigen historischen Ausschnitt; Screenshot-/Recording-Selector bleiben unverändert bedienbar.

### P6: echter Crop und begrenzter Export

**Dateien:** Worker-Crop/Export und Job-Scheduler. **Abhängigkeit:** P5.

- [ ] Verifizierten GPU-Crop-Pfad für den ersten Hardwareadapter implementieren; benötigte Padding-/Farbraumschritte einbeziehen.
- [ ] Audio kopieren, gemeinsamen Zeitbezug erhalten; Full-Frame-Auswahl zu Remux optimieren.
- [ ] Pacing, maximale Queues, `waiting_for_budget`, Retry und Cancel implementieren.
- [ ] Quelldatei und Ausgabe mit Mustern testen, die außerhalb der Auswahl eindeutig erkennbar wären; komplette Ausgabeframes auf unerwünschte Bildinhalte prüfen.

**Abnahme:** Tatsächlich kleinere decodierte Bildfläche; korrekter Ton; Export unter Spielelast stört den laufenden Puffer nicht und wird bei Bedarf langsamer.

### P7: Encoderfamilien vollständig abdecken

**Dateien:** Adapter, Geräte-/Frame-Import, Profile, Capability-Fixtures. **Abhängigkeit:** P6, bei Bedarf P0-Nachweis pro Familie.

- [ ] NVENC, VA-API auf AMD/Intel, QSV, verfügbare AMF-/Vulkan-Pfade mit jeweils eigenen Aufnahme- und Crop-Probes anbinden.
- [ ] Freie Encoder-Namen/Optionen ohne Hersteller-Allowlist behandeln; unbekannte Hardwareframes konkret diagnostizieren.
- [ ] Nur bei nachgewiesenem Adapterdefizit schmalen nativen Import-/Capture-Pfad ergänzen.
- [ ] Software-Encoding ausdrücklich auswählbar machen, mit klarer Lastanzeige und denselben Speicher-/Zeitgarantien.
- [ ] Auf echter Hardware testen. Lokale NVIDIA-Ergebnisse gelten nicht als AMD-/Intel-Nachweis; fehlende Hardware im Abnahmeprotokoll offen lassen.

**Abnahme:** Jede beworbene Kombination hat einen erfolgreichen Capture-, Remux- und Crop-Nachweis. Nicht nutzbare installierte Encoder werden mit konkreter Ursache angezeigt.

### P8: Bedienoberfläche, Autostart und Wiederherstellung

**Dateien:** Tray/Status, Shelf-Anbindung, Config, Supervisor, `docs/replay.md`. **Abhängigkeit:** P4 bis P7.

- [ ] Start/Stop, Historienfüllstand, gewähltes Gerät/Encoder und Exportstatus ohne dauernde Vorschau darstellen.
- [ ] Persistente Einstellungen, zwei Bind-Beispiele und verständliche Fehlertexte ergänzen.
- [ ] Optionalen Replay-Autostart in vorhandenen Daemon-Start integrieren; keine zweite systemd-Servicefamilie.
- [ ] Output-/Audio-Wechsel, Suspend, simulierten Lock, Ressourcenende und Wiederherstellungsmanifest umsetzen.

**Abnahme:** Screenshot, normale Aufnahme und Replay haben getrennte Zustände und funktionieren nach Fehlern weiter. Gespeicherte Clips überleben Kartenentfernung und Neustart.

### P9: Leistungsabnahme, CI und Auslieferbarkeit

**Dateien:** Benchmark-Dokumentation, CI/Release, Paketmetadaten, README-Supportmatrix. **Abhängigkeit:** alle vorigen Pakete.

- [ ] Gesamte Messmatrix aus Abschnitt 10 durchführen, Profile anhand Messergebnissen festlegen.
- [ ] Worker-CI mit relevanten FFmpeg-Versionen und Medienfixtures ergänzen; Hauptprogramm ohne Worker-Abhängigkeiten prüfen.
- [ ] Lizenz-/Build-Konfiguration der tatsächlich ausgelieferten FFmpeg-Komponenten und externen Tools erfassen. Keine ungeprüften fremden Binaries bündeln.
- [ ] Optionale Paketinstallation, fehlende/inkompatible Worker-Version und Bibliotheks-ABI-Fehler verständlich behandeln.
- [ ] Dokumentation und Supportmatrix nur um nachgewiesene Fähigkeiten ergänzen. Keine Windows-Artefakte hinzufügen.

**Abnahme:** Fertig ist das Feature erst nach funktionalem Durchlauf, Fehlertests, Performance-Nachweisen und dokumentierten Hardwaregrenzen. Build-Erfolg allein reicht nicht.

## 10. Tests und messbare Abnahme

### Automatisierte Prüfungen

Für jede Änderung gemäß Repository-Regeln `cargo fmt --check` und `cargo test`, bei Linux-Code zusätzlich `cargo check`. Für den separaten Worker entsprechend `cargo fmt --manifest-path src/platform/linux/replay/worker/Cargo.toml --check`, `cargo test --manifest-path ... --locked` und `cargo check --manifest-path ... --locked`. Keine fehlende Hardware durch angeblich bestandene Hardwaretests ersetzen.

Gezielte Tests:

- Ring: zufällige Paketgrößen, Side-Data, wechselnde Timebases, fehlende Zeitstempel, große GOP, Wrap/Overflow, mehrfach geteilte Snapshots, Eviction bei offenen Aufträgen.
- Grenzen: Start zwischen Keyframes, Ende unter Encode-Verzögerung, wiederholte Requests, VFR/Stille, verkürzte Warmup-Historie, nicht unterstützte B-/Open-GOP-Strukturen.
- A/V: sichtbare Frame-ID mit Audioimpuls, Start/Ende, Priming, längerer Drift-Test und Quellenwechsel.
- Export: vollständiges Decodieren kurzer Fixtures, Crop-Inhalt/Dimensionen, Farbmetadaten, Audio, Container-Endung, kein Überschreiben, volle Disk und kaputte Pipe.
- IPC: übergroße Nachricht vor Allokation ablehnen, falsche Version/IDs, unvollständige Nachricht, Reader-Timeout, langsamer Watcher, Disconnect nach Annahme.
- Prozesse: Worker fehlt/crasht/hängt, Capture endet, Stop/Cancel beim Export, keine Deadlocks oder Waisen.
- Regression: bestehende Screenshots, normale Aufnahme, Pause/Resume, Clipboard/Shelf und Konfigurationsschreiben.

Fixtures werden synthetisch erzeugt und enthalten keine privaten Desktopaufnahmen. Entwicklung und Testbereinigung beachten die Repository-Regel für `/home/mt/.local/bin/safe-rm`; keine permanenten Löschbefehle oder Umgehungen.

### Performance-Matrix

Vergleichen: Replay aus, nur Capture/Encode, natives Tool-Replay als Referenz, Boltsnap-Replay, normaler Save, offene Bereichsauswahl, Crop-Export. Wiederholte A/B-Läufe in wechselnder Reihenfolge mit gleichem Inhalt, gleicher Auflösung und thermisch vergleichbarem Zustand.

Messwerte: Framezeiten des Vordergrundprogramms mit p50/p95/p99/p99.9, Durchschnitts-FPS, Capture-Cadence und ausgelassene Frames, CPU-Zeit, RSS, VRAM, GPU-Video-/Copy-Auslastung soweit verfügbar, Leistungsaufnahme, Paketbelegung, Speicherwachstum, Endpunktlatenz und Exportdauer. Keine ständig laufende teure Telemetrie im Produkt.

Szenarien: ruhiger Desktop, Textscrollen, dunkle UI, schnelle Spielbewegung, GPU-Limit, CPU-Limit, 1080p/1440p/4K entsprechend Hardware sowie 60/120/144/240 FPS. Qualität mit kleinen Schriften, Bewegungsdetails und dunklen Verläufen prüfen; objektive Metriken ergänzen Sichtprüfung, ersetzen sie nicht.

Abnahmeregeln:

- Zusätzliche Last des eigenen Rings/Transports gegenüber identischem Capture muss innerhalb des vorher ermittelten Messrauschens liegen oder durch eine konkret begründete, erneut optimierte Abweichung bewertet werden. Capture selbst bleibt separat sichtbar; kein pauschales Versprechen von 0 % Last.
- Ziel für Standardprofil: keine wahrnehmbare Verschlechterung im Nutzertest und keine reproduzierbare relevante Verschlechterung der Vordergrund-Framezeiten. Vor jedem Gerätevergleich Messrauschen, Laufdauer und erlaubte Streuung festhalten, keine nachträgliche Schönrechnung mit Durchschnitts-FPS.
- Vorläufiges Latenzziel im warmen Zustand: Job-Annahme p95 unter 100 ms und historischer Endpunkt p95 höchstens 100 ms hinter dem Aufruf. Bei Verfehlung Ursache Capture/Encode/Mux/Steuerung getrennt ausweisen und vor Default-Freigabe lösen oder den konkreten Profilgrenzwert dokumentieren.
- A/V-Abweichung bei Impulsfixtures höchstens eine Videoframe- plus eine Audiopaketdauer, ohne wachsenden Drift im Langlauf. Grenzüberhänge und unvermeidbares Priming ausdrücklich messen.
- Initial mindestens zwei Stunden pro repräsentativem Aufnahmeprofil, abschließend acht Stunden mit periodischen Saves/Crops. Kein monotones Wachstum und kein Überschreiten der definierten Paket-/Queue-/Diskreservierungen.
- Mindestens NVIDIA, AMD und Intel real abnehmen, bevor allgemeine Herstellerunterstützung behauptet wird. Hybrid-GPU, HDR und weitere Encoder nur mit separatem Nachweis als getestet markieren.

## 11. Quellen und Ausgangsbefund

Repository geprüft: CLI, Konfiguration, Legacy-IPC samt Windows-Aufrufern, Linux-Daemon/Shelf, Selector, Recording-Session/Audio/Finalisierung, Pfade, Cargo und CI/Release. Vorhandene Aufnahmeprofile und der Windows-Freeze bestimmen die Integrationsgrenzen.

GPU Screen Recorder wurde im offiziellen Git-Repository bei Commit `ab9cf696d013c59c02b7e98cbf103fd7735b0bbd` untersucht. Seine feste Encoderwahl und paketanzahlbasierte RAM-Historie erfüllen allein weder offene Encoderwahl noch unser Bytebudget. Der kontinuierliche Ausgabeweg ist deshalb Kandidat für den Capture-Adapter, nicht Beweis fertiger Integration. [Upstream](https://git.dec05eba.com/gpu-screen-recorder/about/)

wf-recorder löst Encoder per Namen auf; die vorhandene Hardware-Frame-Behandlung muss dennoch je Encoderfamilie passen. [Frame-Writer](https://github.com/ammen99/wf-recorder/blob/master/src/frame-writer.cpp)

FFmpeg dokumentiert NUT ohne wachsendes Indexverzeichnis sowie Container-/Flush-Optionen. Daraus folgt noch keine gemessene Ende-zu-Ende-Latenz. [Formate](https://ffmpeg.org/ffmpeg-formats.html#nut)

Refcounted Pakete und Codec-/Hardwareabfragen bilden die Basis des Workers. Ownership und Fehlerpfade bleiben Implementierungsarbeit. [Pakete](https://ffmpeg.org/doxygen/trunk/group__lavc__packet.html), [Codec-API](https://ffmpeg.org/doxygen/trunk/group__lavc__core.html)

`ffmpeg-next` ist ein Wrapper im Wartungsmodus und beschreibt Unterstützung verschiedener FFmpeg-Versionen. Versionsbindung und benötigte zusätzliche FFI-Oberfläche müssen deshalb im ersten Build-Nachweis geprüft werden. [Rust-FFmpeg](https://github.com/zmwangx/rust-ffmpeg)

GPU-Crop ist pfadspezifisch: NVIDIA dokumentiert Decoder-Crop; FFmpeg enthält Crop-Unterstützung im QSV-VPP und die Verarbeitung von Crop-Rechtecken im VA-API-VPP. Keine universelle Filterkette daraus ableiten. [NVIDIA](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/ffmpeg-with-nvidia-gpu/index.html), [QSV](https://ffmpeg.org/doxygen/trunk/vf__vpp__qsv_8c_source.html), [VA-API](https://ffmpeg.org/doxygen/trunk/vaapi__vpp_8c_source.html)

Planungsbaseline am 20.09.2026: `cargo test` besteht mit 214 Tests. `cargo fmt --check` meldet schon vor diesem Dokument Formatierungsabweichungen in `src/platform/linux/capture.rs`, `paths.rs` und `portal.rs`. Diese vorhandenen Änderungen wurden nicht angepasst. Das Anlegen dieses Plans implementiert keine Replay-Funktion.
