# Selector: Leiste und Auswahl nach dem `ui`-Skill (2026-10-27)

Auftrag des Nutzers (27.10.): „das selector/clipping UI also das was man sieht
nach dem bereich auswaehlen etwas schoener machen und schauen das alles sich
auch an /ui haelt“. Gemeint ist der Aufnahme-Selector (`boltsnap record`) nach
dem Aufziehen: Rahmen, Griffe und die Leiste mit REC, Clip, Frame, Audio.

## 1. Befund (gemessen am Offscreen-Render, 1920×1080, Schrift des Nutzers)

Render: `render.rs` → `scene::paint` mit `SCREENSHOT_OVERLAY_STYLE` (so
zeichnet Linux, `select_skia/mod.rs:594`), Zustände Ruhe, Hover REC, Hover
Audio (aus), Clip nicht verfügbar.

| # | Befund | Regel | Beleg |
| --- | --- | --- | --- |
| B1 | Leiste mit 1 px Innenrand: die Flächen der Controls stoßen fast an den Rand, oben/unten 1 px, zwischen den Controls 4 px | gleicher Randabstand, konzentrische Rundung mit sichtbarem Padding | `render.rs:832` `TB_PAD = 1.0`, `TB_GAP = 4.0` |
| B2 | 21-px-Schrift in 24 px hohen Controls: Füllgrad über 85 %, Control-Höhe ≈ 1,1× Schrift statt ≈ 2× | Füllgrad, Control-Höhe | `render.rs:497-498` |
| B3 | Eigene Skala neben der des Settings-Fensters: 21, 17, 13 px, Radius 6/8/9, Icon 18, Farben `0x18/0xEE/0x99/0x88` | Skalen zentral, ein System je Projekt | `render.rs:489-504`, `708-722` vs. `ui/theme.rs` |
| B4 | Zustand über Schriftgewicht: aktive Frame/Audio-Labels in DemiBold (Breite ist reserviert, Gewicht springt trotzdem hart) | kein harter Zustandswechsel, Zustand über Helligkeit/Fläche | `render.rs:975-980` |
| B5 | Kein Zustandswechsel animiert: Hover, Press, Umschalten, Erscheinen der Leiste schalten hart | nur Federn | `scene.rs:155` (kein Zeitparameter) |
| B6 | Rahmen 1,5 px weiß: liegt auf halben Pixeln und verschmiert über zwei Reihen | Pixelraster | `render.rs:187` `border_width: 1.5` |
| B7 | „REC“ als Label, Versalien-Kürzel neben „Clip“, „Frame“, „Audio“ | Texte, Voice | `render.rs:838` |
| B8 | Clip ausgegraut ohne Grund und ohne Weg dorthin (Replay aus/nicht bereit) | keine Sackgassen | `render.rs:951`, `select_skia/mod.rs:594` |
| B9 | Icons aus eigenem Pfadsatz (`Icon::Record/Cut/Crop/Volume`), 2,4er Strich, nicht das Lucide-Set des Projekts | ein Icon-Set | `render.rs:536-650` vs. `assets/icons/ui/lucide.svg` |
| B10 | `render.rs` hat 1730 Zeilen, `select_skia/mod.rs` 1251 | ≤ 1000 Zeilen je Datei | `wc -l` |

Gut und bleibt: eine primäre Aktion (REC mit ruhender Fläche und der einzigen
Farbe), gleiche Breite der Controls über alle Zustände, Fallback auf 2×2 und
eine Spalte bei kleinen Flächen, Leiste über, sonst unter, sonst in der Auswahl.

## 2. Richtung (entschieden 27.10.: das Designsystem der App)

Die Leiste ahmt bisher die Module der Desktop-Leiste nach (Kommentare in
`render.rs:946-976`, Schrift aus `ui_font` in 500/600). Seitdem hat Boltsnap
ein eigenes Designsystem (`src/ui/theme.rs`: Farben, Abstände, `CTL_H`,
Radien, `TextStyle` mit Versalhöhen auf ganzen Pixeln, Lucide-Icons, drei
Federn), das der Nutzer im Settings-Fenster abgenommen hat.

