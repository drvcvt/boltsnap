# boltsnap Screenshot-Shelf — Design (Phase 1)

**Status:** approved (design); plan written at `docs/superpowers/plans/2026-05-30-screenshot-shelf.md`
**Datum:** 2026-05-30
**Branch:** `feat/screenshot-shelf`
**Scope:** Phase 1 — schwebende Screenshot-Shelf. Phase 2 (Screen-Recording) ist
bewusst ausgeklammert und bekommt eine eigene Spec.

## 1. Ziel

boltsnap verhält sich heute als One-Shot-CLI: jeder Aufruf startet einen Prozess,
captured, kopiert ins Clipboard und beendet sich. Es gibt keine persistente UI.

Wir bauen ein macOS-artiges Verhalten: nach einem Screenshot erscheint eine kleine
schwebende **Thumbnail-Vorschau unten links am Bildschirm** (die "Shelf"). Mehrere
Screenshots stapeln sich dort, neuestes oben. Von dort aus kann man sie ins
Clipboard kopieren (Klick), per Drag-and-Drop in andere Apps ziehen, im
Annotation-Editor öffnen oder verwerfen.

Die Shelf ist **wlroots-/Hyprland-only** (Phase 1). X11 behält unverändert das
bisherige One-Shot-Verhalten.

## 2. Bestätigte Entscheidungen

| Thema | Entscheidung |
|---|---|
| Prozessmodell | **Hybrid (C):** Client self-spawnt Daemon bei Bedarf; `boltsnap daemon` zusätzlich explizit/autostartbar |
| Capture-Default | **Nur in die Shelf**, kein Auto-Copy. Klick=copy, Drag=DnD |
| Lebensdauer der Thumbnails | **Bleiben bis weggeklickt** / rausgezogen (kein Auto-Fade) |
| Persistenz | **Nur RAM**, flüchtig. Daemon-Neustart = leere Shelf |
| Drag-Fallback | **Auto-Copy bei Fehldrop** (Drop ins Leere / ablehnende App) |
| Shelf-Rendering | Rohe `wl_shm`-Buffer, selbst gezeichnet (kein GL/Vulkan im Daemon) |
| Drag-MIME-Typen | `image/png` **und** `text/uri-list` (max. Kompatibilität, auch XWayland) |
| Editor | Bestehender eframe-Editor, unverändert wiederverwendet |
| X11 | Unverändert One-Shot, keine Shelf |

## 3. Warum nicht eframe für die Shelf

eframe/winit kann auf Wayland zwei Dinge nicht, die für die Shelf zwingend sind:

1. **Keine `wlr-layer-shell`-Surface** — winit erzeugt nur normale xdg-Toplevels.
   Für "an die Ecke geankert, immer oben, kein Fokus-Klau" brauchen wir
   layer-shell (overlay layer).
2. **Keine Drag-Quelle** — winit ruft nie `wl_data_device.start_drag` auf.
   "Thumbnail rausziehen und in andere App droppen" ist damit unmöglich.

→ Die Shelf nutzt rohes **smithay-client-toolkit (SCTK)** mit layer-shell +
`wl_data_device` + `wl_shm`. Auswahl-Overlay und Editor bleiben eframe.

## 4. Prozessmodell (Hybrid)

Ein Binary, drei Rollen:

### Capture-Client — `boltsnap area | full | window | active-window`
- Macht Capture + Selection mit dem **bestehenden** Code (libwayshot + eframe-Overlay).
- Ergebnis: fertige PNG-Bytes.
- Verbindet sich zum Daemon-Socket. Existiert keiner → startet den Daemon (self-spawn),
  wartet kurz, retry. Schickt das Bild, dann Exit.

### Shelf-Daemon — `boltsnap daemon`
- Langlebig. Bindet `$XDG_RUNTIME_DIR/boltsnap.sock` (Single-Instance:
  Bind schlägt fehl → läuft schon).
