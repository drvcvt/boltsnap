# Boltsnap Windows – verbindlicher Portierungsplan

## Zielbild

Boltsnap bleibt **ein gemeinsames Rust-Projekt und ein gemeinsames Produkt**.
Es entstehen keine getrennten Quellbäume oder dauerhaft auseinanderlaufenden
Forks für Linux und Windows.

Windows erhält eine native Neuimplementierung aller Betriebssystemfunktionen.
Gemeinsame Semantik, Datenmodelle, Protokolle, Rendering-Code und Tests werden
aus dem vorhandenen Linux-Projekt übernommen. Die Windows-Implementierung ist
keine zeilenweise Übersetzung von Wayland-, X11- oder Unix-Code.

Verbindliche Grundsätze:

- eine CLI und ein Konfigurationsmodell
- ein gemeinsames IPC-Protokoll
- ein gemeinsames Shelf- und Selector-Design
- gemeinsame Bildverarbeitung und Dateiformate
- native Linux- und Windows-Backends
- Auswahl des Backends ausschließlich zur Compile-Zeit
- keine Win32-, Wayland-, X11- oder Unix-Typen in gemeinsamen APIs
- keine externen Linux-Helfer oder Unix-Emulation unter Windows
- keine stille Funktionsattrappe für noch nicht portierte Fähigkeiten

## Implementierungsstand

Phasen 0 bis 4 sind für den Windows-MVP umgesetzt. Phase 5 ist bis auf
Multi-Monitor-Recording und den Live-Watch-Stream umgesetzt.

Umgesetzt:

- `src/lib.rs` als plattformneutral testbarer gemeinsamer Kern
- portable IPC-Typen und Binärframing in `src/protocol.rs`
- öffentliche Recording-Zustände im gemeinsamen Protokoll statt im Linux-Backend
- gemeinsame Selector-Geometrie in `src/image.rs`
- zentrale Compile-Zeit-Auswahl in `src/platform/mod.rs`
- unveränderter Linux-IPC-Transport in `src/platform/linux/ipc.rs`
- nativer Windows-Named-Pipe-Client in `src/platform/windows/ipc.rs`
- gemeinsame, Linux- und Windows-spezifische Cargo-Abhängigkeiten getrennt
- alle Linux-Betriebssystemmodule unter `src/platform/linux/` eingeordnet
- Windows-Pfade über Known Folders und native Prozesshilfen implementiert
- Bild-Clipboard über die native Windows-Zwischenablage implementiert
- gemeinsames Shelf-, Selector- und Bild-Rendering ohne Design-Fork übernommen
- Windows-Binary, Doctor, Self-Test und gemeinsames Shelf-Debug-Rendering lauffähig
- Windows-Gesamttests und plattformneutrale Protokolltests erfolgreich
- DXGI-Capture für Monitor, virtuellen Desktop und Regionen mit GDI-Fallback
- Windows Graphics Capture für Fenster, aktive Fenster und Videoaufnahmen
- nativer Selector, Shelf, Recording-Controls und Tray mit gemeinsamem Rendering
- Named-Pipe-Server mit Benutzer-DACL und Named Mutex für Single Instance
- Bild- und Datei-Clipboard sowie OLE-Datei-Drag-and-drop
- Media-Foundation-H.264/AAC-Encoding mit WASAPI-Systemaudio und Mikrofon
- Per-Monitor-DPI-Awareness-V2-Manifest und Windows-Release-CI
- NSIS-Installer ohne externe Programm-Bundles
- unabhängiger Task-Scheduler-Autostart mit Neustart bei Fehlern

Noch offen:

> Kein Punkt aus dieser Liste wird ohne explizite Absprache mit dem Maintainer
> umgesetzt — erst vorschlagen, dann implementieren.

- Erster-Frame-Video-Thumbnails für Shelf-Karten (unter Windows bleibt der
  Platzhalter; offene Entscheidung: Media Foundation SourceReader oder
  externes ffmpeg)
- kombinierte und getrennte Multi-Monitor-Aufnahmen unter Windows
  (`record_default_target = "output:<name>"`/`"both"` fällt auf den fokussierten
  Monitor zurück)
- dauerhafter `recording watch --json`-Ereignisstream unter Windows
- Recording-Rahmenfenster für ausgewählte Regionen (der Windows-Selector bietet
  die Checkbox nicht an; `record_show_frame` wirkt nur unter Linux)
- Lese-Timeout für `call_daemon` auf der Windows-Named-Pipe (Linux: 5 s)
- Pipe-Server bedient eine Verbindung zur Zeit; ein Client in der Lücke spawnt
  einen redundanten (harmlosen, Mutex-geschützten) Daemon-Prozess