Nutzer, 27.10.: „das tut es gerade eher nicht es hat so einen eigenen look“,
die Leiste wirkt also schon jetzt nicht wie die Desktop-Leiste. Damit gilt die
Empfehlung:

**Entscheidung:** Selector-Chrome (Leiste, Größen-Badge, Rahmen, Griffe) nutzt
dieses System. Grund: Shelf-Vorschau, Settings und Selector sind ein Produkt;
zwei Skalen nebeneinander sind B3. Alternative: Desktop-Leisten-Look behalten
und nur B1/B4–B8 darin beheben.

## 3. Verhalten (Soll)

- **Leiste:** Fläche `PANEL`, Innenrand `GAP_XS` rundum, Controls `CTL_H` hoch,
  Radius außen `RADIUS`, innen `RADIUS − GAP_XS` (konzentrisch). Label in
  `TextStyle::Label`, Icon `ICON` aus dem Lucide-Sprite, Tinte mittig auf den
  Versalien, ein `GAP_S` zwischen Icon und Text, horizontaler Rand der
  Controls `(CTL_H − cap) / 2`.
- **Labels:** „Record“, „Clip“, „Frame“, „Audio“. Icons: `circle` gefüllt in
  `ERROR` für Record (einzige Farbe), `scissors` für Clip, `square-dashed` für
  Frame, `volume-2` / `volume-x` für Audio. `scissors` und `volume-x` kommen
  neu aus Lucide (Pfade aus dem Lucide-Repo, gleiche Lizenz).
- **Zustände:** Record ruht auf `RAISE`, Hover `HOVER`. Frame und Audio an:
  Fläche `SELECTED`, Text `TEXT`; aus: keine Fläche, Text `SUB`, Icon gedimmt.
  Kein Gewichtswechsel. Hover nicht gewählter Controls: Text `TEXT`.
  Press: Fläche eine Stufe dunkler, solange gedrückt.
- **Clip nicht verfügbar:** Text `DIM`; Hover zeigt unter der Leiste einen
  kurzen Hinweis mit dem Grund aus dem Worker-Status („Replay is off“ bzw.
  „Replay is starting“), keinen Absatz.
- **Motion:** Hover, Press, An/Aus und der Hinweis blenden mit `QUICK`.
  Die Leiste blendet beim Loslassen der fertigen Auswahl mit `QUICK` ein; beim
  Ziehen folgt sie der Auswahl direkt (direkte Manipulation, kein Gleiten).
  Wechsel zwischen Leisten-Layouts (Streifen, 2×2, Spalte) und zwischen
  über/unter der Auswahl: blenden statt wandern. Reduced Motion: Werte sofort.
- **Rahmen und Griffe:** 1 px weiß auf ganzen Pixeln mit 1 px dunkler Kontur
  außen; Griffe 8 × 8 auf ganzen Pixeln, mittig auf Ecken und Kanten.
- **Größen-Badge (Screenshot-Modus):** gleiche Fläche, Schrift `Label`,
  `tabular`; gleicher Abstand zur Auswahl wie die Leiste.
- **Windows** behält Aussehen und Code (`WINDOWS_RECORD_OVERLAY_STYLE`,
  `draw_rec_pill`, `draw_record_audio_button`, `draw_badge` für Windows): das
  Backend ist eingefroren. Linux bekommt einen eigenen Stil.

**Vorschläge, nur mit Okay des Nutzers (neue Funktion; am 27.10. gefragt, nicht
beantwortet, die Antwort „Maße“ meinte die Shelf-Vorschau, also nicht gebaut):**
- V-a: Maße der Auswahl („1450 × 942“, `Key`, `SUB`, Breite für „0000 × 0000“
  reserviert) rechts in der Leiste, ohne Control.
- V-b: der Hinweis bei nicht verfügbarem Clip (oben) bietet „Start replay“ als
  Aktion an.

**Macht nicht:** andere Bedienung oder Tastenkürzel, Lupe (bleibt), den
Screenshot-Selector-Ablauf, Windows, die REC-Pille während der Aufnahme.

## 4. Umsetzung

