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

## 4b. S4 Variante A und S5 Squircle (27.10., nach dem Review)

Nutzer zur Leiste aus S1–S3: „geht so vom design her … langweilig“; aus
drei Entwürfen gewählt: **A** (`scratchpad/sel/mock/zA.png`), dazu: „mach
mit hover namen und die kanten und so wirken nicht squircle und etwas
unclean … mathematisch alles gestützt“. Reihenfolge: Plan, S5 (Squircle,
gemeinsam im `ui`, Plan dort: `docs/plans/settings/2026-10-23-settings-ui.md`
„Phase 5f“), dann S4, je ein Commit.

**S5** siehe Settings-Plan Phase 5f: Ecken als Superellipse n = 4 mit
R = r(1 − 1/√2)/(1 − 2^(−1/4)) ≈ 1,84 r (gleicher 45°-Punkt wie der Kreis),
geklemmt auf min(w, h)/2 mit angepasstem n bis zum Kreis (n = 2).

**S4 Verhalten:**
- Eine Leiste, Gruppen mit `GAP_S` dazwischen, `GAP_XS` innerhalb:
  [Record, Clip] · [Frame, Audio] · Maße.
- Record ist die eine primäre Aktion: Fläche `ACCENT`, Label `ON_ACCENT`,
  `circle` gefüllt in `ERROR`. Das Settings-Fenster hat keinen primären
  Knopf (`Look::surface` kennt nur `RAISE → HOVER → SELECTED`, Schalter und
  Slider nutzen `ACCENT` ohne Hover), also neue Regel, hier festgelegt:
  Hover ein Viertel Richtung `SUB`, Druck ein weiteres Viertel
  (`mix(ACCENT, SUB, 0,25 · (hover + press))`: #ECECEC → #D7D7D7 → #C3C3C3).
  `ON_ACCENT` darauf ≥ 9,6:1. Der Punkt `ERROR` auf `ACCENT` 3,2:1, auf der
  Hover-Stufe darunter (Grafik neben dem Wort „Record“, nicht allein
  bedeutungstragend; gemessen und berichtet).
- Clip wie bisher (Label, Schere, keine Fläche in Ruhe, Leiter, Grund-Hinweis).
- Frame und Audio: Icon-Controls `CTL_H × CTL_H`, Icon-Box mittig (ganze
  Pixel), Tinten-Mitte gemessen (Gate 8 ≤ 0,5 px in beiden Achsen). An:
  `HOVER`, aus: keine Fläche, Icon `SUB`; Hover wie bisher eine Stufe;
  Audio behält die Lautsprecher-Überblendung.
- Maße: „1450 × 942“ (`Key`, `SUB`) mit U+00A0 um das ×, damit Zahl und
  Zeichen eine Einheit bleiben (wie „190 px“ in der Shelf-Vorschau). Wert:
  die Ausgabegröße des Crops wie `Crop::from_selection`
  (`src/replay/crop.rs:14`) sie rechnet, aus Auswahl, Fläche (logisch) und
  Videogröße: Replay-Bild (`FrozenSelection` Breite/Höhe), sonst der
  aktuelle Modus des Ausgangs (`OutputInfo::modes`, `current`, bei 90°/270°
  vertauscht; sctk 0.19.2 `output.rs:268–368`), sonst die Fläche. Scheitert
  der Crop (zu klein), bleibt der Platz leer. Kein Control: kein Hover,
  keine Trefferfläche. Breite reserviert für den breitesten Wert, den der
  Bildschirm erlaubt (Ziffernzahl von Breite und Höhe des Videos, jede
  Ziffer in einer Zelle so breit wie die breiteste Ziffer; ab_glyph wendet
  kein `tnum` an, die Zellen ersetzen es; bei Outfit sind die Ziffern
  ohnehin gleich breit, dann ändert sich nichts), links bündig: die Leiste
  ändert beim Ziehen nie ihre Breite. `GAP_S` nach dem letzten Control;
  rechts so viel Rand, dass der Text wie ein Label endet: `SIDE + GAP_XS`
  vom Leistenrand.
- Hover-Namen für Frame und Audio: „Frame on“/„Frame off“, „Audio on“/
  „Audio off“ (Zustand im Wort). Fläche und Lage wie der Clip-Grund, bündig
  mit dem Control. Erscheinen nach 400 ms Ruhe auf dem Control (Feder mit
  Start in der Zukunft: davor exakt 0, kein Aufblitzen beim Überstreichen),
  dann `QUICK`; zum anderen Icon-Control, während ein Name zu sehen ist:
  sofort tauschen (Überblendung, keine neue Wartezeit); noch wartend:
  Wartezeit neu; Verlassen: `QUICK` aus. Wechselt der Zustand per Klick,
  blendet das Wort über. Record und Clip haben Labels, also keine Namen.
- Lagen (die erste, die mit `GAP_S` Rand passt):
  1. eine Zeile mit Maßen; 2. eine Zeile ohne Maße (die Maße fallen
  zuerst); 3. zwei Zeilen [Record, Clip] / [Frame, Audio, Maße];
  4. drei Zeilen [Record] / [Clip] / [Frame, Audio] ohne Maße.
  Zeilen links bündig; zwischen Gruppen `GAP_S`, innerhalb `GAP_XS`, auch
  senkrecht. Eine Lage mit zwei Zeilen ohne Maße wäre so breit wie 3 und
  entfällt.
- Treffer: jedes Control bis zur halben Fuge zum Nachbarn, am Rand bis zum
  Leistenrand; die Maße gehören niemandem.

**S4 Umsetzung:** `render/toolbar.rs` (Items und Zeilen statt Spalten,
`Layout::{controls, hits, readout, arrangement}`, primäre Fläche,
Icon-Controls, `hint_rect` mit Anker-Control), `render/chrome.rs` (Ziffern-
zellen gemeinsam mit dem Badge), `selector/motion.rs` (`Targets::size`,
Namen mit Verzögerung, `Bar::tips`), `scene.rs` (Damage und Zeichnen der
Namen), `select_skia/mod.rs` (Videogröße, Crop-Maß), `replay/mod.rs`
(`FrozenSelection::size`), Gates in `render/gates.rs` (Icon-Controls nach
Tinte, Maße-Rand), Tests, Shots (alle Zustände, Zwischenbilder, Namen bei
300 ms/400 ms/460 ms).

**Macht nicht:** V-a/V-b darüber hinaus, Tastatur in der Leiste, Windows.

## 4c. S6 Kompakt K2 (27.10., nach dem Review von S4)

Nutzer: die Leiste aus S4 ist „zu groß / klobig“ (die Hover-Namen passen).
Aus drei kompakten Entwürfen gewählt: **K2** (`scratchpad/sel/mock/compact-1x.png`,
`compact-2x.png`, Zeile 3; Code `mock/compact.py`). Jede Größe ist eine Formel
aus den Tokens, als benannte Konstante in `render/toolbar.rs` mit Grund:

| Konstante | Formel | Wert | Grund |
| --- | --- | --- | --- |
| `TEXT` | `TextStyle::Label` (S7, vorher `Key`) | Versalien 10 | eine Stufe für Labels, Maße, Namen, Grund; die der Settings-Zeilen |
| `CTL` | `TEXT.cap + 2·GAP_S` | 26 (S6: 24) | Seitenrand = (CTL − cap)/2 = `GAP_S` (Skill: horizontal so viel wie oben/unten), Versalien exakt mittig, `CTL − cap` gerade |
| `SIDE` | `(CTL − TEXT.cap)/2` | 8 | = `GAP_S` |
| `PAD` | `GAP_XS/2` | 2 | halbe Stufe: die Leiste schmiegt sich um ihre Controls, ihr Rand ist nur die Kontur der Gruppe, keine eigene Fläche |
| `IN_GAP` | `PAD` | 2 | jede innere Fuge gleich dem Rand: Controls einer Gruppe stehen wie im Rand gefasst |
| `GROUP_GAP` | `PAD + GAP_XS` | 6 | die innere Fuge plus genau eine Stufe: eine Gruppengrenze liest sich als eine Stufe mehr (der Entwurf hat 6) |
| `CTL_RADIUS` | `SIDE − PAD` (S7, vorher `CTL/4`) | 6 | Seitenraum minus Rand: die Ecke eines Controls sitzt in der der Leiste wie seine Kante in deren Kante; ganze Pixel bei jeder Textstufe (`CTL/4` wäre 6,5) |
| `BAR_RADIUS` | `CTL_RADIUS + PAD` | 8 | konzentrisch genau (innen = außen − Rand), beide über die Squircle-Abbildung |
| `BAR_H` | `CTL + 2·PAD` | 30 (S6: 28) | auch Höhe von Hinweis und Namen |
| `ICON_BOX` | theme `ICON` (S7, vorher `1,5 · Key.cap` = 12) | 14 | so, wie die Settings-Zeilen `ICON` mit `Label` paaren; nie größer als die Nenngröße der Schrift (14,3) |
| `ICON_TEXT` | `GAP_S` | 8 | Tinte → Text genau ein Token; gerendert mit `GAP_XS` und `GAP_S`, der Entwurf hat 7 (Box bei `SIDE − 1`, 6 px Box → Text), `GAP_S` ist 1 entfernt, `GAP_XS` 3 |

- Record ruht nicht mehr weiß: Fläche `RAISE`, Label `TEXT`, Punkt `ERROR`;
  Hover/Druck setzen die Leiter fort (`HOVER`, `SELECTED`). Bleibt die
  einzige ruhende Fläche und die einzige Farbe, also die eine primäre
  Aktion. **Die Primär-Regel aus S4 (`ACCENT`, Viertel-Schritte Richtung
  `SUB`) entfällt.** Kontrast des Punkts auf `RAISE`/`HOVER`/`SELECTED` neu
  gemessen.
- Clip, Frame, Audio wie S4 (an = `HOVER`, aus = keine Fläche + `SUB`);
  Frame/Audio `CTL × CTL`, Icon mittig.
- Maße wie S4 (reservierte Breite, echte Crop-Größe), rechter Rand `SIDE`
  (= `GAP_S`) + `PAD`.
- Hover-Namen und Clip-Grund: dieselben Flächenregeln wie die Leiste, eine
  Leiste mit einem unsichtbaren Control: Höhe `BAR_H`, Radius `BAR_RADIUS`,
  Text `Key` bei `PAD + SIDE`; Abstand zur Leiste `GAP_XS` wie bisher.
- Abstand Leiste ↔ Auswahl bleibt `GAP_M` (12): die Griffe ragen 4 + 1 px
  (Kontur) über die Kante, 12 lässt 7 px Luft zwischen Griff und Leiste;
  kleiner (`GAP_S` 8) ließe 3 px und läse sich als angeklebt.
- Lagen und das Wegfallen der Maße rechnet `layout` aus den neuen Breiten
  neu (Zahlen im Stand).
- Badge im Screenshot-Modus: unverändert (`Label`, `CTL_H`), nicht Teil von K2.

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
| S2 | umgesetzt 27.10.: `selector/motion.rs` (`BarMotion`, `Targets`, `Bar`/`Drawn`), Tests in `selector/motion_tests.rs`; `Scene::bar` statt `toggles`/`hovered`; `SceneCache` damagt nur die Leisten, wenn sich sonst nichts ändert; `toolbar::draw_faded` legt jede Leiste als eine Ebene mit `PixmapPaint::opacity` auf; `select_skia`: Ziele bei jedem `request_redraw` (Ereigniszeit), `pressed`, `pointer_in` (Leave nimmt Hover und Druck zurück), Treffer nur auf der aktuellen Leiste (`BarMotion::layout`), leerer Commit mit Frame-Callback, solange eine Feder läuft | Motion-Liste (alle `QUICK`, 150 ms, bounce 0): Leiste erscheint nach dem Loslassen, verschwindet beim neuen Aufziehen, Wechsel Streifen/2×2/Spalte und über/unter/innen als Überblendung an beiden Orten, Hover, Druck, An/Aus (Fläche, Text, Lautsprecher-Icons überblenden), Clip wird verfügbar. Ohne Animation: Mitziehen mit der Auswahl, Rahmen, Griffe. Tests: Einblenden (50 ms zwischen 0,2 und 0,95, 150 ms > 0,98, exakt 1 nach dem Ausschwingen), Mitziehen ohne Blende, Überblendung (alte Leiste bleibt am alten Ort, nur die neue nimmt Eingaben), Ausblenden ohne Eingaben, Retarget des Hovers ohne Sprung (< 1e-4) mit übernommener Geschwindigkeit (Abweichung < 50 %), Clip hellt auf; Szene: zufällige Folgen mit reinen Leisten-Änderungen gleich dem Vollbild, Damage bleibt in der Leiste (+ 3 px), Ebenen-Deckkraft halbe Mischung ± 1,5. Reduced Motion: keine räumliche Bewegung vorhanden, Blenden bleiben (wie `App::reduced_motion`). Zwischenbilder 30/60/100 ms unter `scratchpad/sel/s2/` angesehen. `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 133 + 388 grün (4 ignoriert), `git diff --check` |
| S3 | umgesetzt 27.10.: `selector/clip.rs` (`ClipState`, sechs Gründe), `replay::preview_target() -> Result<PreviewTarget, ClipState>` über `preview_target_from` (stopped/keine Antwort → Off, starting, busy, nicht ready → Stalled, Fehler oder fehlendes Routing → Failed); `Selector::clip` startet mit Ziel in `Loading`, `FrozenSelection::prepare` → `Ready`, scheitert es (oder anderes Output, kein Bild) → `Failed` statt stumm; `Targets::reason`, `BarMotion` blendet den Hinweis mit `QUICK` (Text bleibt beim Ausblenden), `toolbar::hint_rect`/`draw_hint` (eine Ebene), Damage und Zeichnen in `scene.rs`. Meldung beim Drücken von Clip unverändert. V-a/V-b nicht gebaut | Tests: Zuordnung Status → Grund (8 Fälle), Hinweis blendet ein/aus und behält die Worte, liegt mit Clip bündig `GAP_XS` neben der Leiste weg von der Auswahl (über der Leiste bei „über“, darunter bei „unter“), nie über der Leiste, jeder der sechs Gründe in jeder der 35 Lagen im Bildschirm (Gate 1), Copy-Gate 10 Strings ohne Treffer; zufällige Szenen mit Hinweis gleich dem Vollbild. Bekannt: auf sehr kleinem Bildschirm (Spalte bei 160×240) ist außen kein Platz, der Hinweis liegt dann über dem oberen Rand der Auswahl. Renders unter `scratchpad/sel/after/` (33 Bilder, darunter `noclip-hover*`, `hint-in-*ms`) angesehen. `cargo fmt --check`, `cargo clippy --all-targets` ohne Warnung, `cargo test` 133 + 391 grün (4 ignoriert), `git diff --check` |
| Offen | | Live-Prüfung durch den Nutzer (`boltsnap record`: Aufziehen, Einblenden beim Loslassen, Hover, Frame/Audio, Clip ohne und mit laufendem Replay samt Hinweis, Leave auf einen anderen Monitor); „Ohne Animation“-Liste (Mitziehen, Rahmen, Griffe) bestätigen; breitestes Icon → Text liegt mit `GAP_S` + 0,50 genau an der Grenze (gleiche Arithmetik wie das Settings-Fenster); Aufnahmepunkt auf `SELECTED` (gedrückt) 2,9:1, nur im Moment, in dem der Selector schließt. Kein Windows-Target installiert, Windows-Code unverändert und per Hash gehalten |
| S5 | umgesetzt 27.10., siehe Settings-Plan Phase 5f (Status dort): Squircle für jede `Canvas`-Fläche, also auch Leiste, Badge, Hinweise | Zahlen im Settings-Plan |
| S4 | umgesetzt 27.10. nach Abschnitt 4b: `render/toolbar.rs` neu (Items in Zeilen, vier Lagen, `Metrics::with_size`, primäre Fläche, Icon-Controls, `name`), `render/hint.rs` neu (Ebene, Hinweis, Namen), Ziffern-Helfer in `chrome.rs` gemeinsam mit dem Badge, `motion.rs` (`Targets::size`, Namen mit `NAME_DELAY` 400 ms über eine Feder mit Start in der Zukunft, `Bar::tips`), `scene.rs` (Damage/Zeichnen der Namen), `select_skia/mod.rs` (Videogröße aus `OutputInfo::modes`, gedreht bei 90°/270°, sonst Fläche; Replay-Bild ersetzt sie; Maß über `Crop::from_selection`), `replay/mod.rs` (`FrozenSelection::size`), `gates.rs` neu (Tinte auf dem Kanal mit dem größten Unterschied, Icon-Controls nach Tinte, Gruppen-Fugen, Maße-Rand). **Abweichungen:** (1) Outfit hat proportionale Ziffern (gemessen 4,06 bis 7,49 px in `Key`), ab_glyph kein `tnum`; gleiche Zellen klafften um die „1“, also wird proportional gesetzt und die Breite für den breitesten Wert reserviert, den der Bildschirm erlaubt (alle Werte mit der höchsten Stellenzahl bis Breite und Höhe ausgemessen, einmal beim Start); das Badge im Screenshot-Modus behält die Zellen. (2) Die Namen beider Icon-Controls teilen einen Platz (bündig mit Frame, so breit wie der breiteste Name): getrennt bündig überlappten sich beim Tausch zwei Flächen an verschiedenen Stellen („FramAudio on“); so blenden nur die Worte, die Fläche bleibt stehen. (3) Label-Abstand: statt `SIDE + ICON + GAP_S` nun `SIDE` + rechte Tinte des breitesten Label-Icons (Punkt 12,83 px, Schere 12,43 px) auf ganze Pixel (13) + `GAP_S`, sonst stünde der Text 1,17 px weiter weg als `GAP_S` (Gate 8). (4) Primäre Hover-/Druck-Regel neu (kein Vorbild im Settings-Fenster), Aufnahmepunkt darauf 3,31:1 in Ruhe, 2,75:1 im Hover, 2,22:1 gedrückt (Grafik neben dem Wort, berichtet statt versteckt) | Gates (eingebettete Schrift `cargo test`, Nutzerschrift `selector_shots`): Überlauf 42 Lagen auf 6 Größen, Leiste, Maße, sechs Gründe und der Namensplatz im Bildschirm, nie über der Leiste; Lagen: eine Zeile 332 px, ohne Maße 241, zwei Zeilen 169, drei 98 (eingebettet); Versalien schlechtester Versatz 0,01 (Nutzer 0,00); Icon auf Versalien 0,01 (0,00); Icon-Controls Tinte zur Mitte 0,00 an, 0,12 aus (Lautsprecher mit Kreuz); Innenrand 4,0/4,0/4,0, Maße enden `SIDE + GAP_XS` vom Rand, Gruppen-Fuge 8, innen 4; Achse 0,00 (drei Zeilen 0,34), Kante 0,00, breitestes Icon → Text `GAP_S` + 0,25; Tinten-Abstand rechts der Maße 21,83 px (reserviert für 3840 × 2160, gezeigt 3840 × 2160) gegen 15,25 px links zum Punkt; Kontrast 9 Textpaare, niedrigstes 5,38:1, „Record“ auf der primären Fläche 15,86/13,14/10,63:1; Copy 12 Strings. Motion zusätzlich: Namen nach 400 ms Ruhe (Test: bei 399 ms exakt 0, bei 450 ms zwischen 0,2 und 0,95), Tausch sofort mit Überblendung, Wortwechsel überblendet, Überstreichen zeigt keinen Namen, Verlassen `QUICK`; Primär-Hover/Druck `QUICK`. Renders `scratchpad/sel/s4/` (41 Bilder: alle Zustände, Druck, vier Lagen, Namen bei 300/400/460/600 ms, Tausch 40/100 ms, Erscheinen/Hover/Aus/Wechsel bei 30/60/100 ms), vorher `scratchpad/sel/s4-before/`, Zoom 4× angesehen. fmt, clippy ohne Warnung, `cargo test` 137 + 393 grün (4 ignoriert), `git diff --check` |
| S6 | umgesetzt 27.10. nach Abschnitt 4c (K2): Konstanten `TEXT`, `CTL` 24, `SIDE` 8, `PAD` 2, `IN_GAP` 2, `GROUP_GAP` 6, `CTL_RADIUS` 6, `BAR_RADIUS` 8, `BAR_H` 28, `ICON_BOX` 12, `ICON_TEXT` = `GAP_S` in `render/toolbar.rs` mit Formel und Grund; Record auf der Leiter (`RAISE` → `HOVER` → `SELECTED`, Label `TEXT`), die `ACCENT`-Primärregel aus S4 entfernt; Hinweis und Namen als Leiste mit einem flächenlosen Control (`BAR_H`, `BAR_RADIUS`, Text bei `PAD + SIDE`); Abstand zur Auswahl bleibt `GAP_M`. `ICON_TEXT`: mit `GAP_XS` und `GAP_S` gerendert (`scratchpad/sel/s6/z-icon-text-gap.png`), der Entwurf hat 7 px Tinte → Text, `GAP_S` liegt 1 daneben, `GAP_XS` 3, also `GAP_S` | Gates (eingebettet `cargo test`, Nutzerschrift `selector_shots`): Lagen-Breiten eingebettet: eine Zeile 279 px, ohne Maße 192, zwei Zeilen 141, drei Zeilen 78 (je ab Bildschirmbreite + 2·`GAP_S`; Test leitet die Schwellen aus den Breiten ab); Überlauf 42 Lagen auf 6 Größen inklusive Maße, sechs Gründe, Namensplatz; Versalien 0,01 (Nutzer 0,00); Icon auf Versalien 0,01 (0,00); Icon-Controls zur Mitte 0,03 an, 0,15 aus; Innenrand 2,0/2,0/2,0, Fugen 2/6, Maße enden `SIDE + PAD` vom Rand; Achse 0,00 (drei Zeilen 0,31), Kante 0,00, breitestes Icon → Text `GAP_S` + 0,06; Kontrast 6 Textpaare, niedrigstes 5,38:1; Aufnahmepunkt 3,87:1 auf `RAISE`, 3,47:1 auf `HOVER`, 2,91:1 auf `SELECTED` (gedrückt, schließt sofort); Copy 12 Strings. Renders `scratchpad/sel/s6/` (41 Bilder), vorher `scratchpad/sel/s4/`, Vergleich mit dem Entwurf 1× und 4× (`z-vs-mock-1x.png`, `z-vs-mock-4x.png`), vorher/nachher `z-before-after.png`, Zustände `z-states.png`, Lagen `z-rows.png`. fmt, clippy ohne Warnung, `cargo test` 137 + 393 grün, `git diff --check` |
| S7 | umgesetzt 27.10. (Nutzer zu K2: „insgesamt nen ticken größer“): `TEXT` = `Label`, `CTL` 26, `ICON_BOX` = `ICON` 14, `CTL_RADIUS` = `SIDE − PAD` 6, `BAR_RADIUS` 8, `BAR_H` 30; alle anderen Formeln gleich (Tabelle in 4c). Abstand zur Auswahl bleibt `GAP_M`: er zählt von der Auswahlkante, die Griffe ragen 5 px heraus, 7 px Luft wie vorher, die höhere Leiste ändert daran nichts | Lagen-Breiten (eingebettet): eine Zeile 320 px, ohne Maße 215, zwei Zeilen 163, drei Zeilen 90; Überlauf 42 Lagen auf 6 Größen; Versalien 0,02 (Nutzer 0,00); Icon auf Versalien 0,01 (0,00); Icon-Controls 0,00 an, 0,12 aus; Innenrand 2,0 rundum, Fugen 2/6; Achse 0,00 (drei Zeilen 0,34), Kante 0,00, breitestes Icon → Text `GAP_S` + 0,25; Kontrast unverändert (niedrigstes 5,38:1, Punkt 3,87/3,47/2,91:1); Copy 12 Strings. Renders `scratchpad/sel/s7/`, S6 gegen S7 bei 1× und 4× (`z-s6-vs-s7-1x.png`, `z-s6-vs-s7-4x.png`), Zustände/Lagen `z-states.png`. fmt, clippy ohne Warnung, `cargo test` 137 + 393 grün, `git diff --check` |
| Offen (S4/S5) | | Live durch den Nutzer: Hover-Namen im echten Zeigerverhalten (Hyprland meldet Bewegung innerhalb der Fläche), Maße bei HiDPI-Ausgang (Modus physisch, Fläche logisch), Druck auf Record (schließt sofort). Kein Windows-Target installiert; Windows-Code unverändert, Hash-Test grün |

## Nachtrag 27.10.: Aufnahmepunkt im Druck (Wunsch des Nutzers)

Offen war: der rote Punkt auf Record hat auf der gedrückten Fläche
(`SELECTED`) nur 2,91:1. Nutzer: „dann mach das schöner“. Soll: der Punkt
hellt mit der Fläche auf, wie ein Licht, das beim Drücken angeht:
`mix(ERROR, TEXT, 0,125 · (Stufe − 1))`, Stufe 1 (Ruhe) unverändert, Stufe
3 (gedrückt) ein Viertel Richtung `TEXT`. Gate: Punkt ≥ 3:1 auf jeder
Stufe der Leiter, mit dem Wert, den die Stufe malt. Die übrigen offenen
Punkte: „Ohne Animation“-Liste (Mitziehen, Rahmen, Griffe) und der Hinweis
bei 160 × 240 dem Nutzer genau erklärt. Antwort 27.10.: „die sachen die ich
noch kontrollieren sollte sind alle fine“: beide so abgenommen, keine
Änderung.
Umgesetzt 27.10.: `toolbar::mark`, Record malt damit, `gates.rs` misst den
Punkt mit seiner Farbe. Gate: 3,87:1 Ruhe, 3,95:1 Hover, 3,80:1 gedrückt
(vorher 2,91:1). fmt, clippy, `cargo test` grün.

## Nachtrag 27.10.: Clip fehlt ganz, wenn Replay aus ist (Wunsch des Nutzers)

O-Ton: „wenn ich replay aus mache soll der Clip button aus dem selector UI
verschwinden checkst du? also nicht nur grau sondern ganz raus. bitte plan
diese aenderung erst so gut es geht damit da nichts schief geht.“

**Verhalten:**
- `ClipState::Off` (kein Puffer läuft oder niemand antwortet,
  `replay::preview_target_from`): die Leiste ist [Record] · [Frame, Audio]
  · Größe. Kein Clip-Knopf, keine Trefferfläche, kein Hinweis „Replay is
  off“, keine Lücke: Record und Frame stehen im Gruppenabstand wie zwei
  Gruppen sonst. Die Leiste ist entsprechend schmaler.
- Alle anderen Zustände (Starting, Loading, Busy, Stalled, Failed, Ready)
  wie bisher: Clip da, gedimmt mit Grund, oder bereit. Begründung: nur „aus“
  ist eine Entscheidung des Nutzers; die anderen sind vorübergehend oder ein
  Fehler, den der Hinweis erklärt.
- Der Zustand steht vor dem ersten Bild fest: `run_select_record` fragt
  `preview_target()` synchron, bevor das Overlay entsteht, und während der
  Selector offen ist wird nicht neu gefragt. Die Leiste springt also nie
  zwischen drei und vier Knöpfen; keine Animation nötig (nichts kommt oder
  geht, solange man hinsieht).
- Der Screenshot-Selector hat keine Leiste, unberührt.

**Umsetzung:**
1. `render/toolbar.rs`: `Metrics` bekommt `clip: bool` (Standard `true`) und
   `without_clip()`; `Metrics::width(Control(CLIP))` ist dann `None`, und
   `rows` lässt es weg wie heute die fehlende Größe; leere Reihen fallen weg
   (Anordnung 3 wäre sonst [Record], [], [Frame, Audio] und `extent`
   indiziert `row[0]`). `Layout` bekommt `shown: [bool; 4]`; `draw_labelled`
   überspringt nicht gezeigte Knöpfe (sonst malte es eine Schere bei (0, 0)
   in ein leeres Rechteck). `hits[CLIP]` bleibt `Rect::default()`, das
   `contains` nie trifft (`ui/mod.rs:149`, halboffen).
2. `select_skia/mod.rs`: ein Helfer baut `Chrome` mit `without_clip()`, wenn
   `clip == ClipState::Off`, an beiden Stellen (Fallback-Schrift `:177`,
   echte Schrift `:279`); `set_video` behält es (`with_size` ändert nur die
   Größe).
3. `motion.rs`/`scene.rs`: der Hinweis braucht `hovered == Some(CLIP)`, das
   ohne Trefferfläche nie eintritt; zusätzlich `layout.shown[CLIP]` als
   Bedingung, damit `hint_rect` nie an einem leeren Rechteck ankert.
4. `gates.rs`: die Messung läuft auch für die Leiste ohne Clip und
   überspringt nicht gezeigte Knöpfe.
5. `shots.rs`: eine Aufnahme „replay off“ für den Blick.

**Was brechen kann:** Schleifen über `layout.controls`, die alle vier
annehmen (`draw_labelled`, `gates.rs`, Tests), Ankern des Hinweises,
Anordnung mit leerer Reihe, Trefferflächen ohne Totzone zwischen Record und
Frame.

**Tests:** ohne Clip bei allen Bildschirmgrößen und Auswahl-Lagen der
bisherigen Layout-Tests: keine leere Reihe, `shown[CLIP]` falsch, kein
Punkt der Leiste trifft Clip, die übrigen Knöpfe überlappen nicht und haben
`IN_GAP`/`GROUP_GAP`, Trefferflächen lückenlos; gezeichnet: außerhalb der
Leiste kein Pixel verändert, keine Schere; `bar_targets`/Selector mit Off
baut Metrics ohne Clip, mit Starting mit Clip; UI-Gates (Zentrierung,
Ränder, Kontrast) für die Leiste ohne Clip.

**Macht nicht:** kein Nachfragen des Replay-Zustands während der Selector
offen ist, keine Änderung an den anderen Zuständen oder ihren Hinweisen,
kein neuer Config-Key.

**Live (mit Okay):** Replay aus, `boltsnap record`: Leiste ohne Clip; Replay
an: Clip wie bisher.

**Stand:** umgesetzt 27.10. wie geplant: `Metrics::without_clip`, `rows`
ohne leere Reihen, `Layout::shown`, `draw_labelled` überspringt; Helfer
`select_skia::chrome(fonts, clip)` an beiden Stellen; Hinweis nur bei
`shown[CLIP]`; `gates::measure_with`; Shots `replay-off`,
`replay-off-column` angesehen (Record · Frame im Gruppenabstand, Spalte ohne
leere Reihe). Abweichung: die erste Testannahme „nie breiter als mit Clip“
war falsch, ohne Clip passt bei 300×300 schon eine Reihe; geprüft wird
jetzt „nie eine spätere Anordnung, in derselben schmaler oder niedriger“.
Tests: `without_replay_the_toolbar_leaves_clip_out_entirely` (6 Bildschirme
× 4 Lagen: `shown`, leere Rechtecke, keine leere Reihe, Gruppenabstand,
kompakter, kein Pixel außerhalb der Leiste),
`gate_capitals_padding_and_icons_measure_without_clip_too` (Versalien worst
0,02 px, Icon-only 0,12 px, Rand 2,0 rundum),
`with_replay_off_the_toolbar_has_no_clip_and_still_no_dead_zone` (jeder
halbe Pixel trifft einen Knopf, nie Clip; alle anderen sechs Zustände
behalten Clip). fmt, clippy `-D warnings`, `cargo test` 161 + 404 grün.
Live offen (Replay aus, Selector öffnen).

## Nachtrag 27.10.: Reihen füllen die Leiste (Nutzer: Abstände)

O-Ton zum Bild der Spalte ohne Clip: „achte auch drauf das die abstaende
alle schoen sind und nicht so unterschiedliche abstaende ueberall sind“.

**Befund:** In mehrreihigen Anordnungen ist jede Reihe so breit wie ihr
Inhalt, die Leiste so breit wie die breiteste Reihe. Spalte ohne Clip:
[Record] ist breiter als [Frame, Audio] (2 · `CTL` + `IN_GAP` = 54 px),
rechts neben Audio bleibt ein Loch, also rechts ein ganz anderer Rand als
die 2 px (`PAD`) sonst. Dasselbe gab es schon mit Clip ([Clip] kürzer als
[Record]) und in Anordnung 2 je nach Größe. Abstände sonst: `PAD` 2,
`IN_GAP` 2, `GROUP_GAP` 6 (eine Stufe mehr zwischen Gruppen) bleiben.

**Verhalten:** Bei mehr als einer Reihe endet jede Reihe am rechten Rand
(`PAD` vor der Kante). Den Rest einer Reihe bekommt die Größe, wenn sie in
der Reihe steht (ihr Text bleibt links, wie sie ohnehin steht); sonst
teilen ihn die Knöpfe der Reihe zu gleichen ganzen Pixeln, ein übriger
Pixel geht an den letzten. Beschriftete Knöpfe behalten Icon und Text an
derselben Stelle (Textkante wie gestapelt bisher), Icon-Knöpfe ihr Icon in
der Mitte, auf ganzem Pixel (abgerundet). Eine Reihe ändert nie die
Höhe. Einreihige Leisten unverändert.

**Umsetzung:** `toolbar::layout`: nach der Wahl der Anordnung die Breiten
der Reihen auffüllen, bevor die Rechtecke entstehen (Treffer folgen von
selbst). `icon_rect` rundet die Lage ab. `gates::check`: Icon-Knöpfe nur in
einer Reihe quadratisch, sonst mindestens `CTL`; neu: bei mehreren Reihen
endet jede an `bounds.right() − PAD`.

**Tests:** alle Bildschirme und Lagen aus Gate 1, mit und ohne Clip: jede
Reihe endet bündig, Abstände in der Reihe genau `IN_GAP`/`GROUP_GAP`, ganze
Pixel; Gates (Zentrierung ≤ 0,5 px) auch für die gestreckten Icon-Knöpfe;
Shot der Spalte mit und ohne Clip.

**Stand:** umgesetzt 27.10. wie geplant (`toolbar::justify`, `icon_rect`
abgerundet, `gates::check` mit Reihen-Ende). Test
`every_row_of_a_stacked_toolbar_reaches_the_padding` (mit Größe, ohne Clip,
ohne Größe × 5 Bildschirme × 3 Lagen: jede Reihe `PAD` vom linken und
rechten Rand, Abstände nur `IN_GAP`/`GROUP_GAP`, ganze Pixel; Gegenprobe:
1 px schmaleres Audio fällt dem Gate auf). Gates mit der Nutzer-Schrift
auf den gestapelten Leisten: Versalien worst 0,00 px, Icon-only 0,12 px,
Rand 2,0 rundum. Shots `noclip-hover-column`, `replay-off-column`
angesehen: Frame und Audio füllen die Reihe. fmt, clippy `-D warnings`,
`cargo test` 161 + 405 grün.
