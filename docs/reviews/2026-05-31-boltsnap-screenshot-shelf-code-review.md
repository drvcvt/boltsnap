# Code Review: boltsnap Screenshot Shelf

Datum: 2026-05-31  
Branch: `feat/screenshot-shelf`  
Scope: aktueller Worktree inkl. `main...HEAD` und uncommitted `src/shelf/mod.rs`.

## Kurzfazit

Die neue Richtung ist architektonisch sinnvoll: Capture-Client + langlebiger Wayland-Shelf-Daemon, pure Teilmodule für IPC/Layout/Model/Thumbnail/Paint und wiederverwendeter Editor/Selector sind eine gute Basis. Die Implementierung ist aber noch nicht review-clean: Es gibt mindestens einen potenziell destruktiven Edit-Pfad, mehrere Robustheits-/Privacy-Lücken im Daemon/IPC und klare Drift zwischen Spec, README und Code.

**Status:** nicht merge-ready, bis Blocker/Majors unten adressiert oder bewusst akzeptiert sind.

## Review-Methode

- Lokale Inspektion von `src/main.rs`, `src/ipc.rs`, `src/shelf/*`, `src/capture.rs`, `src/paths.rs`, README und Shelf-Spec.
- Diff-Scope: `git diff main...HEAD` plus uncommitted `src/shelf/mod.rs`.
- Zwei unabhängige read-only Reviewer wurden genutzt: ein Korrektheits-/Regression-Review und ein Codequalitäts-/Test-/Docs-Review.
- Keine GUI/Wayland-UI gestartet; DnD/Layer-Shell/Hyprland-Interaktion ist hier nur statisch geprüft.

## Checks

| Check | Ergebnis |
|---|---|
| `cargo build --release --locked` | PASS |
| `cargo test` | PASS, 24 Tests; Warnung: unused import `image::ImageBuffer` in `src/main.rs:389` |
| `cargo test --release --locked` | PASS, 24 Tests; gleiche Warnung |
| `cargo fmt --check` | FAIL, Format-Diffs in vielen Dateien |
| `cargo clippy --all-targets -- -D warnings` | FAIL, 22+ Errors (unused import, collapsible-if, too_many_arguments, identity_op, needless_return, …) |
| `git diff --check main...HEAD; git diff --check` | PASS |

## Positiv aufgefallen

- Der frühere Großklumpen wurde stark modularisiert: `capture`, `clipboard`, `editor`, `ipc`, `paths`, `select`, `shelf/*`.
- Pure Logik ist testbar und teilweise getestet: IPC-Roundtrip, `ShelfModel`, Layout-Hit-Testing, Thumbnail-Skalierung, Paint-Helfer.
- Release-Build und Release-Tests laufen mit `--locked` durch; der Release-Workflow sollte also nicht am Build scheitern.
- X11/Wayland-Backends sind weiterhin klar getrennt; das Shelf ist im Code auf Wayland-Routing begrenzt.

## Blocker

### B1 — `--edit -o PATH` kann vorhandene Dateien vor dem Speichern überschreiben

`capture_flow` nimmt bei `args.edit` direkt `edit_output_path(args)` als Capture-Ziel (`src/main.rs:297-302`) und startet danach den Editor mit demselben Pfad als Input und Output (`src/main.rs:308`). Der Editor schreibt erst beim Save nach `output_path` (`src/editor.rs:229-231`).

Auswirkung: `boltsnap area --edit -o important.png` überschreibt `important.png` schon durch den Roh-Capture, bevor der Nutzer im Editor speichert oder abbricht. Das ist ein Datenverlust-Risiko und eine Regression gegenüber dem sichereren Tempfile-Workflow.

Empfehlung:
- Für `--edit` immer in ein Tempfile capturen.
- `-o/--save` nur als finalen Editor-Output verwenden.
- Test ergänzen: vorhandene `-o`-Datei bleibt unverändert, wenn der Editor abbricht/fake-failt.

### B2 — Quality Gates sind nicht sauber