Reihenfolge, je Schritt ein Commit, Verhalten nach jedem Schritt unverändert
lauffähig:

1. **S0 Aufteilen ohne Änderung** (B10): `src/selector/render.rs` wird
   `render/mod.rs` (Overlay, Rahmen, Griffe, Lupe, Badge), `render/toolbar.rs`
   (Leiste: Layout, Zeichnen, Treffer), `render/legacy.rs` (Windows-only:
   REC-Pille, Audio-Knopf, Checkbox, alte Icons), `render/tests.rs`.
   `select_skia/mod.rs`: Pointer- und Keyboard-Handler nach `input.rs`.
   Abnahme: `cargo test`, Pixelgleichheit der Offscreen-Renders vorher/nachher.
2. **S1 Tokens und Icons:** Leiste und Badge auf `ui::theme`, `ui::text::Fonts`
   (`TextStyle::Label`), `ui::icons` (+ `scissors`, `volume-x`). Linux-Stil für
   Rahmen und Griffe. Trefferflächen bleiben die Control-Rechtecke
   (`record_control_hit`, `select_skia/mod.rs:443`).
3. **S2 Federn:** `Scene` bekommt Zeit und `ui::motion::Spring` je Control
   (Hover, Press, An) plus Leisten-Deckkraft; `SceneCache::update` vergleicht
   Ziele, nicht Frames; `select_skia` fordert Frame-Callbacks an, solange eine
   Feder läuft (Mechanismus existiert: `request_redraw`, `frame` bei
   `mod.rs:530/745`), Damage nur die Leiste.
4. **S3 Clip-Grund** (+ V-a/V-b, falls freigegeben): Status kommt schon über
   `FrozenSelection::prepare` bzw. `preview_target` (`replay/mod.rs:173`);
   der Grund wird ins `Scene` gereicht.

Betroffene Dateien: `src/selector/{render/*,scene.rs,edit.rs}`,
`src/platform/linux/select_skia/{mod.rs,input.rs}`, `src/ui/icons.rs`,
`assets/icons/ui/lucide.svg`, `docs/codebase.md`. Nicht:
`src/platform/windows/*`.

Was kaputtgehen kann: Pixel des Windows-Selectors (Gegenmaßnahme: Windows-Stil
und -Funktionen unverändert in `legacy.rs`, Test auf identische Pixel);
Trefferflächen gegen Zeichnung (Test: jede gezeichnete Control-Fläche ist ihre
Trefferfläche); Damage bei laufenden Federn (Test: Damage bleibt in der
Leiste); Leistung beim Ziehen großer Auswahlen (Damage-Strips bleiben).

## 4a. Umsetzung im Detail (geschrieben 27.10. vor dem Code)

Belege: `ui::text::Fonts` (`src/ui/text.rs`: `size`, `width`, `baseline`,
`draw`, `ellipsize`), `ui::Canvas` (`src/ui/mod.rs:171`, `fill`, Feld
`pixmap`), `ui::icons::Icon::draw` (`src/ui/icons.rs`), `ui::motion::Spring`
(`retarget`, `value`, `done`, `settled`), Tokens `src/ui/theme.rs`
(`PANEL`, `RAISE`, `HOVER`, `SELECTED`, `TEXT`, `SUB`, `DIM`, `ERROR`,
`WINDOW`, `GAP_XS/S/M`, `CTL_H`, `RADIUS`, `ICON`, `QUICK`), Gruppen-Deckkraft
über `Pixmap::draw_pixmap` mit `PixmapPaint { opacity }` (tiny-skia 0.11,
`tiny-skia-0.11.4/src/painter.rs:150`, `shaders/pattern.rs:32`), leerer Commit mit
Frame-Callback während einer Animation wie `linux/app/window.rs:306`.
Selector und `ui` liegen in verschiedenen Crates (Binary `boltsnap`, Lib
`boltsnap::ui`), der Selector benutzt `boltsnap::ui::…` wie
`linux/app/mod.rs:16`.

