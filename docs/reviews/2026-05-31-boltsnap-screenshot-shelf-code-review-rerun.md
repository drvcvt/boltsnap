# Code Review Rerun: boltsnap Screenshot Shelf / Redesign

Datum: 2026-05-31  
Branch: `feat/screenshot-shelf`  
Scope: aktueller Branch nach den neuen Shelf-Redesign-Commits, inkl. Vergleich mit `docs/reviews/2026-05-31-boltsnap-screenshot-shelf-code-review.md`.

## Kurzfazit

Ein paar Dinge sind besser: Die Shelf-Redesign-Richtung ist konkreter, Tests sind von 24 auf 30 gewachsen, feste 260×180-Karten, zwei Hover-Buttons, Preview-Math und Drag-Icon-Pixeltests sind vorhanden. Trotzdem ist der Branch weiterhin **nicht merge-ready**. Mehrere alte Review-Blocker sind unverändert offen, und die neue Lightbox/Preview bringt neue konkrete Ursachen für „behaves weird“ mit.

Die wichtigsten Verdächtigen für komisches Verhalten:

1. Preview-Lightbox wird mit `output = None` erstellt, obwohl der Shelf-Code selbst sagt, Hyprland mappt Null-Output-Layer nicht zuverlässig.
2. Preview hat laut Spec X/Edit-Buttons, im Code aber nur „click anywhere closes“.
3. Drag-Icon nutzt die gecroppte Karte, nicht die volle Screenshot-Ansicht.
4. Shelf-Surface-Recreate zeichnet weiterhin direkt nach dem initialen bufferlosen Commit.
5. README beschreibt noch das alte Click=Copy/Copy-Button-Verhalten.

## Checks

| Check | Ergebnis |
|---|---|
| `cargo build --release --locked` | PASS |
| `cargo test` | PASS, 30 Tests; Warnung: unused import `image::ImageBuffer` in `src/main.rs:389` |
| `cargo test --release --locked` | PASS, 30 Tests; gleiche Warnung |
| `cargo fmt --check` | FAIL, weiterhin Format-Diffs in mehreren Dateien |
| `cargo clippy --all-targets -- -D warnings` | FAIL, 23+ Errors / 25 test-target Errors |
| `git diff --check main...HEAD; git diff --check` | PASS |

Hinweis: Ein Subagent-Reviewlauf wurde versucht, ist aber im Harness mit `unknown error, write` fehlgeschlagen. Dieser Report basiert daher auf lokaler Inspektion + Checks.

## Seit dem letzten Review sichtbar verbessert

- `src/shelf/mod.rs` ist nicht mehr uncommitted; die Änderungen sind in Commits gelandet.
- Tests sind von 24 auf 30 gestiegen; neu dabei sind u. a. `shelf::preview::*`, `shelf::thumbnail::*` und Drag-Icon-/Hover-Paint-Tests.
- Feste Karten sind implementiert: `CARD_W = 260`, `CARD_H = 180` und `make_card_thumbnail` croppt auf exakt diese Größe (`src/shelf/thumbnail.rs:5-25`).
- Hover-Layout hat nur noch `Body`, `Edit`, `Close`; `Copy` ist aus `Hit` raus (`src/shelf/layout.rs:27-31`, `src/shelf/layout.rs:87-91`).
- Right-click-copy ist im Pointer-Handling angekommen (`src/shelf/mod.rs:828-834`).
- Drag-Icon hat jetzt einen retained `drag_icon_pool`, der erst bei cancel/finish freigegeben wird (`src/shelf/mod.rs:64-67`, `src/shelf/mod.rs:537-550`, `src/shelf/mod.rs:970-984`).

## Noch offene alte Blocker/Majors

### B1 — `--edit -o PATH` überschreibt weiterhin vor Editor-Save

Unverändert: `capture_flow` nutzt bei `args.edit` weiterhin `edit_output_path(args)` als Capture-Ziel (`src/main.rs:297-298`) und gibt denselben Pfad danach als Editor-Input/Output weiter (`src/main.rs:308`).

Auswirkung: `boltsnap area --edit -o important.png` kann `important.png` schon beim Roh-Capture überschreiben, bevor der Nutzer im Editor speichert.

### B2 — fmt/clippy weiterhin rot

`cargo fmt --check` scheitert. `cargo clippy --all-targets -- -D warnings` scheitert u. a. an:

- unused import `image::ImageBuffer` (`src/main.rs:389`)
- `collapsible_if` in `src/capture.rs`, `src/ipc.rs`, `src/select.rs`, `src/shelf/mod.rs`
- `single_element_loop` in `src/paths.rs:23`
- `too_many_arguments` in `src/shelf/paint.rs`
- `needless_return` in `src/main.rs:249`, `src/main.rs:257`

### M1 — X11 `--no-copy` ohne Output kopiert weiterhin

`decide_post_capture` routet Nicht-Wayland ohne Output weiterhin zu `PostCapture::CopyOnly` (`src/main.rs:137-140`), und `CopyOnly` kopiert immer (`src/main.rs:330-331`). `--no-copy` wird dort ignoriert.