- Dead-Code-Warnings des Windows-Targets durch Linux-only-Helfer in geteilten
  Modulen per `#[cfg]` gaten
- Tray-Menü-Sprache angleichen (Windows deutsch, Linux englisch)
- HDR-Tonemapping, Hotplug/RDP/Sleep-Wake-Härtung und ARM64-Gerätetests
- Code Signing, automatischer Updatepfad und vollständige Windows-10/11-Testmatrix
- Linux-Gesamttests in einer Linux-Umgebung ausführen

## Repository-Struktur

```text
src/
├── main.rs
├── config.rs
├── image.rs
├── protocol.rs
├── selector/
│   ├── mod.rs
│   ├── edit.rs
│   └── render.rs
├── shelf/
│   ├── mod.rs
│   ├── model.rs
│   ├── layout.rs
│   ├── paint.rs
│   ├── thumbnail.rs
│   └── recording.rs
├── record/
│   ├── mod.rs
│   ├── session.rs
│   └── finalize.rs
└── platform/
    ├── mod.rs
    ├── linux/
    │   ├── mod.rs
    │   ├── capture.rs
    │   ├── clipboard.rs
    │   ├── window.rs
    │   ├── ipc.rs
    │   ├── paths.rs
    │   ├── tray.rs
    │   ├── recording.rs
    │   ├── audio.rs
    │   └── process.rs
    └── windows/
        ├── mod.rs
        ├── capture.rs
        ├── clipboard.rs
        ├── window.rs
        ├── ipc.rs
        ├── paths.rs
        ├── tray.rs
        ├── recording.rs
        ├── audio.rs
        └── process.rs
```

`src/platform/mod.rs` ist die einzige zentrale Auswahlstelle:

```rust
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "windows")]
pub use windows::*;
```

Die Plattform-API besteht bevorzugt aus kleinen Modulen und einfachen
Funktionen. Es werden keine dynamischen Trait-Objekte eingeführt, wenn eine
Compile-Zeit-Auswahl genügt. Das hält Binärgröße, Startzeit und Aufrufkosten
klein.

## Was aus dem vorhandenen Projekt übernommen wird

Der größte Teil des fachlichen Codes bleibt erhalten oder wird mit kleinen,
plattformneutralen Anpassungen übernommen:

- CLI-Befehle, Argumentsemantik und Ausgabeformate aus `main.rs`
- TOML-Parsing, Standardwerte und Prioritäten aus `config.rs`
- PNG-/JPEG-Verarbeitung und Thumbnail-Erzeugung
- serialisierte Requests, Responses und Recording-Snapshots aus `ipc.rs`
- Selector-Zustand und nahezu das gesamte `tiny-skia`-Rendering
- Shelf-Modell, Layout, Paint-Code, Badges, Icons und eingebettete Schrift
- Recording-Zustandsmaschine für Start, Pause, Resume, Finalize und Discard
- reine Segment-, Zeit-, Geometrie- und Dateinamenslogik
- Editor-Konfiguration und Prozessvertrag
- bestehende plattformneutrale Unit-Tests

Nicht übernommen werden direkte Wayland-, X11-, Unix-, systemd-, `ksni`-,
`wf-recorder`-, `pactl`- oder `hyprctl`-Aufrufe. Deren Verhalten wird über das
gemeinsame fachliche Vertragsmodell nativ für Windows implementiert.

## Gemeinsame Verträge

Gemeinsame Module arbeiten nur mit eigenen Datentypen:

```rust
pub struct CaptureRequest {
    pub target: CaptureTarget,
    pub cursor: bool,
}

pub enum CaptureTarget {
    VirtualDesktop,
    Monitor(MonitorId),
    Region(PhysicalRect),
    Window(WindowId),
    ActiveWindow,
}

pub struct CapturedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}
```

`MonitorId` und `WindowId` sind eigene opaque IDs. `HWND`, `HMONITOR`,
Wayland-Outputs oder X11-Handles bleiben vollständig im jeweiligen Backend.

Das gleiche Prinzip gilt für Clipboard, IPC, Tray, Audio, Recording und
Prozessverwaltung. Gemeinsame Logik entscheidet **was** passieren soll; das
Backend entscheidet **wie** es auf dem Betriebssystem umgesetzt wird.

## Windows-Kompatibilitätsziel

Primäres Ziel:

- Windows 11 x64
- Windows 10 ab Version 2004, x64
- Per-Monitor-DPI-Awareness V2
- mehrere Monitore, negative Desktop-Koordinaten und unterschiedliche DPI-Werte
- Querformat, Hochformat und gedrehte Monitore
- SDR als garantierter Basispfad
- HDR-Erkennung und korrekte Tonemapping-/Float-Pfade als eigener Meilenstein
- ARM64-fähige Architektur, aber ARM64-Binaries erst nach separaten Tests

Windows 10 Version 2004 ist die sinnvolle Untergrenze, weil dort unter anderem
`WDA_EXCLUDEFROMCAPTURE` für Selector, Shelf und Recording-Controls zuverlässig
verfügbar ist. Vorhandene Windows-11-Funktionen dürfen nur mit Laufzeitprüfung
verwendet werden.

## Native Windows-Technik

### Screenshot-Capture

Es wird ein hybrides Backend verwendet:

- **DXGI Desktop Duplication** für virtuellen Desktop, vollständige Monitore und
  frei gewählte Regionen
- **Windows Graphics Capture** für konkrete Fenster und das aktive Fenster
- **GDI `BitBlt`** ausschließlich als klar gekennzeichneter Kompatibilitäts-
  Fallback, nicht als Standardpfad

DXGI liefert schnelle D3D11-Texturen und eignet sich für Monitor-Stitching und
Bereiche über Monitorgrenzen. Windows Graphics Capture kann ein konkretes
Fenster unabhängig von sichtbarer Überdeckung aufnehmen.

Die Capture-Schicht berücksichtigt:

- Adapter- und Monitorzuordnung
- Rotation pro Output
- negative virtuelle Desktop-Koordinaten
- Cursor optional ein-/ausschalten
- Displaywechsel, Sleep/Wake und verlorene D3D-Geräte
- geschützte Inhalte und explizite Fehlermeldungen
- BGRA8-SDR sowie einen späteren HDR-Pfad

### Selector und Shelf

Das vorhandene `tiny-skia`- und `ab_glyph`-Rendering bleibt die visuelle
Wahrheitsquelle. Dadurch bleiben Farben, Abstände, Radien, Badges und Icons
identisch.

Windows verwendet:

- ein randloses Top-Level-Fenster pro betroffenem Monitor
- `WS_EX_TOOLWINDOW`, damit keine Taskleisten-Einträge entstehen
- `WS_EX_NOACTIVATE`, wo keine Texteingabe erforderlich ist
- per-pixel Alpha für transparente Flächen
- `WDA_EXCLUDEFROMCAPTURE` für eigene Overlays und Controls
- physische Pixel intern, logische Designwerte über den jeweiligen Monitor-DPI

Zunächst wird das vorhandene CPU-Rendering in premultipliziertes BGRA
übernommen und nur bei Zustandsänderungen aktualisiert. Erst wenn Messungen
einen Engpass zeigen, wird die Ausgabe auf DirectComposition/D3D11 umgestellt.
Das vermeidet einen riskanten UI-Rewrite und erhält das bestehende Design.

### Clipboard und Drag-and-drop

- Bilddaten über natives Win32-Clipboard mit `CF_DIBV5`
- zusätzlich registriertes `PNG`-Format für verlustfreie kompatible Übergabe
- Dateipfade über `CF_HDROP`
- Shelf-Drag-and-drop über OLE `IDataObject`, `IDropSource` und `DoDragDrop`
- COM-Initialisierung auf dem zuständigen UI-Thread

`arboard` kann während der frühen Screenshot-Phase als abgesicherter
Zwischenschritt dienen. Der vollständige Shelf-Vertrag benötigt jedoch OLE und
eine eigene Datenobjekt-Implementierung.

### IPC und Single Instance

- Named Pipe `\\.\pipe\boltsnap-<user>` statt Unix-Socket
- Pipe-Zugriff nur für den aktuellen Benutzer
- Named Mutex für genau eine Shelf-/Tray-Instanz
- versioniertes gemeinsames Request-/Response-Protokoll
- blockierende Pipe-Worker oder Overlapped I/O hinter einem schmalen Backend
- Windows Message Loop bleibt frei von langsamen Datei- oder Encoding-Aufgaben

### Tray und Autostart

- natives Notification-Area-/Tray-Icon mit Screenshot- und Aufnahmeaktionen
- Shelf, Selector und Recording-Controls bleiben vollständig titelleistenlos
- Autostart bevorzugt über Benutzer-Startup/Registry, nicht als Windows-Dienst
- keine erhöhte Berechtigung und keine UAC-Anforderung

### Recording

Die Zielimplementierung ist nativ und benötigt langfristig kein externes
FFmpeg:

- Windows Graphics Capture oder DXGI liefert D3D11-Frames
- Media Foundation transformiert und encodiert Video
- Hardware-H.264 wird bevorzugt, Software-H.264 ist der Pflicht-Fallback
- MP4-Ausgabe über Media Foundation Sink Writer
- WASAPI Loopback erfasst Systemaudio
- WASAPI Capture erfasst das Mikrofon
- gemeinsamer Mixer synchronisiert, resampelt und mischt beide Quellen
- AAC ist der Standard-Audiocodec
- mehrere Monitore werden vor dem Encoder in eine D3D11-Zieltextur komponiert
- Pause/Resume nutzt die bestehende Zustandsmaschine und korrigierte Zeitstempel

Ein aktuelles FFmpeg darf ausschließlich als Entwicklungs-, Diagnose- oder
zeitlich begrenztes Fallback dienen. Es ist nicht Teil des endgültigen
Windows-Laufzeitvertrags.

### Dateien und Prozesse

- Konfiguration: `%APPDATA%\boltsnap\config.toml`
- Cache: `%LOCALAPPDATA%\boltsnap\cache`
- Screenshots: Windows Known Folder `Pictures\Boltsnap`
- Recordings: Windows Known Folder `Videos\Boltsnap`
- Temporärdateien: Windows-Tempverzeichnis über die Standardbibliothek
- Prozessgruppen/SIGINT werden durch Prozess-Handles, kontrolliertes Beenden
  und bei Hilfsprozessen Windows Job Objects ersetzt
- Editor-Aufruf bleibt konfigurierbar und verwendet keine Shell-Kommandozeile

## Performance-Regeln

- keine Electron-, WebView- oder dauerhaft laufende Browser-Runtime
- kein Tokio-Runtime-Zwang für einfache UI- und Pipe-Abläufe
- D3D11-Ressourcen wiederverwenden; keine Geräteerzeugung pro Frame
- Capture und Encoding nie auf dem UI-Thread
- GPU-intern kopieren und komponieren, solange kein CPU-Bild benötigt wird
- CPU-Readback nur für Screenshot-Encoding, Clipboard oder Tests
- Thumbnails asynchron dekodieren und cachen
- Shelf und Selector nur bei Dirty-State neu rendern
- keine zusätzliche Bildkodierung zwischen Capture, Clipboard und Shelf
- Release-Build mit LTO und kleinem Codegen-Unit-Count beibehalten

Für Startzeit, erste Selector-Anzeige, 4K-Capture, Clipboard und Recording werden
Benchmarks angelegt. Optimierungen erfolgen anhand dieser Messungen statt durch
einen vorsorglichen Wechsel auf ein neues UI-Framework.

## Design-Parität

Das Windows-Design wird nicht neu interpretiert. Maßgeblich bleiben:

- vorhandene `tiny-skia`-Paint-Funktionen
- vorhandene Layoutkonstanten und Hit-Test-Bereiche
- vorhandene eingebettete Schrift und Icons
- gleiche Hover-, Close-, Save-, Copy-, Edit- und Drag-Zustände
- gleiche Reihenfolge und Stapelung der Shelf-Karten
- gleiche Recording-Badges und Controls
- gleiche CLI-Texte, soweit sie nicht Linux-spezifische Voraussetzungen nennen

Zusätzlich entstehen pixelbasierte Golden-Tests aus dem bestehenden
Headless-Rendering. Zulässige Unterschiede werden nur für DPI-Rundung und
systemabhängiges Window-Chrome definiert; eigenes Chrome bleibt pixelgleich.

## Cargo- und Build-Struktur

Gemeinsame Abhängigkeiten bleiben unter `[dependencies]`. OS-Crates werden
strikt getrennt:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
libwayshot = "..."
wayland-client = "..."
x11rb = "..."
ksni = "..."
libc = "..."

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "...", features = ["..."] }
```

Linux darf keine Windows-Bindings kompilieren; Windows darf keine Wayland-,
X11- oder Unix-Crates kompilieren. Gemeinsame Tests laufen auf beiden
Plattformen ohne versteckende `#[cfg]`-Attribute.

## Umbaufolge

Der Umbau erfolgt capability-by-capability. Jeder Schritt hält Linux lauffähig
und stellt für Windows entweder die echte Funktion oder einen eindeutigen
`unsupported`-Fehler bereit.

### Phase 0 – Gemeinsamer Kern

1. Protokolltypen aus `ipc.rs` nach `protocol.rs` extrahieren.
2. reine Bild- und Geometrietypen nach `image.rs` verschieben.
3. `src/platform/mod.rs` und beide Backend-Wurzeln anlegen.
4. Cargo-Abhängigkeiten nach Zielplattform trennen.
5. Linux-Code ohne Verhaltensänderung durch das neue Plattform-API führen.

