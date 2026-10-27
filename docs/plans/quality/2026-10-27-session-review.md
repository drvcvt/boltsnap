# Review 27.10.: Shelf-Painter, Settings-Vorschauen, Replay, Controls, Doku

Umfang: Commits `0ba034d..HEAD` (Stand `26be77b`), 28 Commits, 83 Dateien.
Vier Reviewer, je ein Bereich, nur lesend (kein Build, kein Prozess, kein
Desktop). Wunsch des Nutzers: „review agents losschicken die einmal alles
reviewen“. Befunde sind aus dem Code gelesen; „bestätigt“ heißt, ich habe
die Stelle selbst nachgelesen, „reproduziert“ heißt, ein Test oder Lauf hat
es gezeigt.

## Befunde

| # | P | Bereich | Befund | Auslöser → Wirkung | Beleg | Stand |
| --- | --- | --- | --- | --- | --- | --- |
| B1 | P1 | Vorschau | Unsichtbare Kartenteile nehmen Klick und Hover | Ecke oben, Karten ≥ 210 px, zwei gezeigt: die untere Karte ragt bis 240 px über den Block; `card_at` prüft nicht gegen den Block → Slider „Card size“ und „Cards shown“ darunter reagieren nicht, ein Klick kann über das verdeckte X eine Karte schließen | `app/preview/shelf.rs::card_at`, `app/mod.rs::target_at` | bestätigt |
| B2 | P1 | Vorschau | Wegklicken einer Karte, die gerade ankommt | `anim()` nimmt den ersten Eintrag, `dismiss_card` hängt „leaving“ hinter das Appear → Karte wächst weiter und verschwindet ohne Animation, bleibt klickbar, jeder weitere Klick legt eine Karte nach | `shelf.rs:131`, `:279–296`; der Overlay räumt vorher auf (`linux/shelf/anim.rs` `retain`) | bestätigt |
| A1 | P2 | Open with | Der Editor bearbeitet die Cache-Datei der Karte | Speichern am Platz (Pinta, Krita, GIMP „Overwrite“), dann Karte schließen oder Cache-Grenze → bearbeitete Datei gelöscht (**Datenverlust**) | `cards.rs::open_media_card` gibt `png_path` (temporär), `Thumb::delete_file_on_dismiss`, `trim_shelf_cache` | bestätigt |
| C1 | P2 | Replay-Log | Panik im Log-Thread tötet den Replay | `eprintln!` panikt bei Schreibfehler (volle Platte, `daemon.log`), der Thread endet, die Pipe schließt, gsr stirbt an SIGPIPE | `linux/replay/capture_log.rs` (`eprintln!` in `follow`) | bestätigt |
| C2 | P2 | Lastanzeige | Eine laufende Aufnahme zählt mit | Aufnahme-gsr ist auch Kind des Daemons mit `comm` `gpu-screen-reco` → „In use now“ enthält die Aufnahme | `linux/app/load.rs::replay_pids` | bestätigt |
| C3 | P2 | Selector | Lupe „Always“ steht bis zur ersten Bewegung bei (0, 0) und bleibt nach Leave stehen | `cursor` startet bei (0, 0), `magnifier_on` fragt `pointer_in` nicht, Enter zeichnet nicht neu | `select_skia/mod.rs`, `input.rs` | bestätigt |
| C4 | P2 | Controls | Die Zeit rutscht beim Ticken um etwa 1 px | `ink_to` setzt die Tintenkante der wechselnden Ziffer | `shelf/paint/controls.rs` Zeit | bestätigt (gelesen) |
| B3 | P2 | Vorschau | Weggeklickte Karte behält Hover | `press_card` lässt Hover stehen → mit „Solid on hover“ geht sie deckend statt in ihrer Deckkraft, und es wird doppelt gemalt | `shelf.rs::press_card`; Overlay `linux/shelf/mod.rs` Close löscht `hovered` | bestätigt |
| B4 | P2 | Lastanzeige | „Off, nothing in use“, obwohl der Buffer läuft | kein Messwert (älterer Daemon ohne `session`, keine Kinder gefunden) wird als aus gelesen | `preview/load.rs`, `linux/app/save.rs::poll_replay` | bestätigt |
| A2 | P2 | Open with | Fenster und Daemon können verschieden finden | anderer `PATH` (systemd-Dienst) → Editor angeboten, Klick öffnet `xdg-open`; hier nicht der Fall (`PATH` des Daemons enthält `~/.local/bin`, geprüft) | `linux/app/prefs.rs`, `cards.rs` | nicht reproduziert |
| A3 | P3 | Open with | Start schlägt fehl → nichts passiert | `has_cmd` prüft das Ausführrecht nicht; Spawn-Fehler nur `eprintln` | `linux/paths.rs::has_cmd`, `open_media_card` | bestätigt |
| A4 | P3 | Open with | „not found“ nur einmal pro Daemon | ein zweiter fehlender Editor wird nicht mehr gemeldet | `cards.rs` `TOLD` | bestätigt |
| A5 | P3 | Daemon | veralteter Kommentar | „tick_animations removes it“, bei Animations aus geht es sofort | `linux/shelf/mod.rs` Close | bestätigt |
| A7 | P3 | Painter | Knopfradius `s/2 − 0,5` unskaliert | nur Vorschau bei Skala ≠ 1, ein halber Gerätepixel | `shelf_card/paint.rs` Hover-Knöpfe | bestätigt |
| A8 | P3 | Windows | neue Linux-only Felder/Getter als `dead_code` auf Windows | Warnung, kein Fehler (nicht kompiliert, Ziel fehlt) | `config.rs` | nicht geprüft |
| B5 | P3 | Vorschau | Maske je Frame beim Scrollen neu | ragen die Karten über den Block, wird bei jeder Blocklage eine fensterweite Maske gebaut | `shelf.rs::clip_mask` | bestätigt |
| B6 | P3 | Vorschau | unnötiger Painter-Lauf | Bild „ohne Hover“ wird auch geholt, wenn nur das Hover-Bild gezeigt wird | `shelf.rs::paint_cards` | bestätigt |
| B7 | P3 | Vorschau | Platzhalter-Cache ohne Skala im Schlüssel | gleiche Pixelgröße bei anderer Skala → Icon falsch groß | `shelf.rs::picture` | bestätigt |
| B8 | P3 | App | toter Re-Export, Doppelung | `card_height` außen ungenutzt; Font-Dropdown baut `choices` von Hand | `app/mod.rs`, `pages/mod.rs::general` | bestätigt |
| B9 | P3 | Vorschau | X der Karten ohne Druck- und Fokuszustand | wie im Overlay; ui-Skill verlangt es | `app/keys.rs`, Painter | Entscheidung: bleibt wie das Overlay (die Vorschau zeigt das Overlay), steht hier begründet |
| C5 | P3 | Replay-Log | Log wächst pro Session ohne Grenze | ~60–70 B/s, ~5 MB/Tag | `capture_log.rs` | bestätigt |
| C6 | P3 | Lastanzeige | Meter wird bei Session-Wechsel nicht zurückgesetzt | erste Sekunde nach Neustart evtl. falsch niedrig | `load.rs::Meter` | bestätigt |
| C7 | P3 | Tests | Ink-Gate misst nur „Recording“ | die breitesten Labels („Resume“, „Confirm“) nie gemessen | `shelf/paint.rs` Gate | bestätigt |
| D | P3 | Doku | ~20 Stellen veraltet oder ungenau | README (Kartenklick vs. `open_with`, Replay-Seite, `autostart`), `codebase.md` (tote Links, „SAVE FAILED“, `capture_log.rs`, Selector-Plan nicht verlinkt), `replay.md` (Capture-Log), `Cargo.toml` (Replay nicht im Paket), Plan-Abweichungen in 5g/5h nicht markiert; im Code: Selector-Hinweis „Replay buffer → Start“, `usage()` ohne `settings [PAGE]`/`--copy` | Doku-Review | bestätigt |