### M2 — IPC/Daemon-DoS-Risiko weiterhin offen

`read_frame` allokiert weiter direkt aus ungeprüften Peer-Längen (`src/ipc.rs:25-33`). `handle_client` läuft weiter synchron im Accept-Loop (`src/shelf/mod.rs:279`) und liest mit 5s Timeout (`src/shelf/mod.rs:356`). Keine Caps, keine Oversized-/Truncated-Tests.

### M3 — „RAM-only“ stimmt weiterhin nicht

`add_png` schreibt weiterhin jede Aufnahme als volle PNG-Datei in ein Tempfile (`src/shelf/mod.rs:410-411`). Cleanup gibt es nur beim expliziten Close (`src/shelf/mod.rs:497`), nicht beim Daemon-Ende. README sagt weiterhin „RAM-only“ (`README.md:171`).

### M4 — Shelf-Layer-Recreate ist weiterhin protokoll-riskant

`place_on_focused_output` erstellt/committed eine neue Shelf-Layer-Surface (`src/shelf/mod.rs:323-333`). `add_png` ruft direkt danach `self.draw(qh)` auf (`src/shelf/mod.rs:423`). Wenn die Surface neu ist, kann das weiterhin vor dem ersten Configure/Ack einen Buffer attachen.

Teilfix: Der Shelf-Code fällt jetzt auf einen konkreten Output zurück, wenn der fokussierte Monitor nicht resolved (`src/shelf/mod.rs:312-328`). Das löst aber nicht das initial-configure-Problem.

### M5 — Routing-Tests fehlen weiterhin

`main.rs` hat weiter nur Parser-Tests (`src/main.rs:392-435`), keine Tests für `decide_post_capture`: Wayland default→Shelf, `--copy`, X11 `--no-copy`, `--edit`, `-o`, stdout.

### M6 — `shelf/mod.rs` ist noch größer geworden

`src/shelf/mod.rs` hat jetzt 1051 Zeilen (`wc -l`). Es mischt Daemon-Startup, IPC, Shelf-Layer, Preview-Layer, Pointer, Keyboard, DnD, Editor-Spawn und Rendering-Orchestrierung. Der riskanteste Code ist damit noch schwerer zu reviewen.

## Neue Findings aus dem Redesign

### N1 — Preview-Lightbox nutzt wieder `output = None` und kann auf Hyprland unsichtbar sein

Der Shelf-Code erklärt beim normalen Shelf-Fallback selbst, dass Hyprland eine Layer-Surface mit Null-Output nicht zuverlässig mappt: „Hyprland never maps a layer surface created with a null output“ (`src/shelf/mod.rs:312-317`). Die neue Preview erstellt ihre Layer-Surface aber mit `None` (`src/shelf/mod.rs:458-463`).

Auswirkung: Left-click kann scheinbar „nichts tun“ oder die Preview auf dem falschen Output öffnen. Das passt sehr gut zu „behaves komisch“.

Empfehlung:
- Preview denselben konkreten Output wie die Shelf verwenden lassen.
- Oder ein eigenes `focused_output()`-Helper extrahieren und für beide Surfaces nutzen.
- Test/Manual: `WAYLAND_DEBUG=1 boltsnap daemon`, dann left-click Preview auf Hyprland mit mehreren Monitoren.

### N2 — Preview-Spec sagt X/Edit-Buttons, Code implementiert nur „click anywhere closes“

Redesign-Spec: „X and edit buttons remain visible inside it“ (`docs/superpowers/specs/2026-05-31-shelf-redesign-design.md:46-47`) und X/Edit sollen Aktionen ausführen (`…:112-121`). Code rendert nur Backdrop + Bild (`src/shelf/preview.rs:34-73`), `PreviewState` speichert nur Surface/Pool/Image (`src/shelf/mod.rs:102-106`) und Pointer auf der Preview schließt bei jedem Press (`src/shelf/mod.rs:771-779`). Kein Button-Hit-Test, kein Card-ID/Path in `PreviewState`, kein Edit/Delete aus der Preview.

Auswirkung: Nutzer erwartet sichtbare/bedienbare X/Edit-Controls in der Preview, bekommt aber nur sofortiges Schließen.

Empfehlung:
- Entweder Spec vereinfachen: Preview ist reine Lightbox, Klick/Esc schließen.
- Oder PreviewState um `id/path`, Button-Rects und `render_lightbox_with_controls` erweitern.

### N3 — `LayerShellHandler::closed` beendet den ganzen Daemon für jede Layer-Surface

`closed` setzt pauschal `self.exit = true` (`src/shelf/mod.rs:673-674`). Seit dem Redesign gibt es mindestens zwei Layer-Surfaces: Shelf und Preview. Wenn der Compositor die Preview-Surface schließt, kann der komplette Shelf-Daemon beendet werden.

Empfehlung:
- In `closed` zwischen Shelf- und Preview-Surface unterscheiden.
- Preview-close sollte nur `self.preview = None` setzen; nur die Shelf-Hauptsurface darf den Daemon beenden.