**S0, reine Aufteilung (keine Pixeländerung):**
- `src/selector/render.rs` → `render/mod.rs` (Overlay, Cache, Rahmen,
  Griffe, Schrift-Helfer, Badge, Lupe), `render/toolbar.rs`
  (`record_toolbar`, `draw_record_toolbar`, die eigenen Icons),
  `render/legacy.rs` (nur Windows: `rec_pill_rect`, `draw_rec_pill`,
  `record_frame_checkbox_rect`, `draw_record_frame_checkbox`,
  `record_audio_button_rect`, `draw_record_audio_button`), `render/tests.rs`.
  Öffentliche Pfade bleiben über `pub use` gleich, Windows ändert sich nicht.
- `select_skia/mod.rs` → `input.rs` (Pointer, Tastatur, Seat,
  `record_control_hit`, `Interaction`, Konstanten, Test) und `present.rs`
  (`draw`, `FrameBuffer`, `OutputView`, `CompositorHandler`).
  **Abweichung (freigegeben 27.10.):** `present.rs` zusätzlich, sonst bliebe
  `mod.rs` bei etwa 970 Zeilen.
- Neuer ignorierter Test `select_skia/shots.rs::selector_shots`
  (`SEL_OUT=dir cargo test --bin boltsnap selector_shots -- --ignored`):
  Schrift des Nutzers, Hintergrund `SEL_BASE` (sonst ein Verlauf), Szenen
  Ruhe, Hover je Control, Clip nicht verfügbar, 2×2 bei 400×300, Spalte bei
  160×240, Auswahl oben (Leiste unten), bildschirmfüllend (Leiste innen),
  Screenshot-Modus mit Badge und Griffen; ab S2 auch Zwischenbilder.
- Neuer Test `render/tests.rs::windows_pixels_stay_pinned`: Hash über die
  Pixel aller Windows-Funktionen (Pille, Audio-Knopf, Badge, Rahmen und
  Griffe in beiden Windows-Stilen, Lupe) mit der eingebetteten Schrift; der
  Wert stammt aus dem Stand vor S0. **Abweichung (freigegeben):** statt
  „Test auf identische Pixel“ gegen eine Kopie ein Hash, der S0–S3 überdauert.
- Abnahme S0: Renders vor und nach der Aufteilung byte-gleich (`cmp`).

**S1, Tokens und Icons:**
- Schrift: `install_ui_font` wird `ui_fonts() -> Fonts` (gleicher Lader
  `load_ui_font_weights`, 500/600), im Font-Thread geladen, Feld
  `Selector::fonts`. `render::set_ui_font` entfällt; das alte `ui_font` liefert
  nur noch die eingebettete Schrift für `legacy.rs` (Windows hat sie nie
  gesetzt, Pixel gleich).
- `CachedOverlay::frame` wird `ui::Canvas` (Skala 1); `scene::paint(frame:
  &mut Canvas, base, scene, fonts)`. Die Windows-Funktionen bekommen weiter
  `&mut Pixmap` (`&mut canvas.pixmap`).
- `toolbar.rs` neu für Linux: `Metrics::new(&Fonts)` (Breite je Control
  einmal), `layout(sel, area, &Metrics) -> Option<Layout>` mit `columns`,
  `placement` (Above/Below/Inside), `bounds`, `controls`, `hits`. Maße:
  Fläche `PANEL`, Innenrand `GAP_XS`, Controls `CTL_H`, Abstand zwischen
  Controls `GAP_XS`, Radius außen `RADIUS`, innen `RADIUS − GAP_XS`; Control
  = `(CTL_H − cap)/2` + `ICON` + `GAP_S` + Label + `(CTL_H − cap)/2`. Lage wie
  bisher (über, sonst unter, sonst in der Auswahl; `GAP_M` Abstand zur
  Auswahl, `GAP_S` zum Bildschirmrand), auf ganze Pixel gerundet. Treffer:
  Control-Rechteck plus halber Abstand zum Nachbarn und der Innenrand zum
  Leistenrand, also keine Totzone. **Abweichung (freigegeben):** Treffer
  größer als die Fläche, „Listen ohne Totzonen“ des `/ui`-Skills.