## Fixplan

Reihenfolge nach Priorität, je mit Test, wo es geht.

1. **B1** `card_at`: zusätzlich `rects[block].contains(at)`. Test: Ecke oben,
   große Karten, ein Punkt unter dem Block trifft keine Karte.
2. **B2** `dismiss_card`: vor dem Anhängen `anims.retain(|a| a.id != id)`.
   Test: zweites X während des Appears → Karte geht, genau zwei bleiben.
3. **A1** Datenverlust: eine Bildkarte merkt sich die Änderungszeit ihrer
   Datei beim Ankommen; ist die Datei beim Schließen oder Verdrängen jünger
   (bearbeitet), wird sie nicht gelöscht, sondern in `save_dir` verschoben
   und das gemeldet. Tradeoff: eine Datei mehr im Speicherordner, dafür
   geht nie Bearbeitetes verloren; unberührte Karten gehen wie bisher.
4. **C1** `let _ = writeln!(std::io::stderr(), …)`.
5. **C2** gsr nur zählen, wenn seine Kommandozeile `-o /dev/stdout` hat
   (der Replay schreibt dorthin, Aufnahmen in Dateien). Tradeoff: hängt an
   einem Argument; ein Test hält es fest.
6. **C3** `magnifier_on` verlangt `pointer_in`; Enter und Leave zeichnen
   neu, wenn die Lupe gilt.