- Hält die layer-shell-Surface und alle Thumbnails im RAM.
- Führt Clipboard-Copy und Drag-Source selbst aus.

### Editor — `boltsnap edit <tmp.png>`
- Bestehender eframe-Annotation-Editor, unverändert.
- `✎` am Thumbnail startet ihn; das Ergebnis ersetzt das Thumbnail in der Shelf.

**Single-Instance & Lifecycle:** Der Daemon hält den Socket. Ein Client, dessen
`connect()` fehlschlägt, forkt den Daemon (`boltsnap daemon` detached), pollt den
Socket mit kurzem Timeout und sendet dann. Autostart ist optional
(`exec-once = boltsnap daemon` in der Hyprland-Config), aber nicht erforderlich.

## 5. Code-Struktur

Die aktuelle `src/main.rs` (~2207 Zeilen) wird modularisiert. Bestehende Logik
wandert um, Shelf-Code kommt neu dazu:

```
src/
├─ main.rs          — CLI-Parsing, Routing (area/full/window/daemon/edit/doctor)
├─ capture/         — bestehender Wayland (libwayshot) + X11 (x11rb) Code
├─ select.rs        — bestehender eframe Auswahl-Overlay
├─ editor/          — bestehender eframe Annotation-Editor
├─ clipboard.rs     — wl-clipboard-rs / arboard
│  ─────────── NEU ───────────
├─ ipc.rs           — Unix-Socket-Protokoll (Client ⇄ Daemon)
├─ shelf/mod.rs     — Daemon: State, Thumbnail-Liste, Lifecycle
├─ shelf/layer.rs   — SCTK: wlr-layer-shell-Surface, wl_shm-Rendering
├─ shelf/input.rs   — Pointer: Hover, Klick, Drag-Start
├─ shelf/dnd.rs     — wl_data_device Drag-Source (png + uri-list)
└─ shelf/paint.rs   — Thumbnails, runde Ecken, Hover-Icons ins Buffer zeichnen
```

**Neue Crates:** `smithay-client-toolkit` (layer-shell, DnD, shm),
`wayland-protocols-wlr`. `image` (schon vorhanden) für Skalierung.

**Rendering:** Kein GL/Vulkan im Daemon → Kaltstart bleibt schnell. Text/Icons
werden in v1 als Vektor-Shapes direkt ins `wl_shm`-Buffer gezeichnet → keine
Font-Abhängigkeit. (Die Editor-Icons via egui-phosphor bleiben dem Editor
vorbehalten.)

## 6. IPC-Protokoll

Unix-Domain-Socket unter `$XDG_RUNTIME_DIR/boltsnap.sock`.

**Frame:** 4-Byte Big-Endian Längen-Präfix für den JSON-Header, gefolgt vom
JSON-Header, gefolgt vom optionalen Binär-Payload (PNG-Bytes), dessen Länge im
Header steht.

Befehle (Client → Daemon):
- `ADD` — Header `{ "cmd": "add", "png_len": <n>, "source": "<area|full|...>" }` + n Bytes PNG.
- `PING` — Liveness-Check beim self-spawn-Retry; Daemon antwortet `PONG`.

Antworten sind klein und JSON. Das Protokoll ist bewusst minimal; spätere
Befehle (z. B. `list`, Recording in Phase 2) lassen sich additiv ergänzen.

## 7. Datenfluss eines Screenshots

1. Hotkey → `boltsnap area` (Client).
2. Client: parallel capture + Auswahl-Overlay → fertiges RGBA/PNG (bestehender Pfad).
3. Client: `connect()` zu `boltsnap.sock`. Kein Daemon → self-spawn, poll, retry.
4. Client: sendet `ADD`-Frame (Header + PNG-Bytes) → Client-Exit.
5. Daemon: schreibt PNG in eine Temp-Datei (für Editor & Drag-`uri-list`), skaliert
   ein Thumbnail, fügt es oben in die Shelf ein, mappt/aktualisiert die layer-surface.