- Zustände über eine Leiter `PANEL`(keine Fläche) → `RAISE` → `HOVER` →
  `SELECTED`: Stufe = Ruhe + Hover + Druck, höchstens `SELECTED`. Ruhe:
  Record 1, Clip 0, Frame/Audio aus 0, an 2. Text: Record/Clip `TEXT`,
  Frame/Audio `mix(SUB, TEXT, max(an, hover))`, Clip nicht verfügbar `DIM`.
  **Abweichungen (freigegeben):** „an“ ist `HOVER` statt `SELECTED` (sonst
  hätte Hover auf einem eingeschalteten Control keine Stufe mehr, gleicher
  Abstand von zwei Stufen wie die gewählte Segment-Fläche auf ihrer Spur);
  Druck ist eine Stufe heller wie `Look::surface` im Settings-Fenster.
- Icons: `circle` (gefüllt, `ERROR`), `scissors`, `square-dashed`,
  `volume-2`/`volume-x`. Neu im Sprite `circle`, `scissors`, `volume-x`,
  unverändert aus `lucide-static` 1.54.0 `sprite.svg` (gleiche Version wie der
  Sprite, gefunden unter `~/projects/stepi/node_modules/lucide-static`).
  `Icon::fill` neu (gefüllter Pfad) für den Aufnahmepunkt.
- Rahmen (Linux, beide Modi): gerundetes Crop-Rechteck `X0 = round(x)`,
  `X1 = round(x + w)`; weißer Ring 1 px direkt außen, Kontur 1 px `WINDOW`
  außen daran, beides deckend ohne Kantenglättung. Griffe 8 × 8 weiß mit
  1 px `WINDOW`-Kontur, Mitte auf der Crop-Kante bzw. `round` der Kantenmitte,
  ohne Rundung. Deckend und auf ganzen Pixeln heißt: zweimal zeichnen ändert
  nichts (Voraussetzung für Damage nur in der Leiste, S2).
- Badge (Screenshot-Modus, Linux): Fläche `PANEL`, `CTL_H` hoch, Radius
  `RADIUS`, Text `Label` in `TEXT`, „1450 × 942“ mit geschützten Leerzeichen,
  Ziffern in gleich breiten Zellen (tabular von Hand, ab_glyph kennt kein
  `tnum`); Lage wie die Leiste (`GAP_M` zur Auswahl) im Bereich des Monitors
  unter dem Zeiger (`chrome_viewport`).
- Windows: `legacy.rs`, `WINDOWS_RECORD_OVERLAY_STYLE`,
  `SCREENSHOT_OVERLAY_STYLE` und `*_with_style` unverändert; der Test
  „Windows-Screenshot gleich Linux“ entfällt, weil Linux jetzt anders zeichnet,
  die Windows-Pixel hält der Hash-Test.

**S2, Federn:**
- `src/selector/motion.rs`: `BarMotion` mit `QUICK`-Federn je Control
  (hover, press, on, enabled), Deckkraft der Leiste, Deckkraft des Hinweises,
  dazu bis zu vier ausblendende alte Leisten (`ghosts`: eingefrorenes Layout
  und Aussehen). `update(layout, targets, now)`; `sample(now) -> Bar`;
  `moving(now)`.
- Leiste blendet ein, sobald die erste Auswahl losgelassen ist (Modus
  Editing), aus beim neuen Aufziehen. Beim Ändern der Auswahl folgt sie direkt.
  Wechselt `columns` oder `placement`, blendet die alte an ihrer letzten Stelle
  aus und die neue ein (beide `QUICK`); nur die aktuelle nimmt Eingaben.
- Gruppen-Deckkraft: jede Leiste wird in eine eigene kleine Pixmap
  gezeichnet und mit `opacity` aufgelegt, damit innere Flächen beim Blenden
  nicht durchscheinen.
- `Scene` bekommt `bar: Bar` (gesampelte Werte) statt `toggles`/`hovered`.
  `SceneCache::update`: ist alles außer `bar` gleich, sind die Damage-Regionen
  nur alte und neue Leisten-, Geister- und Hinweisflächen; gezeichnet wird
  trotzdem die ganze Szene (Rahmen und Griffe sind idempotent, siehe S1).
