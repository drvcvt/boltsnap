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

## 2. Richtung (Entscheidung offen, Empfehlung)

Die Leiste ahmt bisher die Module der Desktop-Leiste nach (Kommentare in
`render.rs:946-976`, Schrift aus `ui_font` in 500/600). Seitdem hat Boltsnap
ein eigenes Designsystem (`src/ui/theme.rs`: Farben, Abstände, `CTL_H`,
Radien, `TextStyle` mit Versalhöhen auf ganzen Pixeln, Lucide-Icons, drei
Federn), das der Nutzer im Settings-Fenster abgenommen hat.

**Empfehlung:** Selector-Chrome (Leiste, Größen-Badge, Rahmen, Griffe) nutzt
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

**Vorschläge, nur mit Okay des Nutzers (neue Funktion):**
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

## 5. Abnahme (Gates mit Zahlen)

Offscreen über einen ignorierten Test, der `scene::paint` auf ein Bild rendert
(`SEL_OUT=dir cargo test --bin boltsnap selector_shots -- --ignored`):
Streifen bei 1920×1080, 2×2 bei 400×300, Spalte bei 160×240, Auswahl am
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
| Richtung | offen, Empfehlung Abschnitt 2 | |
| S0–S3 | nicht begonnen | |