7. **C4** Zeit rechtsbündig nach Vorschub, Lagerkorrektur aus der Referenz
   „00:00“ (jede Ziffer als 0), sodass die Zeichenbreite und nicht die
   letzte Ziffer die Lage bestimmt.
8. **B3** nach dem X: Hover der Karte sofort auf 0, `hover` gelöscht.
9. **B4** „Running, not measured“, wenn der Buffer läuft (Haltezeit da),
   aber kein Messwert.
10. **A3/A4** Spawn-Fehler → `xdg-open` und Meldung; Meldung je Name statt
    einmal.
11. **A5, A7, B6, B7, B8, C5, C6, C7** wie in der Tabelle: Kommentar; `0,5 ·
    scale`; Bild ohne Hover nur bei Bedarf; Skala im Schlüssel; toten
    Re-Export weg, Font-Dropdown über `choices`; Log-Grenze 4 MiB pro
    Session (danach nur noch Verlangsamungen); Meter je Session; Gate für
    Paused und Confirm.
12. **D** Doku nach den Korrekturen des Doku-Reviews; Selector-Hinweis und
    `usage()`.

Nicht in diesem Durchgang: B5 (Maske nur an den Ecken, mehr Code als
Nutzen bei 60 Frames Scroll), A2 über ein Protokollfeld (der Spawn-Fallback
deckt es ab), A6 Test für Animations aus im Daemon (braucht Wayland-Objekte),
A8 (Windows eingefroren, Warnungen ohne Wirkung), A9 (`linux/shelf/` steht
bei 12 Dateien: vor der nächsten Datei dort aufteilen).

## Stand

| Schritt | Stand |
| --- | --- |
| 1 B1 | behoben (`card_at` prüft den Block); Test `a_card_past_the_block_is_not_there_to_hover`, schlägt mit dem alten Code an |
| 2 B2 | behoben (`retain` vor dem Anhängen); Test `closing_a_card_that_is_still_arriving_takes_it_away_and_leaves_two`, schlägt mit dem alten Code an |
| 3 A1 | behoben (`Thumb::arrived`, `Thumb::edited`, `Daemon::drop_card_file` verschiebt Bearbeitetes nach `save_dir`, löscht nur nach Erfolg); die PNG ist vor dem Anlegen fertig geschrieben (`prepare_add`), unbearbeitete Karten gelten also nicht als bearbeitet; Test `an_image_saved_over_after_it_arrived_counts_as_edited` |
| 4 C1 | behoben (`writeln!` statt `eprintln!`) |
| 5 C2 | behoben (gsr nur mit `-o /dev/stdout`); Test erweitert um eine Aufnahme als Kind; live 34 % / 440 MB weiter gefunden |
| 6 C3 | behoben (`pointer_in`, Enter zeichnet neu) |
| 7 C4 | behoben (`steady_ink_to`); Ink-Gate weiter grün mit beiden Schriften |
| 8 B3 | behoben (Hover sofort 0, `hover` gelöscht) |
| 9 B4 | behoben („Running, not measured“), im Test |
| 10 A3/A4 | behoben (Spawn-Fehler → `xdg-open` und Meldung, Meldung je Name) |
| 11 | A5 Kommentar, A7 `0,5 · scale`, B6 Bild ohne Hover nur bei Bedarf, B7 Skala im Schlüssel, B8 Re-Export weg und Font-Zeile über `choices`, C5 4 MiB je Session, C6 Meter je Session, C7 Gate für Paused und Confirm: erledigt |
| 12 D | README (Screenshots-Hinweis, Editor, Kartenklick, Replay-Seite, `autostart`, `open_with`, Windows), `codebase.md` (Links, „Save failed“, `cards.rs`, `capture_log.rs`, Selector-Plan verlinkt), `replay.md` (Logs und Last, Clip sagt warum), `Cargo.toml`, Plan-Abweichungen 5g V6 und 5h markiert; Selector-Hinweis ohne „REC“ und „→ Start“, `usage()` mit `settings [PAGE]` und `--copy` |