- `select_skia`: `Instant::now()` je Zeichnen; läuft eine Feder, bleibt
  `needs_redraw` gesetzt; hat ein Output dann keinen neuen Puffer bekommen,
  geht ein leerer Commit mit Frame-Callback raus (wie das Settings-Fenster).
  Zeiger verlässt die Fläche (`Leave`): Hover und Druck aus.
- **Reduced Motion (Abweichung, freigegeben):** wie `App::reduced_motion`:
  Blenden bleiben, nur räumliche Bewegung entfällt; der Selector hat keine
  räumliche Bewegung, also kein eigener Schalter. Die Plattform meldet ohnehin
  keine Einstellung (Settings-Plan Phase 4b).
- Ohne Animation: das Mitziehen der Leiste mit der Auswahl (direkte
  Manipulation), Rahmen und Griffe.

**S3, Clip-Grund:**
- `selector::clip::ClipState` (plattformneutral): `Ready`, `Off`, `Starting`,
  `Loading`, `Busy`, `Stalled`, `Failed`; Texte „Replay is off“, „Replay is
  starting“, „Replay is loading“, „Replay is busy“, „Replay has no video“,
  „Replay could not load“. **Abweichung (freigegeben):** sechs Gründe statt
  zwei, jeder ein kurzer Satz ohne Punkt, durch den Copy-Gate.
- Quelle: `replay::preview_target()` liefert `Result<PreviewTarget,
  ClipState>` aus `status_quiet` (`state` stopped/starting, `ready`, `busy`;
  keine Antwort heißt Off). Mit Ziel startet der Selector in `Loading`;
  `FrozenSelection::prepare` → `Ready`, scheitert es → `Failed` (bisher blieb
  der Zustand stumm).
- Hinweis: Fläche `PANEL`, `CTL_H`, Radius `RADIUS`, Text `Label` in `SUB`,
  links bündig mit dem Clip-Control, `GAP_XS` neben der Leiste auf der Seite
  weg von der Auswahl (Leiste darüber: Hinweis darüber; darunter oder innen:
  darunter), sonst die andere Seite; immer im Bildschirm, zu lang wird er
  mit … gekürzt. **Abweichung (freigegeben):** nicht immer „unter“, damit er
  nie die Auswahl verdeckt. Sichtbar, solange der Zeiger auf dem nicht
  verfügbaren Clip liegt; blendet mit `QUICK`. Wird Clip verfügbar, blendet
  der Hinweis aus und Clip hellt auf. Die Meldung beim Drücken bleibt.
- Kein „Start replay“ (V-b), keine Maße (V-a).

**Randfälle:** Streifen/2×2/Spalte nach Bildschirmgröße; Leiste über, unter,
in der Auswahl; winzige Auswahl (Leiste bleibt im Bildschirm, Test);
Auswahl am Bildschirmrand (Rahmen teils außerhalb, `fill_rect` schneidet);
mehrere Monitore nur im Screenshot-Modus (Badge im Monitor unter dem Zeiger),
die Aufnahme hat genau ein Output; HiDPI: der Selector zeichnet in logischen
Pixeln ohne Buffer-Scale (unverändert), ganze Pixel gelten dort.

**Macht nicht (zusätzlich):** HiDPI-Puffer, Tastatur-Fokus in der Leiste
(gibt es heute nicht), Hysterese beim Layoutwechsel.

## 5. Abnahme (Gates mit Zahlen)

Offscreen über einen ignorierten Test, der `scene::paint` auf ein Bild rendert
(`SEL_OUT=dir cargo test --bin boltsnap selector_shots -- --ignored`):
Streifen bei 1920×1080, 2×2 bei 300×300 (Abweichung: die neue Leiste ist schmaler, bei 400×300 passt schon der Streifen), Spalte bei 160×240, Auswahl am
oberen Rand (Leiste unten) und bildschirmfüllend (Leiste innen).

- Gate 1 Überlauf: Leiste und Hinweis immer in der Fläche (bestehender Test
  `record_toolbar_keeps_controls_together_at_edges` erweitert um den Hinweis).