`cargo fmt --check` scheitert. `cargo clippy --all-targets -- -D warnings` scheitert u. a. an `src/main.rs:389`, `src/capture.rs:364`, `src/ipc.rs:80`, `src/paths.rs:23`, `src/select.rs:187`, `src/shelf/paint.rs:85` und mehreren Stellen in `src/shelf/mod.rs`.

Auswirkung: Der Branch wirkt unfertig; spätere kleine Änderungen werden schwerer reviewbar.

Empfehlung:
- `cargo fmt` laufen lassen.
- Unused import entfernen.
- Clippy entweder fixen oder einzelne bewusst akzeptierte Paint-Helfer mit lokalem `#[allow(clippy::too_many_arguments)]` begründen.

## Major Findings

### M1 — X11 `--no-copy` ohne Output wird ignoriert

`--no-copy` setzt `args.copy = false` (`src/main.rs:182-183`). `decide_post_capture` routet aber jeden Nicht-Wayland-Fall ohne Output zu `PostCapture::CopyOnly` (`src/main.rs:137-140`), und `CopyOnly` kopiert immer ins Clipboard (`src/main.rs:330-331`).

Auswirkung: `boltsnap --backend x11 --no-copy` kopiert trotzdem. Das widerspricht der CLI-Semantik und der Aussage, X11 bleibe klassisch/kompatibel.

Empfehlung:
- `CopyOnly { copy: bool }` oder separater `RememberOnly`-Pfad.
- Routing-Tests für Wayland/X11 × `--copy`/`--no-copy`/`-o`/`--edit` ergänzen.

### M2 — IPC kann den Daemon blockieren oder zu groß allozieren

`read_frame` vertraut ungeprüften `u32`-Längen und allokiert Header/Payload direkt (`src/ipc.rs:28-33`). `handle_client` läuft synchron im calloop-Accept-Pfad (`src/shelf/mod.rs:250-252`) und wartet bis zu 5 Sekunden pro Client (`src/shelf/mod.rs:321-322`).

Auswirkung: Ein lokaler fehlerhafter oder bösartiger Client kann die Shelf-UI blockieren oder sehr große Speicherallokationen auslösen.

Empfehlung:
- Harte Caps für Header/Payload, z. B. Header <= 64 KiB, PNG <= konfigurierbares Limit.
- Truncated-/Oversized-Frame-Tests.
- Client-Reads aus dem Wayland-Eventloop herausziehen oder nonblocking/framed in calloop modellieren.

### M3 — Privacy-/Lifecycle-Drift: Shelf ist nicht wirklich RAM-only

README verspricht: „The shelf is RAM-only“ und beim Daemon-Restart leer (`README.md:171-172`). `add_png` schreibt aber jede Aufnahme als volle PNG-Datei in `temp_png("shelf")` (`src/shelf/mod.rs:375-376`). Aufräumen passiert nur beim expliziten Close (`src/shelf/mod.rs:406`); beim Daemon-Ende wird nur der Socket entfernt (`src/shelf/mod.rs:269`).

Auswirkung: Screenshots können nach Daemon-Crash/Restart weiter auf Disk liegen. Das ist besonders für Screenshot-Tools ein Privacy-Befund.

Empfehlung:
- Entweder README/Spec ehrlich auf „RAM + temporäre Datei für Editor/DnD“ ändern.
- Oder Tempfiles tracken und per Drop/Shutdown/Startup-GC entfernen.
- Mittelfristig: `tempfile` statt PID+Millis-Pfad.

### M4 — Layer-Surface-Recreate kann gegen das wlr-layer-shell-Protokoll laufen

Beim Monitorwechsel erstellt `place_on_focused_output` eine neue Layer-Surface und macht initial einen bufferlosen Commit (`src/shelf/mod.rs:288-300`). Direkt danach ruft `add_png` aber `self.draw(qh)` (`src/shelf/mod.rs:386-388`), welches einen Buffer attached und committed (`src/shelf/mod.rs:495-525`). Das Protokoll sagt, vor dem ersten `layer_surface.configure` dürfen keine Buffer attached werden; erst nach Configure/Ack ist Mapping erlaubt (`wlr-layer-shell-unstable-v1.xml:44-53`).