6. Thumbnail hovert unten links — bis es genutzt oder weggeklickt wird.

## 8. Interaktion pro Thumbnail

| Geste | Wirkung |
|---|---|
| Klick (kurz) | PNG ins Clipboard |
| Drücken + ziehen | Wayland-DnD startet (`image/png` + `text/uri-list`) |
| Hover | Icons `✎ ⧉ ✕` erscheinen |
| `✎` | Öffnet Editor; Ergebnis ersetzt das Thumbnail |
| `⧉` | Copy ins Clipboard |
| `✕` | Thumbnail verwerfen (Temp-Datei aufräumen) |
| Drag → Fehldrop | Auto-Copy ins Clipboard (Fallback) |

**Drag-MIME-Typen:** `image/png` (native Wayland-Apps nehmen das Bild direkt) und
`text/uri-list` (Datei-Pfad; viele Apps, auch XWayland, droppen lieber eine Datei).
Wird der Drop nirgends akzeptiert, greift Auto-Copy.

## 9. Shelf-Layout

- Anker: untere linke Ecke (layer-shell anchor bottom-left, mit Margin).
- Thumbnails gestapelt, **neuestes oben** (vertikal, `column-reverse`-Logik).
- Feste Thumbnail-Breite (Richtwert ~170 px), Höhe proportional zum Seitenverhältnis,
  gedeckelt. Runde Ecken, dezenter Rand/Schatten.
- Hover-Zone pro Thumbnail blendet die drei Icons oben rechts + Zeitstempel unten
  links ein.
- Die layer-surface ist nur so groß wie der aktuelle Stapel (kein ganzseitiges
  Overlay), damit Klicks außerhalb normal an den Desktop gehen.

## 10. doctor-Erweiterungen

- Prüft, ob das Compositor `wlr-layer-shell` anbietet.
- Prüft, ob ein Daemon läuft / der Socket erreichbar ist.

## 11. Testing

- **Unit:** IPC-Framing (encode/decode-Roundtrip), Shelf-State (add/remove/reorder),
  Thumbnail-Skalierung, Layout-Geometrie (Trefferzonen der Icons).
- **Bestehende Tests** (Parser, `render_annotations`, Hypr-Geometrie …) bleiben grün —
  der Refactor darf nichts brechen.
- **Manuell auf Hyprland:** capture → erscheint Thumbnail? Klick→paste? Drag in
  Discord/Helium/Terminal? `✕`→weg? Fehldrop→Clipboard?
- **doctor:** neue Checks liefern sinnvolle Ausgabe.

## 12. Phasen

- **Phase 1 (diese Spec):** Daemon, IPC, layer-shell-Shelf, Thumbnails, Klick=copy,
  Drag=DnD, Hover-Icons, Editor-Anbindung, Auto-Copy-Fallback.
- **Phase 2 (später, eigene Spec):** Screen-Recording-Modus für kurze Clips, die als
  Video-Thumbnail in dieselbe Shelf wandern. Erst wenn Phase 1 steht.

## 13. Risiken / offene Punkte

- **SCTK-Lernkurve / API-Stabilität:** layer-shell + DnD roh ist fummelig; konkrete
  Crate-Versionen werden im Implementierungsplan fixiert.
- **DnD-Akzeptanz** variiert je Ziel-App (besonders XWayland). Der `uri-list`-Typ +
  Auto-Copy-Fallback decken das ab; volle Garantie für jede App gibt es nicht.
- **HiDPI/Scaling:** Thumbnail-Größen und Maus-Trefferzonen müssen den
  Output-Scale-Faktor berücksichtigen.
- **Multi-Monitor:** Auf welchem Output die Shelf erscheint (fokussierter Output)
  wird im Plan festgelegt; v1 darf "fokussierter Monitor" annehmen.
- **Binär-Größe:** SCTK zieht zusätzliche Wayland-Crates; akzeptabel.