### Phase 1 – Pfade, Prozesse und Build

1. Known-Folder-Backend implementieren.
2. Prozessstart und Editor-Aufruf abstrahieren.
3. Windows-Manifest mit Per-Monitor-DPI-Awareness V2 anlegen.
4. Windows- und Linux-CI parallel aufbauen.
5. portable Windows-Debug-Binary erzeugen.

### Phase 2 – Screenshot-MVP

1. Monitorenumeration und virtuelle Desktop-Geometrie.
2. DXGI-Capture für Monitor, Full und Region.
3. Windows-Graphics-Capture für Window und Active Window.
4. PNG-Dateiausgabe und Clipboard.
5. bestehende CLI-Verträge und Fehlerfälle testen.

### Phase 3 – Selector

1. Win32-Overlayfenster pro Monitor.
2. vorhandenes Selector-Rendering anbinden.
3. Maus, Tastatur, Abbruch und DPI-Wechsel implementieren.
4. Capture-Ausschluss und Multi-Monitor-Regionen testen.
5. visuelle Golden-Tests gegen das Linux-Design.

### Phase 4 – Shelf, IPC und Tray

1. Named-Pipe-Server und Single Instance.
2. Shelf-Fenster mit bestehendem Layout/Paint-Code.
3. Clipboard-, Save-, Edit- und Close-Aktionen.
4. OLE Drag-and-drop.
5. Tray-Menü und unsichtbaren Autostart integrieren.

### Phase 5 – Recording

1. Video-only-Aufnahme und H.264-Fallbackkette.
2. WASAPI-Systemaudio und Mikrofon.
3. A/V-Synchronisation und AAC.
4. Pause, Resume, Finalize und Discard.
5. getrennte und kombinierte Multi-Monitor-Aufnahmen.
6. Recording-Controls und Shelf-Integration.

### Phase 6 – Produktreife

1. HDR, Rotation, Sleep/Wake, Hotplug und RDP.
2. Crash-Recovery und temporäre Recording-Segmente.
3. Installer, portable ZIPs, Code Signing und Updates.
4. Windows-10/11-Testmatrix und Performance-Benchmarks.
5. Dokumentation und Capability-Matrix aktualisieren.

## Teststrategie

- vorhandene Unit-Tests für Shared-Code erhalten
- Linux: `cargo fmt --check`, `cargo test`, Linux-Build und Smoke-Tests
- Windows: `cargo fmt --check`, `cargo test`,
  `cargo check --target x86_64-pc-windows-msvc`
- Windows-Integrationstests für Named Pipes, Clipboard und Known Folders
- echte Smoke-Tests für Capture, Multi-Monitor, DPI und Recording
- Golden-Images für Shelf und Selector
- Gerätetests mit Intel-, AMD- und NVIDIA-GPUs
- Tests ohne Hardwareencoder und ohne Mikrofon
- Display-Hotplug-, Sperrbildschirm- und Explorer-Neustarttests

Windows-Support wird erst als fertig bezeichnet, wenn Capture, Clipboard,
Ausgabepfade und Fehlerfälle auf realen Windows-Systemen erfolgreich geprüft
wurden.

## Lokale Entwicklungsanforderungen

Vorhanden:

- Windows 11 25H2 x64
- Rust/Cargo 1.96.1 für `x86_64-pc-windows-msvc`
- Git

Noch erforderlich:

- Visual Studio Build Tools mit „Desktop development with C++“
- aktueller MSVC-x64/x86-Linker
- Windows 11 SDK
- optional Visual Studio Graphics Diagnostics und PIX für GPU-Analyse

FFmpeg ist für die endgültige native Zielarchitektur keine Voraussetzung.

## Upstream-Integration

- Basis ist `upstream/main` auf Commit `a746c37`.
- Der Arbeitsbranch `windows` verfolgt `upstream/main`.
- Vor jedem Capability-Schritt wird Upstream aktualisiert und anschließend
  gezielt rebased.
- Neue Linux-Änderungen werden nur einmal in den gemeinsamen Core oder das
  Linux-Backend integriert; Windows implementiert denselben Vertrag separat.
- Die Regeln aus `AGENTS.md` und `README.md#contributing` sind verbindlich.

```powershell
git fetch upstream
git rebase upstream/main
```

## Primärquellen

- https://github.com/microsoft/windows-rs
- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture
- https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
- https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
- https://learn.microsoft.com/en-us/windows/win32/medfound/sink-writer
- https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-operations
- https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- https://rust-lang.github.io/rustup/installation/windows-msvc.html