Auswirkung: Auf Monitorwechseln droht ein Wayland-Protokollfehler oder compositorabhängiges Verhalten.

Empfehlung:
- Nach Surface-Recreate `pending_draw = true` setzen und erst im `configure`-Handler zeichnen.
- Manuell mit `WAYLAND_DEBUG=1`: Daemon auf Monitor A, Fokus Monitor B, Capture.

### M5 — Tests decken das neue Routing kaum ab

`main.rs` enthält aktuell Parser-Tests, aber keine Tests für `decide_post_capture`/`capture_flow`-Semantik (`src/main.rs:391-437`). Gerade dort liegen die neuen Default-Änderungen: Wayland default → Shelf, `--copy` → Shelf+copy, X11 → altes Verhalten.

Empfehlung:
- Unit-Tests für `decide_post_capture`: stdout, edit, file, Wayland shelf, Wayland `--copy`, X11 `--no-copy`.
- Falls nötig `PostCapture`/`decide_post_capture` testfreundlich halten, nicht über Integrationstests erzwingen.

### M6 — `src/shelf/mod.rs` ist ein neuer God-Module

`shelf/mod.rs` hat 831 Zeilen und mischt Daemon-Startup, Socket-Accept, IPC, Layer-Surface-Lifecycle, Monitor-Placement, Pointer-Handling, DnD, Editor-Spawn und Rendering-Orchestrierung.

Auswirkung: Der riskanteste Teil des Features ist am schwersten zu reviewen und zu testen. Fehler wie M2/M4 verstecken sich genau in dieser Kopplung.

Empfehlung:
- Split in mindestens: `shelf/layer.rs`, `shelf/input.rs`, `shelf/dnd.rs`, `shelf/ipc_server.rs`.
- `mod.rs` nur als Orchestrator/Daemon-State behalten.

## Minor Findings

### m1 — Spec/IPC-Protokoll driften

Spec beschreibt 4-Byte Header-Länge + JSON mit `png_len` und PING→PONG (`docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md:108-114`). Code nutzt `[header_len][payload_len][header][payload]` (`src/ipc.rs:16`), `ADD` ohne `png_len` (`src/ipc.rs:42-44`) und `daemon_alive` liest PONG nicht (`src/ipc.rs:89-96`), obwohl der Daemon PONG schreibt (`src/shelf/mod.rs:333-335`).

Empfehlung: Entweder Spec aktualisieren oder Golden-Frame-Tests gegen die Spec schreiben.

### m2 — README/CLI Usage ist widersprüchlich zur neuen Wayland-Default-Semantik

README-Beispiele sagen weiterhin `boltsnap`/`boltsnap full` kopieren (`README.md:84`, `README.md:89`), während der Shelf-Abschnitt sagt, Wayland captures landen defaultmäßig in der Shelf (`README.md:138-177`). Auch `usage()` in `src/main.rs:154-159` ist noch copy-lastig.

Empfehlung: Usage-Matrix nach Backend/Sink explizit machen.

### m3 — Hyprland-No-Anim-Kommentar verweist auf nicht vorhandene README-Doku

Der uncommitted Kommentar sagt, statische `noanim`-Regeln seien im README dokumentiert (`src/shelf/mod.rs:116-122`). README enthält aber nur Daemon-Autostart, keine `windowrule`/`layerrule`/Hyprlua-Snippets.

Empfehlung: README ergänzen oder Kommentar abschwächen.

### m4 — `doctor` erfüllt die Spec nicht vollständig

Spec fordert Prüfung, ob der Compositor `wlr-layer-shell` anbietet (`docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md:156-159`). `print_doctor` meldet aktuell nur Wayland-Session, Daemon und Socket (`src/paths.rs:39-50`).

Empfehlung: Registry-Global für `zwlr_layer_shell_v1` prüfen oder die Spec/README entsprechend reduzieren.

### m5 — Erfolgreicher Drag räumt Thumbnail nicht ab