### N4 — Drag-Icon nutzt die gecroppte Karte, nicht die volle Screenshot-Ansicht

Die Spec verspricht „actual screenshot stuck to the cursor“ und „drops the full image“ (`docs/superpowers/specs/2026-05-31-shelf-redesign-design.md:27-28`, `…:159-162`). Der Code baut das Drag-Icon aus `t.thumb` (`src/shelf/mod.rs:535-537`), also aus dem 260×180 Cover-Crop. Bei extrem breiten/hohen Screenshots zeigt der Drag-Cursor nur den Ausschnitt, nicht die volle Bildform.

Auswirkung: Drag kann „falsch“ wirken, weil der Cursor nicht das erwartete vollständige Bild repräsentiert.

Empfehlung:
- Wenn die UX „echtes vollständiges Bild“ meint: aus `png_path` laden und fit-within auf ein Max-Icon rendern.
- Wenn bewusst nur Card-Crop: README/Spec entsprechend präzisieren.

### N5 — README ist jetzt stärker stale als vorher

README sagt weiterhin:

- `boltsnap` / `boltsnap full` kopieren (`README.md:84`, `README.md:89`)
- Thumbnail-Click kopiert (`README.md:153`)
- Hover zeigt ✎, ⧉, ✕ (`README.md:158-159`)

Aktueller Redesign-Code/Spec: left-click opens preview, right-click copies, hover hat nur X/Edit (`docs/superpowers/specs/2026-05-31-shelf-redesign-design.md:25`, `…:45-47`; `src/shelf/layout.rs:27-31`).

Auswirkung: Jeder manuelle Test nach README testet das falsche Verhalten.

### N6 — `doctor`/layer-shell-Fähigkeit weiterhin nicht geprüft

Spec alt fordert `wlr-layer-shell`-Check; `print_doctor` meldet weiterhin nur Wayland-Session, Daemon und Socket (`src/paths.rs:39-50`). Für einen Feature-Branch, der komplett an layer-shell hängt, ist das als Diagnose zu dünn.

### N7 — `Layout` ist weiterhin generisch statt fixed-card-enforced

Redesign-Spec sagt: `ThumbRect` ist `CARD_W × CARD_H`, keine `widest thumb`-Logik mehr (`docs/superpowers/specs/2026-05-31-shelf-redesign-design.md:65-66`). Code behält `sizes`/`widest` (`src/shelf/layout.rs:46`, `src/shelf/layout.rs:65`) und der Test nutzt sogar alte variable Größen (`src/shelf/layout.rs:125`). Runtime funktioniert, weil Thumbnails jetzt 260×180 sind; als Design-Invariant ist es aber nicht im Layout verankert.

## Weird-Behavior-Verdachtsliste

Priorität für Debugging:

1. **Preview nicht sichtbar/falscher Monitor:** `output = None` in `open_preview`.
2. **Preview schließt unerwartet:** Jeder Pointer-Press auf Preview schließt sofort; keine Buttons/Hit-Zonen.
3. **Daemon stirbt beim Preview-Lifecycle:** `closed` unterscheidet Preview nicht von Shelf.
4. **Drag sieht falsch/cropped aus:** Drag-Icon kommt aus `t.thumb`.
5. **Monitorwechsel-Shelf crash/verschwindet:** `add_png` zeichnet direkt nach Surface-Recreate.
6. **CLI wirkt falsch:** README/usage behauptet Copy-Verhalten, Code nutzt Shelf/Preview/Right-click.

## Empfohlene nächste Fix-Reihenfolge

1. **Sofort:** Preview `output` konkret setzen, nicht `None`.
2. `closed` handler surface-aware machen.
3. Preview-Spec entscheiden: reine Click-to-close-Lightbox oder echte X/Edit-Controls; Code/Docs angleichen.
4. Alte Blocker B1/B2 fixen: `--edit -o` Tempfile-Flow, dann `cargo fmt` + Clippy.
5. X11 `--no-copy` und Routing-Tests ergänzen.
6. IPC-Length-Caps + Daemon-Blocking entschärfen.
7. Tempfile/RAM-only ehrlich lösen.
8. README/usage komplett aktualisieren.
9. Danach erst weiter an Styling feilen; sonst wird gegen falsche/instabile Semantik poliert.

## Manuelle Validierung nach Fixes

- Multi-Monitor Hyprland: left-click Preview erscheint auf dem aktiven/fokussierten Monitor.
- Preview: Esc schließt; Klick schließt; falls X/Edit gewünscht, X löscht und Edit öffnet Editor.
- Drag: Cursor-Icon entspricht der final gewünschten UX (Crop-Card vs vollständiges Bild bewusst entscheiden).
- Nach Preview schließen: Daemon bleibt aktiv, neue Captures funktionieren.
- Monitorwechsel + Capture: keine Wayland-Protokollfehler.
- `boltsnap --backend x11 --no-copy`: keine Clipboard-Mutation.
- `boltsnap area --edit -o existing.png`, Editor abbrechen: `existing.png` bleibt unverändert.