Checks nach den Fixes: `cargo fmt --check`, `cargo clippy --all-targets`
ohne Warnung, `cargo test` grün (Zahlen im Commit). Live offen: Daemon-Neustart
beim Nutzer, dann Kartenklick mit eddy, Bearbeiten und Schließen.

## Zweiter Durchgang: die zurückgestellten Punkte (27.10., Wunsch des Nutzers)

Wunsch: „dann plan und fix die findings und restarte meinen daemon“.

| # | Plan | Tradeoff | Prüfung |
| --- | --- | --- | --- |
| B5 | Die Vorschau malt ihre Karten in eine Pixmap in Blockgröße und schneidet dort mit einer Maske in Blockgröße (abgerundete Ecken), behalten je (Breite, Höhe, Skala); Scrollen ändert nur die Lage, also kein Neubau. Erst danach auf die Leinwand. | eine Pixmap in Blockgröße je Frame mit überstehenden Karten (≈ 0,7 MB bei 1×) statt einer fensterweiten Maske | Test: zwei Frames an verschiedenen Scroll-Lagen bauen die Maske einmal (Zähler) |
| A2 | `Response` bekommt `editors: Option<Vec<String>>`; der Daemon füllt es in der Antwort auf `GetSettings` aus `EDITORS` mit seinem eigenen `PATH`; das Fenster nimmt diese Liste, ohne Daemon oder bei einem älteren (Feld fehlt) seine eigene. | ein Feld im Protokoll, rückwärtsverträglich (fehlt = unbekannt) | Protokoll-Test hin und zurück, alter Header ohne Feld liest sich |
| A6 | Die Entscheidung in `start_anim` wird eine reine Funktion `anim_start(animations, kind) -> Start { Animate, RemoveNow, Nothing }`, getestet; `start_anim` folgt ihr. | eine kleine Funktion mehr | Test der drei Fälle |
| A8 | `#[cfg_attr(windows, allow(dead_code))]` an den Feldern und Gettern, die nur Linux liest (`ShelfPrefs`-Look-Felder, `look()`, `magnifier_always`, `shelf_open_with`). | keine Windows-Prüfung möglich (Ziel nicht installiert); Attribute ändern nichts am Verhalten | `cargo build`/`clippy` auf Linux unverändert |
| A9 | `linux/shelf/` unter die Grenze: `tests.rs` und `focus_tests.rs` per `git mv` nach `linux/shelf/tests/{mod,focus}.rs`; 10 Dateien und ein Ordner. | nur Pfade | `cargo test` unverändert grün, gleiche Zahl |

Danach: fmt, clippy, `cargo test`, Release, Installation, Daemon-Neustart.

Stand zweiter Durchgang (27.10.): alle fünf umgesetzt. B5
`preview/shelf.rs::draw_clipped` (Maske und Bild in Blockgröße, Schlüssel
Breite, Höhe, Skala), Test
`cards_past_the_block_are_cut_with_one_mask_however_the_page_scrolls` (drei
Scroll-Lagen, eine Maske, unter dem Block der Seitengrund). A2
`Response::editors`, `ok_settings_with_editors`, Daemon `send_settings`,
Fenster `prefs::editors` mit Rückfall auf den eigenen `PATH`; Tests im
Protokoll (hin und zurück, alter Header ohne Feld = `None`) und
`the_daemons_list_decides_which_editors_are_offered`. A6 `anim::anim_start`
mit Test. A8 `cfg_attr(windows, allow(dead_code))` an `ShelfPrefs`, `look`,
`magnifier_always`, `shelf_open_with`, `Thumb::arrived` (nicht auf Windows
kompiliert). A9 `linux/shelf/tests/{mod,focus}.rs`, 10 Dateien und ein Ordner,
`mod.rs` 3477 Zeilen. Checks: fmt, clippy ohne Warnung, `cargo test` 155 +
395 grün.
Danach aufgeteilt: [Shelf-Split](../shelf/2026-10-27-shelf-split.md).