- Gate 2 Versalien je Control `|off| ≤ 0,5 px` nach Tinte.
- Gate 3 Innenrand der Leiste rundum gleich (± 0,5 px), alle Controls gleich hoch.
- Gate 4 Kontrast: Label auf jeder Control-Fläche ≥ 4,5:1, `SUB` ≥ 4,5:1.
- Gate 5 Motion-Liste: Hover, Press, An/Aus, Leiste, Hinweis; Retarget-Test,
  Reduced Motion.
- Gate 6 Copy-Check über die neuen Strings.
- Gate 7 Screenshot im Dark Theme angesehen.
- Gate 8 Icon-Tinte mittig auf den Versalien, gemeinsame Abstände Icon→Text.
- Live (nur mit Okay): `boltsnap record`, Bereich ziehen, Hover, Frame/Audio,
  Clip bei laufendem Replay.

## 6. Stand

| Schritt | Stand | Checks |
| --- | --- | --- |
| Befund, Plan | geschrieben 27.10. | Offscreen-Render von vier Zuständen |
| Richtung | entschieden 27.10., Abschnitt 2 | |
| Detailplan | 27.10., Abschnitt 4a, Abweichungen freigegeben | |
| S0 | umgesetzt 27.10.: `selector/render/{mod,toolbar,legacy,tests}.rs` (721/287/301/~490 Zeilen), `select_skia/{mod,input,present}.rs` (611/430/240), `select_skia/shots.rs` (ignorierter `selector_shots`), `windows_pixels_stay_pinned` (Hash `0x34ed7442fdc3e91d` vom Stand vor der Aufteilung), `examples/selector_bench.rs` auf den neuen Pfad | 14 Offscreen-Renders vor/nach der Aufteilung byte-gleich (`cmp`); `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 133 + 376 grün (4 ignoriert), `git diff --check`. Kein Windows-Target installiert: keine `cargo check --target x86_64-pc-windows-msvc`, Windows-Code unverändert |
| S1 | umgesetzt 27.10.: `render/chrome.rs` (Rahmen 1 px weiß + 1 px `WINDOW` außen am gerundeten Crop, Griffe 8 × 8 deckend, Badge `PANEL`/`CTL_H`/`Label` mit Ziffern in gleich breiten Zellen, gemeinsame Lage `place`), `render/toolbar.rs` neu (`Metrics`, `layout` mit Treffern ohne Totzone, Leiter `PANEL→RAISE→HOVER→SELECTED`, Lucide-Icons), `render/gates.rs` (Messung auf den Pixeln), `render/chrome_tests.rs`; Windows-Teile (Badge, Rahmen/Griffe mit Stil, Lupe mit Bildschirmplatzierung, alte Schrift) nach `legacy.rs`; `CachedOverlay` zeichnet auf `ui::Canvas`; `select_skia::ui_fonts()` statt `install_ui_font`, `Selector::chrome`; Sprite + `circle`, `scissors`, `volume-x` aus `lucide-static` 1.54.0, `Icon::fill`. Test „Windows-Screenshot gleich Linux“ entfernt (Linux zeichnet jetzt anders), `windows_pixels_stay_pinned` unverändert grün | Gates mit eingebetteter Schrift (`cargo test`) und der des Nutzers (`selector_shots`): Überlauf 35 Lagen auf 5 Bildschirmgrößen im Bildschirm; Versalien schlechtester Versatz 0,03 px (Nutzer 0,01); Icon auf Versalien 0,01 (Nutzer 0,00); Innenrand 4,0/4,0/4,0/4,0 in Streifen, 2×2, Spalte; Icon-Achse je Spalte 0,34, Textkante 0,00, breitestes Icon → Text `GAP_S` + 0,50; Kontrast 6 Textpaare, niedrigstes 5,38:1, Aufnahmepunkt 3,87:1 (Ruhe) / 3,47:1 (Hover); Copy 4 Labels; Gegenprobe 1 px Versatz schlägt an. Renders unter `scratchpad/sel/s1/` angesehen. `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 133 + 381 grün (4 ignoriert), `git diff --check`. Kein Windows-Target installiert |
| S2–S3 | offen | |