Spec/README formulieren, dass die Shelf-Items bleiben, bis sie genutzt/weggeklickt/rausgezogen werden (`docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md:29`, `README.md:151-159`). `dnd_finished` räumt nur Drag-State auf (`src/shelf/mod.rs:770-775`), entfernt aber das Thumbnail nicht.

Empfehlung: UX entscheiden: Nach erfolgreichem Drop entfernen oder Docs auf „bleibt nach Drag erhalten“ ändern.

### m6 — Hover-Zeitstempel fehlt

Spec verlangt Icons plus Zeitstempel auf Hover (`docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md:151-152`). `draw_hover_icons` zeichnet nur drei Buttons (`src/shelf/paint.rs:187-199`).

Empfehlung: Entweder Timestamp implementieren oder Spec kürzen.

### m7 — Hit-Zonen und Paint-Geometrie sind dupliziert

`Layout::icon_rect` berechnet Hit-Zonen (`src/shelf/layout.rs:56-63`), `draw_hover_icons` spiegelt dieselbe Mathematik manuell (`src/shelf/paint.rs:187-195`).

Empfehlung: `icon_rect` public/internal teilen, damit Buttons und Hit-Zonen nicht auseinanderlaufen.

### m8 — `text/uri-list` wird nicht URI-escaped

DnD schreibt `file://{abs.display()}\r\n` (`src/shelf/mod.rs:739-742`). Pfade mit Leerzeichen, `#`, `%`, Nicht-ASCII usw. können in Zielapps falsch ankommen.

Empfehlung: URI-Encoding nutzen; idealerweise mit kleinem Helper + Tests.

### m9 — HiDPI/Scaling ist offen

Spec nennt HiDPI als Risiko (`docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md:184-185`), aber `scale_factor_changed` ist leer (`src/shelf/mod.rs:530-531`).

Empfehlung: Auf 1.25/1.5/2.0 Scaling prüfen; ggf. Buffer-Scale und Pointer-Koordinaten sauber modellieren.

### m10 — Workspace-Hygiene

Aktueller Worktree enthält neben `src/shelf/mod.rs` untracked `.claude/` und `docs/handoffs/2026-05-31-screenshot-shelf.md`. Das ist nicht per se falsch, sollte aber vor Merge/Release bewusst sortiert werden.

## Testlücken / manuelle Validierung

Priorität hoch:

1. `--edit -o existing.png` darf `existing.png` nicht vor Editor-Save verändern.
2. `--backend x11 --no-copy` darf nicht kopieren.
3. Oversized/truncated IPC frames dürfen Daemon nicht blockieren/killen.
4. Daemon-Restart/Crash muss Tempfiles entweder löschen oder Docs müssen Disk-Temp klar benennen.
5. Monitorwechsel: Capture auf anderem fokussierten Monitor ohne Wayland-Protokollfehler.

Manuell auf Hyprland/wlroots:

- `boltsnap area/full/window` → Thumbnail auf richtigem Monitor.
- Klick → Clipboard.
- Drag in native Wayland-App und XWayland-App.
- Fehldrop → Clipboard-Fallback.
- Editor über ✎ → Thumbnail reload.
- Close ✕ → Thumbnail und Tempfile weg.
- Scaled Monitor / Mixed-DPI.
- `boltsnap doctor` auf wlroots und Nicht-wlroots-Wayland.

## Empfohlene Fix-Reihenfolge

1. B1 Fix: Edit-Capture immer in Tempfile, finaler Output separat.
2. B2 Fix: `cargo fmt`, Clippy-Warnungen bereinigen/gezielt erlauben.
3. M1 Routing-Tests + X11 `--no-copy` korrigieren.
4. M2 IPC caps + blocking aus Eventloop entschärfen.
5. M3 Tempfile-Lifecycle oder Docs/Privacy-Statement korrigieren.
6. M4 Surface-Recreate erst nach Configure zeichnen.
7. Docs/Spec/README konsolidieren.
8. `shelf/mod.rs` danach splitten, solange Kontext frisch ist.
