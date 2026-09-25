# Review-Fixes: Spezifikation und Implementierungsplan

Stand: 25.09.2026. Status: A1 freigegeben und umgesetzt; übrige Items geplant.
Ursprüngliche Basis: `7e736ad`; vertieftes Review: `cf243b5`.
Ursprünglich erstellt mit dem lokalen Skill
`/home/mt/.codex/skills/spec-plan/SKILL.md`.

## 1. Zusammenfassung und Grenzen

Dieser Plan behandelt die neun ursprünglichen Befunde, sechs zusätzliche Befunde
aus dem vertieften Review und die separat aufgeführte Cursor-Verhaltensfrage.
Damit sind 14 Fehler offen und A1 behoben; F1 ist eine Verhaltensentscheidung.
Der versionierbare aktuelle
[Befundstand](../codebase.md#open-review-findings) enthält die Codebelege und
kennzeichnet A1 als behoben. Der historische Review liegt lokal unter
`docs/reviews/2026-09-25-linux-review.md`; `docs/reviews/` ist absichtlich
Git-ignoriert und in einem frischen Checkout nicht vorhanden. Die dortigen
isolierten Repros (`docs/reviews/2026-09-25-repros.patch`) sind Ausgangspunkte für
Regressionstests, kein fertig übernehmbarer Test-Patch. Insbesondere enthält
der Container-Test Platzhalterbytes, keine echte Matroska-Datei
(`docs/reviews/2026-09-25-repros.patch:33`).

Die Empfehlungen in Abschnitt 4 sind Planungsannahmen, keine bereits
getroffenen Nutzerentscheidungen. Nach Erläuterung der lokalen atomaren
Dateiveröffentlichung wurde A1 zur Umsetzung freigegeben. Die übrigen Items
bleiben geplant. Keine Installation und kein Eingriff in den laufenden Daemon.
Die Arbeit erfolgt ohne Subagents gemäß der Projektvorgabe des Nutzers.
Ursachenbelege und unveränderte Planungsabschnitte zitieren die ursprüngliche
Basisrevision; G1 bis H1 und die A4-Ergänzung beziehen sich auf `cf243b5`.
Abschnitt 6 dokumentiert Prüfumfang, isolierte Repros und parallele Arbeit.

Linux bleibt der einzige Implementierungsumfang. Neue OS-Dateioperationen
gehören unter `src/platform/linux/`; Windows bleibt eingefroren. Gemeinsame
Daten-/Rendering-Helfer dürfen keine OS-Typen bekommen und müssen bisherige
Windows-Aufrufer weiter bedienen (`AGENTS.md:22`, `AGENTS.md:49`). Kein neues
Capture-Backend, keine neue Containerkonvertierung, keine neue Persistenz des
Shelf nach Daemon-Neustart. Der Shelf ist ausdrücklich RAM-only
(`README.md:259`).

| ID | Review-Befund | Priorität / Nachweis |
| --- | --- | --- |
| A1 | 4: kollidierende Part-Dateien | P2, reproduziert und behoben; Regressionstest grün |
| A2 | 1: Bild-Save überschreibt Dateien | P1, Codepfad; Test fehlt |
| A3 | 3: Dismiss/Eviction gegen Video-Save | P1, Codepfad; Scheduling-Test fehlt |
| A4 | 5: falsche Videoendung und MIME-Angabe | P2, Endung reproduziert, MIME im Code |
| B1 | 6: Video-Import über Symlink | P2, reproduziert |
| C1 | 7: doppelte Imports nach ACK-Abbruch | P2, reproduziert |
| D1 | 8: vollständiger Video-Read beim Drag | P2, Codepfad; RAM-Test fehlt |
| D2 | 9: Bild-Save im UI-Thread | P2, Codepfad; Blockier-Test fehlt |
| E1 | 2: Combined-Recovery und Resume | P1, reproduziert |
| F1 | Cursor-Handover | bestätigtes Verhalten, kein freigegebener Fix |
| G1 | Daemon-Cleanup vor exklusiver Startabsicherung | P1, vollständiger Codepfad; Race nicht live ausgelöst |
| G2 | Replay-Remux-Partials überleben die Neustartbereinigung | P2, Namensprüfung reproduziert |
| G3 | Prozess-Timeout begrenzt geerbte Ausgabepipes nicht | P2, reproduziert: 40 ms werden 1,202 s |
| G4 | RecordingWatch hält getrennte Idle-Clients unbegrenzt | P2, vollständiger Codepfad; FD-Stress nicht ausgeführt |
| G5 | Replay-Watchdog erkennt eine gehaltene Auswahl nicht | P2, Worker-Zustand synthetisch reproduziert; Neustart aus Code belegt |
| H1 | Nix-Paket meldet Version 0.4.3 statt 1.0.1 | P3, Manifeste direkt verglichen |

## 2. Wellen und Abhängigkeiten

| Welle | IDs in Reihenfolge | Ergebnis |
| --- | --- | --- |
| 0 | G1 | genau ein Daemon besitzt Socket und Startbereinigung |
| 1 | A1, A2, A3 | sichere Dateiveröffentlichung, kein Überschreiben, Schutz laufender Saves |
| 2 | E1 | fehlgeschlagene Finalisierung bleibt bedienbar und speicherbar |
| 3 | A4, B1, C1 | richtige Mediennamen, zuverlässiger Import, kein mutierendes IPC-Retry |
| 4 | D1, D2 | begrenzter Transferspeicher und responsive Bild-Saves |
| 5 | G2, G3, G4, G5 | begrenzter Cache, wirksame Deadlines, begrenzte Watches, geschützte Replay-Auswahl |
| 6 | H1 | Paketversion aus derselben Quelle wie Cargo |
| keine | F1 | Entscheidung dokumentieren, vorerst kein Produktcode |

A1 stellt die Kopier-/Veröffentlichungsprimitive für A2 bereit. A3 muss vor D2
stehen, damit asynchrone Bild-Saves nicht dieselbe Dateilebensdauer-Race erhalten.
D2 verwendet die A2-Operation unverändert weiter. A4 und D1 bearbeiten denselben
Drag-Pfad nacheinander, sind fachlich aber getrennte Fähigkeiten. B1 kann die
Dateioperationen aus A1 wiederverwenden. E1 braucht eine Produktentscheidung und
gezielte UI-/Session-Tests, wird wegen P1 vor die übrigen Komfort-/Performance-
Korrekturen gezogen. Keine Welle als vollständige Fehlerbereinigung ausgeben,
solange andere Befunde offen sind.

G1 hat wegen möglicher Löschung noch verwendeter Medien Vorrang vor weiterer
Cleanup-Arbeit. G2 setzt diese abgesicherte Eigentümerschaft voraus. G3, G4 und
G5 sind voneinander unabhängig, werden aber jeweils einzeln umgesetzt. G5 berührt
Supervisor und Worker-Protokoll gemeinsam; kein isolierter Worker-only-Fix.

### Gemeinsame Verifikation nach Umsetzung

Pro Item vor einem Commit: `cargo fmt --check`, `cargo test`, `cargo check` auf
Linux. Das folgt aus `AGENTS.md:55` und `AGENTS.md:58`. Plattformneutrale Tests
bleiben ohne OS-Ausblendung (`AGENTS.md:56`). Jeder Fix bekommt zuerst den unten
genannten roten Test; keine umfangreiche generische Test-Infrastruktur.

Replay-bezogene Änderungen, insbesondere A4: zusätzlich Worker-Formatprüfung,
Worker-Unit-Tests, Release-Build sowie `probe.py` und `live.py`, wie in
`README.md:508` und `src/platform/linux/replay/worker/README.md:63` verlangt.
Den Replay-Export anschließend durch den betroffenen Shelf-Save-Pfad schicken:
Worker-Tests allein prüfen keine dauerhaften Shelf-Zielnamen.

Tests verwenden isolierte Cache-/Zielverzeichnisse und Socket-Paare. Kein
Produktiv-Daemon-Neustart als Testabkürzung. Live-Checks erfolgen später
gesondert; ein ungetesteter manueller Check bleibt als offen gekennzeichnet.
Für Agent-Cleanup ausschließlich `/home/mt/.local/bin/safe-rm` nach Inspektion
der exakten Ziele verwenden. Keine pauschale Ausführung aller ignorierten Tests:
darunter liegt eine echte Desktop-Aufnahme
(`src/record/session.rs:1534`).

## 3. Items

### A1: Eigene Part-Datei je Kopiervorgang

**Phase 1: Verhalten und Ursache**

`copy_then_remove` verwendet `<Ziel>.part-<pid>` und entfernt diesen Pfad bei
jedem Fehler, auch wenn `create_new` gar nicht erfolgreich war
(`src/record/finalize.rs:721`, `src/record/finalize.rs:728`,
`src/record/finalize.rs:744`). Zwei Save-Threads desselben Prozesses können so
einander die Zwischenkopie entziehen. Ziel: Jeder Vorgang besitzt ausschließlich
seine eigene Part-Datei. Ein belegter Zielname oder fremder Part-Pfad darf weder
Quelle noch fremde Daten verändern. Der vollständige Inhalt wird erst nach
erfolgreichem Kopieren veröffentlicht.

**Phase 2: Umsetzung**

1. Die Copy-/Publish-Primitive und `rename_noreplace` nach
   `src/platform/linux/files.rs` auslagern und in `linux/mod.rs` einbinden.
   `move_final_file` bleibt als schmaler Dispatcher bei der Finalisierung;
   `copy_then_remove` behält Disk-Reservepolitik und Quellbereinigung.
   Diese Präzisierung vermeidet einen zusätzlichen Policy-Parameter und
   verändert die Abhängigkeiten anderer Items nicht
   (`src/record/finalize.rs:708`, `src/record/finalize.rs:718`).
2. Part-Namen verlustfrei mit `OsString`/`PathBuf`, PID und monotoner Sequenz
   bilden; `create_new` bleibt der Eigentumsnachweis. Bei Kollision einen neuen
   Namen versuchen. Cleanup erst nach erfolgreichem Anlegen aktivieren.
3. Eine Copy-Operation auf einem geöffneten Reader ohne Quelllöschung anbieten;
   die bestehende Move-Operation behält ihren Rename-Fast-Path. Veröffentlichung weiter ohne Ersetzen; bestehendes
   `sync_all` und Fehlerverhalten beim nachträglichen Quell-Cleanup erhalten
   (`src/record/finalize.rs:733`, `src/record/finalize.rs:735`).
4. Die Finalisierung und Sidecar-Moves auf diesen Helfer umstellen; Sidecars
   benutzen bisher denselben Move-Helfer (`src/record/cursor_sidecar.rs:263`).

**Risiken:** Ein Umzug darf weder den Same-Filesystem-Rename durch Kopieren
ersetzen noch nach einem erfolgreichen Commit die Zieldatei wegen fehlgeschlagener
Quellbereinigung entfernen. Nicht-UTF-8-Pfade dürfen durch das bisherige
`display()`-basierte Naming nicht weiter beschädigt werden
(`src/record/finalize.rs:722`). Power-loss-Garantien über den bestehenden Umfang
hinaus werden nicht behauptet.

**Roter Test:** fremde vorhandene Part-Datei bleibt nach Create-Fehler unverändert;
zwei per Barrier synchronisierte Kopien zum selben Ziel dürfen einander keine
Part-Datei entfernen. Ein Gewinner, ein sauberer `AlreadyExists`-Fehler, beide
Quellen bis zum jeweiligen erfolgreichen Move geschützt. Kein Timing per Sleep.
Vorhandene No-replace- und Cross-Filesystem-Tests übernehmen
(`src/record/finalize.rs:1476`, `src/record/finalize.rs:1551`).

**Verifikation:** gemeinsame Checks; zwei Clips gleichzeitig auf ein anderes
Dateisystem kopieren und beide Dateien vollständig dekodieren. Manueller
Shelf-/GUI-Check auf einem weiteren physischen Datenträger bleibt offen.

**Abschluss A1:**

- `copy_preserves_another_operations_partial_file` schlug vor dem Fix mit
  `NotFound` fehl: der alte Fehlerpfad hatte die fremde Datei gelöscht.
  Derselbe Test ist jetzt grün (`src/record/finalize.rs:1500`).
- `copy_noreplace` legt eigene Part-Dateien mit `create_new` an, kopiert und
  synchronisiert vor dem Rename und bereinigt nur den selbst angelegten Pfad
  (`src/platform/linux/files.rs:9`, `src/platform/linux/files.rs:23`).
- Ein Barrier-Test prüft zwei gleichzeitig geöffnete Kopien mit genau einem
  Gewinner; der Verlierer meldet `AlreadyExists`, Quellen bleiben erhalten,
  keine Part-Datei bleibt liegen (`src/platform/linux/files.rs:113`).
  Weitere Tests prüfen Lesefehler und nicht-UTF-8-Pfade
  (`src/platform/linux/files.rs:160`, `src/platform/linux/files.rs:181`).
- `cargo fmt --check`, `cargo test` und Linux-`cargo check` bestanden:
  322 Tests erfolgreich, zwei bestehende Tests ignoriert. Log:
  `/tmp/boltsnap-a1-tests-20260925.log`.
- Ein isolierter Harness bindet die echte `files.rs` ein und prüft einen
  vorab belegten PID-/Sequenz-Part-Namen sowie zwei parallele Video-Kopien
  von `/tmp` nach `/dev/shm`. Die vollständigen Kopien sind bytegleich;
  FFmpeg dekodiert Video und Audio ohne Fehler. Harness und Quelldateien:
  `/tmp/boltsnap-a1-media-1xtrt542`; Ergebnisse:
  `/dev/shm/boltsnap-a1-media-rb5xcig5`. Das prüft die Dateioperation, nicht
  die laufende Shelf-Oberfläche. Bestehender echter Cross-Filesystem-Move-Test
  ebenfalls grün (`src/record/finalize.rs:1547`).

### A2: Bild-Save ohne Überschreiben

**Phase 1: Verhalten und Ursache**

Ein Sekundentimestamp wird direkt mit `fs::copy` kombiniert
(`src/platform/linux/shelf/mod.rs:1835`, `src/platform/linux/shelf/mod.rs:1843`,
`src/platform/linux/paths.rs:290`). Ziel: Jeder abgeschlossene Save erhält einen
eigenen Namen, vorhandene Dateien und Symlinks bleiben unangetastet. Bevorzugtes
Namensschema bleibt `boltsnap-<Zeit>[-N].png`. Ein Fehler zeigt keinen Erfolgsblitz.
Die Bildkarte behält zunächst ihren temporären Ursprung; wiederholtes bewusstes
Speichern nach Abschluss erzeugt weiterhin eine neue Kopie. Diese Planungsannahme
erhält die bisherige Copy-Semantik (`src/platform/linux/shelf/mod.rs:1843`).

**Phase 2: Umsetzung**

1. Unter `src/platform/linux/shelf/save.rs` einen kleinen synchronen Bild-Save-
   Helfer für Zielwahl und Copy/Publish anlegen; von `save_card` aufrufen.
2. Namensbildung in Linux `paths.rs` für explizite Erweiterungen wiederverwenden
   bzw. schmal erweitern. Bestehende `.mp4`-Wrapper für ihre Aufrufer erhalten
   (`src/platform/linux/paths.rs:262`). Belegte Einträge einschließlich dangling
   Symlinks erkennen; die atomare No-replace-Veröffentlichung bleibt maßgeblich.
3. A1-Copy ohne Quelllöschung verwenden und bei Zielkollision mit dem nächsten
   Suffix erneut veröffentlichen. Keinen zweiten ungeschützten `copy`-Pfad bauen.

**Risiken:** Ein bloßes `exists()` schließt weder konkurrierende Writer noch
dangling Symlinks sicher aus; die bisherige Namenssuche prüft nur `exists()`
(`src/platform/linux/paths.rs:279`). Nach Fehler muss die temporäre Bildquelle
weiter zum Wiederholen verfügbar sein. A2 bleibt noch synchron; D2 entfernt
gezielt den UI-Block und hängt davon ab.

**Roter Test:** feste Uhrzeit, zwei verschiedene PNGs, gleicher Zielordner:
zwei erhaltene vollständige Dateien. Zusätzlich konkurrierender Writer zwischen
Namenswahl und Publish sowie belegter Name durch Symlink, ohne fremdes Ziel zu
verändern. Test der echten Save-Operation statt nur des Formatstrings.

**Verifikation:** gemeinsame Checks; manuell zwei Karten schnell hintereinander
speichern und beide Bilder sowie Fehlerrückmeldung bei nicht schreibbarem Ziel
prüfen.

**Abschluss A2:** `shelf/save.rs::save_image` legt das Zielverzeichnis an, öffnet
die Quelle einmal und veröffentlicht mit `files::copy_noreplace`; bei
`AlreadyExists` wird die Quelle zurückgespult und der nächste Suffix gewählt.
`paths::unique_saved_path` ersetzt die feste `.mp4`-Namenssuche durch eine mit
freier Endung und prüft mit `symlink_metadata`, erkennt also auch hängende
Symlinks; `unique_recording_path` bleibt als `.mp4`-Wrapper (A4 nutzt die neue
Funktion). `boltsnap_filename_ext` entfällt. Fehler melden sich jetzt auch per
Notification. Tests mit fester Uhrzeit: zwei Saves derselben Sekunde ergeben
`boltsnap-<t>.png` und `-1.png` mit jeweils eigenem Inhalt; ein hängender
Symlink auf dem ersten Namen bleibt unverändert und sein Ziel wird nicht
angelegt; fehlende Quelle legt nichts an. Mit der alten Logik (fester Name +
`fs::copy`) sind die ersten beiden rot. Die Verdrahtung in `save_card` bleibt
synchron (D2); manueller Check offen.

### A3: Karte während eines Saves vor Löschung schützen

**Phase 1: Verhalten und Ursache**

`saving_cards` unterbindet doppelte Saves, aber `remove_card_now` entfernt Karte,
Quelle und Save-Markierung trotzdem (`src/platform/linux/shelf/mod.rs:1823`,
`src/platform/linux/shelf/mod.rs:1760`, `src/shelf/model.rs:30`).

Empfehlung: Die Karte bleibt bis zum Ergebnis angeheftet. Close während Save
merkt einen Schließwunsch vor. Nach Erfolg darf die Karte verschwinden, die
permanente Datei bleibt. Nach Fehler bleibt die Karte mit Quelle und verständlicher
Meldung für erneutes Speichern verfügbar. Automatische Eviction überspringt
laufende Saves. Dies gilt später identisch für asynchrone Bildkopien.

**Phase 2: Umsetzung**

1. Linux-Shelf-Save-Zustand um einen gemerkten Close-Wunsch ergänzen, ohne einen
   neuen permanenten Zustand in `ShelfModel` einzuführen.
2. Close-Animation, `remove_card_now` und Eviction gegen diesen Zustand prüfen.
   Das bisherige Eviction-Verfahren berechnet seine Kandidaten ohne Save-Zustand
   (`src/platform/linux/shelf/mod.rs:318`); nur den anschließenden Remove-Aufruf
   zu überspringen reicht für konsistente Kapazitätsberechnung nicht.
3. `CardPromoted` behandelt Erfolg zuerst als Promotion, danach den Close-Wunsch.
   Bei Fehler Pin aufheben, Close-Wunsch verwerfen und Quelle behalten
   (`src/platform/linux/shelf/mod.rs:2930`, `src/shelf/model.rs:114`).
4. Cache-Grenzen weiter einhalten: Wenn alle entfernbaren Karten geschützte Saves
   sind, neue Arbeit mit bestehender Fehlerrückgabe ablehnen statt geschützte
   Quellen zu löschen oder unbegrenzt Karten anzusammeln. Nach Save-Ende erneut
   trimmen; die gerade fehlgeschlagene Karte nicht unmittelbar selbst evicten.

**Risiken:** Bildkopie und Video-Move brauchen unterschiedliche Erfolgsbehandlung;
eine Bildkopie darf nicht blind `model.promote` auslösen. Animationsabschluss und
Save-Abschluss können in beliebiger Reihenfolge eintreffen
(`src/platform/linux/shelf/mod.rs:2085`, `src/platform/linux/shelf/mod.rs:2930`).
Das Pinning garantiert keine Recovery nach Daemon-Absturz; RAM-only bleibt bestehen
(`README.md:259`).

**Roter Test:** Save vor Öffnen der Quelle per Channel anhalten, Close und
Cache-Eviction auslösen, dann Worker freigeben. Quelle und Sidecar bleiben bis
zum Abschluss vorhanden. Erfolg hinterlässt permanente Bytes; injizierter Fehler
lässt dieselbe Karte erneut speichern. Auch ein bereits laufender Close beim
Save-Start sowie erschöpfte Cache-Kapazität testen.

**Verifikation:** gemeinsame Checks; manuell Save+Close auf langsamem Ziel,
fehlgeschlagener Save und neue Karte während Save. Alternative aus Entscheidung 2:
sofortiges Verstecken braucht einen separaten Besitzer samt Wiederherstellung
der Karte bei Fehler und ist daher eine größere Änderung.

**Abschluss A3 (Entscheidung 2 A):** `shelf/save.rs::CardSaves` ersetzt
`saving_cards` und merkt Close-Wünsche. `remove_card_now` entfernt eine Karte
mit laufendem Save nicht, sondern merkt den Close vor; das deckt Close-Klick,
eine schon laufende Dismiss-Animation und alle weiteren Remove-Pfade ab. Der
Close-Klick startet für eine solche Karte keine Animation. Die Eviction zählt
laufende Saves mit, wählt sie aber nie als Kandidaten. `CardPromoted` beendet
den Save: Erfolg promotet, schließt bei vorgemerktem Close und trimmt den
Cache nach; ein Fehler verwirft den Close-Wunsch, die Karte bleibt mit Quelle
und Sidecar für einen neuen Versuch und wird nicht sofort nachgetrimmt.
**Abweichung von Schritt 4:** Neue Screenshots werden nicht abgelehnt, wenn nur
gespeicherte Karten entfernbar wären. Der Cache kann dann kurz um die Zahl
laufender Saves über den Grenzen liegen; das ist durch Klicks begrenzt und
endet mit dem Save. Tests: `CardSaves` (Close wartet auf Erfolg, Fehler behält
die Karte, Doppelstart abgelehnt) und die Eviction mit geschützten Karten. Die
Eviction-Signatur ist neu; der alte Code hätte die gespeicherte Karte 1
entfernt. Die Verdrahtung im Daemon braucht Wayland und ist nur per Code
geprüft; der manuelle Save+Close-Check auf langsamem Ziel bleibt offen. Bild-
Saves laufen bis D2 synchron und brauchen den Schutz noch nicht.

### A4: Containerendung und Drag-MIME erhalten

**Phase 1: Verhalten und Ursache**

`promote_recording` wählt immer einen `.mp4`-Namen und verschiebt unveränderte Bytes
(`src/record/finalize.rs:340`, `src/platform/linux/paths.rs:278`). Der Drag bietet
für jedes Video `video/mp4` an (`src/platform/linux/shelf/mod.rs:1907`). Ziel:
Replay bleibt `.mkv`, MP4 bleibt `.mp4`; beim reinen Save findet keine Umwandlung
statt. Bekannte Endungen bekommen den passenden angebotenen MIME-Typ, unbekannte
Dateitypen mindestens `text/uri-list`, ohne falsches MP4-Versprechen.

**Vertiefung auf `cf243b5`:** `DataSourceHandler::cancelled` erkennt ausschließlich
`.mp4` als Video (`src/platform/linux/shelf/mod.rs:3638`). Ein abgebrochener
Replay-MKV-Drag fällt deshalb in den Bild-Clipboard-Pfad. Die bereits bekannte
falsche Typbehandlung umfasst auch diesen Callback, nicht nur Save und MIME.

**Phase 2: Umsetzung**

1. `promote_recording` benutzt die erweiterungsfähige Linux-Namenswahl aus A2.
   Originale gültige Erweiterung erhalten; fehlende/ungeeignete Erweiterung
   konservativ als neutralen Dateinamen behandeln, nicht still MP4 behaupten.
2. MIME-Auswahl für bekannte `.mp4`, `.mkv`, `.webm` im Linux-Shelf kapseln.
   Nicht anhand der Endung echte Medienvalidierung behaupten. Kein neuer ffprobe-
   Aufruf pro Save oder Drag; bisherige Inputs werden nicht neu transkodiert.
3. Sidecar-Namensraum berücksichtigen: gleiche Basen mit verschiedenen Video-
   Erweiterungen dürfen nicht denselben `X.cursor.json` beanspruchen. Bei belegtem
   zugehörigem Sidecar einen anderen Basisnamen wählen; atomarer Sidecar-Move
   bleibt geschützt (`src/record/cursor_sidecar.rs:258`).
4. Replay-Dokumentation um unveränderten Container beim Shelf-Save ergänzen.
5. Den vorhandenen Kartentyp beim Drag-Start bis `clear_drag` behalten und im
   Cancel-Callback auswerten. Nicht erneut aus einer kurzen Endungsliste raten.
   `begin_drag`, `cancelled`, `dnd_finished` und alle Abbruchpfade zusammen prüfen.

**Risiken:** normale Recording-Finalisierung erzeugt ausdrücklich MP4
(`src/record/finalize.rs:240`); diese Pfade behalten ihre Endung. Bestehende falsch
benannte Saves nicht automatisch umbenennen. Die MIME-Angabe für Bytes und der
Dateiname in `text/uri-list` müssen übereinstimmen.

**Roter Test:** vorhandener Endungs-Repro; ergänzend echte synthetische MKV-Datei
vor/nach Promotion auf identische Bytes und ffprobe-Container prüfen. MIME-Auswahl
für MP4/MKV/WebM/unbekannt und Sidecar-Kollision testen.
Abgebrochener MKV-/WebM-Drag darf keine Bild-Clipboard-Operation auslösen;
der Screenshot-Fallback muss weiterhin funktionieren.

**Verifikation:** gemeinsame und Replay-Checks; manuell gespeicherten Replay-Clip
öffnen und in ein Ziel mit Datei- sowie Byte-Import ziehen.

**Abschluss A4:** `promote_recording` wählt den Namen über
`paths::unique_saved_path` mit der kleingeschriebenen Quellendung (1-8
ASCII-Alphanumerische), sonst ohne Endung; die Bytes wandern unverändert.
Die Namenssuche reserviert jetzt den ganzen Stamm: `X` ist belegt, sobald `X`
selbst, `X.cursor.json`, `X.mp4`, `X.mkv` oder `X.webm` existiert. Damit
beansprucht kein neuer Clip den Sidecar eines vorhandenen. `drag_mime_types`
bietet `video/mp4`, `video/x-matroska` oder `video/webm` nach Endung an,
unbekannte Videos nur `text/uri-list`. `drag_card` merkt Pfad und Kartentyp
beim Drag-Start bis `clear_drag`; `cancelled` wertet den Typ aus, also fällt
kein abgebrochener Video-Drag mehr in den Bild-Clipboard-Pfad. Tests: Promotion
von mkv/mp4/WebM/ungültiger/fehlender Endung (Bytes gleich, Endung erhalten,
Quelle weg; mit dem alten Code rot wie der Review-Repro), Namenssuche mit
belegtem Sidecar-Stamm und leerer Endung, MIME-Auswahl. Der Cancel-Pfad braucht
Wayland und ist nur per Code geprüft. Replay-Worker unverändert; deshalb keine
Worker-, `probe.py`- oder `live.py`-Läufe. Normale Finalisierung bleibt MP4.
Manueller Drag-/Save-Check offen.

### B1: Video-Import übernimmt reguläre Dateiinhalte

**Phase 1: Verhalten und Ursache**

Die Prüfung folgt einem Symlink, `hard_link` übernimmt anschließend unter Linux
den Link selbst (`src/platform/linux/shelf/mod.rs:483`,
`src/platform/linux/shelf/mod.rs:500`). Der isolierte Symlink-Repro schlägt fehl
(`docs/reviews/2026-09-25-repros.patch:50`).

Empfehlung: künftig einen vom Original unabhängigen Dateiinhalt übernehmen.
Die README beschreibt inzwischen korrekt den vorhandenen Hardlink; die frühere
Zusage einer eigenen Kopie war zu weitgehend. Eine echte Kopie bleibt der hier
vorgeschlagene, noch nicht beschlossene Fix. Relative/absolute Symlinks auf gültige reguläre Dateien sind
erlaubt. Nach erfolgreichem ACK darf Entfernen oder Überschreiben des Originals
den Shelf-Clip nicht verändern. Annahme: Der Produzent hält die Datei während des
Imports stabil; eine transaktionale Momentaufnahme eines gleichzeitig schreibenden
Fremdprozesses ist nicht Teil dieses Fixes.

**Phase 2: Umsetzung**

1. `prepare_shelf_video` öffnet die Quelle einmal und prüft die Metadaten dieses
   Handles. Typ-/Größenprüfung und Copy greifen damit auf dasselbe Objekt zu.
2. Hardlink-Fast-Path entfernen, regulären Inhalt über den geöffneten Handle in
   eine selbst angelegte Cache-Datei streamen. A1-Primitive bei passender Signatur
   wiederverwenden. Keine eigene Reflink-Bibliothek oder neue Abhängigkeit.
3. Größenlimit beim Kopieren begrenzen, vollständigen Abschluss vor ACK verlangen;
   nur selbst angelegte unvollständige Ziele auf Fehler bereinigen. Bestehende
   UID-Prüfung und Cache-/Disk-Prüfung erhalten
   (`src/platform/linux/shelf/mod.rs:646`, `src/platform/linux/shelf/mod.rs:490`).
4. README präzisiert: Quelle während des Imports stabil halten, danach unabhängig.

**Risiken:** echte Kopie kostet I/O und Platz, der bisherige Hardlink nicht.
Aufruf läuft bereits im Request-Worker, das Ergebnis geht über `VideoPrepared`
in die UI (`src/platform/linux/shelf/mod.rs:644`). Keine Aussage, dass die
bisherige Cache-Vorabprüfung eine globale Reservation gegen alle parallelen
Writer wäre (`src/platform/linux/shelf/mod.rs:493`).

**Roter Test:** relativer und absoluter Symlink, gültige reguläre Datei, leere
Datei/Verzeichnis/dangling Link. Nach ACK Original ändern bzw. wegbewegen und
Inhalt des Imports prüfen. Kopierfehler darf weder Erfolg noch fremde Dateilöschung
erzeugen. Tests bekommen ein injizierbares Cache-Verzeichnis statt globaler
Environment-Mutationen in parallel laufenden Tests.

**Verifikation:** gemeinsame Checks; Import realer kleiner und größerer Clips,
Thumbnail/Öffnen/Save prüfen, I/O-Zeit gegenüber Hardlink-Baseline berichten.
Alternative Entscheidung 3: aufgelöste reguläre Datei weiter hardlinken, aber dann
explizit keine Unabhängigkeit gegenüber In-place-Änderungen versprechen.

**Abschluss B1 (Entscheidung 3 A):** `import_shelf_video` öffnet die Quelle einmal
(Symlinks werden aufgelöst, `O_NONBLOCK`, damit eine FIFO nicht hängt), prüft
Typ und Größe auf diesem Handle, lässt die Cache-/Reserveprüfung über einen
injizierbaren `reserve`-Callback laufen und kopiert genau die geprüfte Größe mit
`files::copy_noreplace` in eine neue Cache-Datei. Weicht die veröffentlichte
Größe ab, wird die eigene Datei entfernt und ein Fehler gemeldet. Kein
Hardlink mehr; `paths::rec_file` entfällt. Tests mit eigenem Cache-Verzeichnis:
reguläre Datei, relativer und absoluter Symlink ergeben reguläre Kopien, die
eine In-place-Änderung des Originals nicht sehen (mit dem alten Hardlink rot,
der relative Symlink war dort sogar hängend); leere Datei, Verzeichnis,
hängender Symlink, FIFO und abgelehnte Reservierung legen nichts an. Kosten:
eine volle Kopie statt Hardlink; I/O-Zeit gegen die Hardlink-Baseline nicht
gemessen. README angepasst.

### C1: Kein erneuter Import nach unklarem ACK

**Phase 1: Verhalten und Ursache**

Jedes `UnexpectedEof` führt zum erneuten mutierenden Request ohne Thumbnail
(`src/platform/linux/ipc.rs:90`). Der Socket-Repro zeigt zwei Requests nach
einer unvollständigen Erfolgsantwort (`docs/reviews/2026-09-25-repros.patch:68`).
Ziel: höchstens ein mutierender Importversuch pro Nutzeraktion; alte Daemons
erhalten weiterhin plain `add`. Fehlt die Bestätigung nach Senden, ist das Ergebnis
unbekannt und wird als solches gemeldet, nicht durch Retry oder Erfolg kaschiert.

**Phase 2: Umsetzung**

1. Vor dem Senden eines vorhandenen Thumbnails eine nicht mutierende
   Kompatibilitätsabfrage durchführen. Den bestehenden `RecordingStatus`-Request
   verwenden: er liefert eine gerahmte `Response`
   (`src/platform/linux/shelf/mod.rs:1604`). `Ping` ungeeignet umdeuten: dessen
   Antwort ist heute ungeframtes `PONG` (`src/platform/linux/shelf/mod.rs:1595`).
2. In der gemeinsamen Response-Serialisierung eine optionale Thumbnail-Capability
   ergänzen, fehlend bedeutet false. Nur die Linux-Statusantwort setzt sie.
   Konstruktoren behalten ihren Default; Parser bleibt tolerant gegenüber fehlenden
   und unbekannten Feldern (`src/protocol.rs:174`, `src/protocol.rs:214`).
3. Auf separater Verbindung genau einmal `add_thumb` oder plain `add` senden.
   EOF/Timeout/Reset danach niemals automatisch erneut importieren. Auch ein
   Daemon-Wechsel zwischen Probe und Mutation darf kein mutierendes Retry auslösen.
4. `add_with` und dessen alten Fallback-Test auf sichere Vorabwahl umstellen
   (`src/platform/linux/ipc.rs:86`, `src/platform/linux/ipc.rs:377`). Keine globale
   Capability-Cache-Invalidierung oder Request-ID-Deduplizierung einführen.

**Risiken:** eine zusätzliche lokale Roundtrip auf dem Screenshot-Pfad. Alte
Daemons ohne Flag verlieren nur die Thumbnail-Optimierung, nicht die Screenshot-
Funktion. Probe-Fehler nicht als positives Capability-Signal interpretieren.
Ein neuer Client muss bei einem unklaren Ergebnis den vorhandenen Screenshot
erhalten; Verhalten des aufrufenden CLI einschließlich Fehlermeldung bei der
Umsetzung vollständig nachverfolgen. Dieser Teil ist noch eine offene
Implementierungsprüfung, keine bereits bestätigte Garantie.

**Roter Test:** Repro auf Anzahl mutierender Requests statt bloßer Verbindungen
anpassen; Capability-Probe ist keine Mutation. Vollständig verlorenes ACK,
teilweise Antwort, Timeout, altes fehlendes Flag, neues Flag und Daemon-Wechsel
testen. Portable Parser-Roundtrips ohne OS-`cfg`.

**Verifikation:** gemeinsame Checks; neue und alte Daemon-Antworten in isolierten
Socket-Tests, Screenshot-Latenz mit/ohne Probe messen. Alternative Entscheidung 4:
EOF-Fallback schlicht entfernen, dann kann ein alter laufender Daemon bis zum
Neustart keine Thumbnail-Requests neuer Clients verarbeiten. Nicht empfohlen.

**Abschluss C1 (Entscheidung 4 B, gegen die Empfehlung):** Das mutierende Retry
ist entfernt. `add_with` nimmt genau einen Stream und sendet einmal; ein EOF vor
der Antwort wird als `UnexpectedEof` mit dem Hinweis gemeldet, dass die Karte
fehlen kann und ein älterer Daemon einen Neustart braucht. Das CLI legt den
Screenshot vorher als `last.png` ab und nennt diesen Pfad in der Fehlermeldung.
Keine Capability-Abfrage, keine zusätzliche Roundtrip. Folge: Nach einem Install
ohne Daemon-Neustart landen Screenshots bis zum Neustart nicht im Shelf. Test:
Ein Fake-Daemon nimmt das `add_thumb`-Frame, schickt die halbe Antwort und
schließt die Schreibseite; der Client meldet den Fehler, und auf dem Socket
kommt danach nichts mehr an. Der alte Rückfall-Test entfällt; der alte Code hätte
hier ein zweites Frame gesendet (Review-Repro `repros.patch:68`).

### D1: Drag-Dateien streamen

**Phase 1: Verhalten und Ursache**

Der nicht-URI-Pfad liest die gesamte Datei in einen Vec und hält sie während
`write_all` (`src/platform/linux/shelf/mod.rs:3617`). Ziel: Dateitransfer mit
konstantem kleinen Userspace-Puffer pro Transfer, unabhängig von Clipgröße und
Lesegeschwindigkeit des Empfängers. Bestehender Worker und URI-Pfad bleiben.

**Phase 2: Umsetzung**

1. In `DataSourceHandler::send_request` nur `fs::read` plus `write_all` durch
   `File::open` plus `io::copy` im vorhandenen Thread ersetzen.
2. Fehler/BrokenPipe beenden den Transfer und schließen die Handles. Für Tests
   nur bei Bedarf einen kleinen Transfer-Helfer extrahieren, keine neue Queue.

**Risiken:** Blockieren durch langsame Empfänger bleibt im Transfer-Thread, nicht
im UI-Thread (`src/platform/linux/shelf/mod.rs:3609`). Ein gleichzeitig vor dem
Öffnen entfernter Quellpfad bleibt ein eigener Lifecycle-Aspekt; aus A3 keine
allgemeine Drag-Lifetime-Garantie ableiten. Unbegrenzte Anzahl Threads wird durch
Streaming allein ebenfalls nicht zu einem globalen Ressourcenlimit.

**Roter Test:** begrenzter Read-Adapter bzw. separater Testprozess mit langsamer
Pipe zeigt vor dem Fix vollständiges Einlesen und danach Streaming, plus gleiche
Bytes und vorzeitig geschlossener Empfänger. RAM-Messung nur im isolierten Prozess
mit kontrollierter Fixture, nicht die Desktop-Sitzung mit GiB belasten.

**Verifikation:** gemeinsame Checks; Datei- und Byte-Drop kleiner/großer Videos
und PNGs, Abbruch des Zielprogramms; RSS darf nicht proportional zur Datei wachsen.

**Abschluss D1:** `send_file` streamt mit `File::open` plus `io::copy` im
vorhandenen Transfer-Thread; der URI-Pfad bleibt. Test mit 1-MiB-Datei: gleiche
Bytes, größter einzelner Write höchstens 64 KiB (mit der alten Logik rot: ein
Write von 1.048.576 Bytes); geschlossener Empfänger und fehlende Datei enden
mit Fehler. Keine RSS-Messung im Daemon; manueller Drop großer Videos offen.

### D2: Bild-Save außerhalb des Eventloops

**Phase 1: Verhalten und Ursache**

Bild-Save führt `create_dir_all` und vollständige Kopie im Klickhandler aus,
Video-Save hat bereits einen Worker (`src/platform/linux/shelf/mod.rs:1828`,
`src/platform/linux/shelf/mod.rs:1839`). Ziel: Shelf-/Recording-Steuerung bleibt
während langsamer Bild-I/O bedienbar. Erfolg erst nach vollständigem Publish;
doppelte Klicks während desselben Jobs starten keine weiteren Kopien.

**Phase 2: Umsetzung**

1. A2-Helfer in einem Save-Worker aufrufen und A3-Schutz für Bildkarten aktivieren.
   Insbesondere auch Verzeichniserstellung und Namenssuche im Worker ausführen.
2. Completion-Event für Copy-Erfolg von Video-Promotion unterscheiden. Bildkarte
   behält ihren Originalpfad; kein versehentlicher Wechsel zu Permanent. Das
   bestehende Video-Event ruft `model.promote` auf
   (`src/platform/linux/shelf/mod.rs:2935`).
3. UI-Thread verwaltet ausschließlich Job-Start, Ergebnis, Fehlermeldung,
   Erfolgsblitz und vorgemerkten Close. Kein zusätzlicher Thumbnail-Decode.

**Risiken:** doppelte Save-Jobs, zu früher Erfolgsblitz und Close während Save.
A3 ist deshalb zwingend vorher fertig. Bild-Save bleibt eine Kopie; zusätzliche
Bild-Promotion wäre eine separate Verhaltensänderung, die nicht benötigt wird.

**Roter Test:** injizierte blockierende Copy-Operation hält den Worker an,
während eine weitere Event-/Control-Anfrage bearbeitet wird. Währenddessen keine
Erfolgsmeldung und kein zweiter Save; nach Erfolg genau ein Ziel. Bei Fehler
erneutes Speichern möglich, Close-/Eviction-Tests aus A3 auch für Bilder.

**Verifikation:** gemeinsame Checks; großes PNG und langsames Testziel, während
des Saves andere Karte anklicken und Recording-Control bedienen. Relative
Responsivität messen, keinen unbelegten pauschalen Millisekundenwert versprechen.

### E1: Recovery nach Finalisierung ohne kaputtes Resume

**Phase 1: Verhalten und Ursache**

Combined ersetzt Output-Gruppen durch `None`; Fehler beim anschließenden Move
liefert diesen Zwischenstand zurück. Der Daemon übernimmt ihn und erlaubt wieder
Resume (`src/record/finalize.rs:93`, `src/record/finalize.rs:134`,
`src/platform/linux/shelf/mod.rs:2921`, `src/record/session.rs:256`). Neue
Output-Segmente passen nicht mehr dazu. Das trifft insbesondere den Multi-Stream-
Fallback bei gemischten Skalen/großem Canvas (`src/platform/linux/gsr.rs:40`).

Empfehlung: Ein fehlgeschlagener tatsächlicher Finalisierungsversuch wechselt in
einen Recovery-Zustand mit erneutem Shelf-/Disk-Save und Discard, aber ohne Resume.
Normales Pause/Resume sowie Recorder-Stop-Fehler vor der Finalisierung bleiben
weiter fortsetzbar. Der Nutzer sieht, dass die Aufnahme vor einer neuen Aufnahme
zuerst gespeichert oder verworfen werden muss. Bereits erfolgreich gespeicherte
Teile bleiben unangetastet.

**Phase 2: Umsetzung**

1. In der Linux-Session eine separate Recovery-Phase bzw. gleichwertige eindeutige
   Policy einführen. `can_accept`, `begin_finalize`, `begin_discard`, `resume` und
   Statusabbildung gemeinsam anpassen (`src/record/session.rs:209`,
   `src/record/session.rs:221`, `src/record/session.rs:246`).
2. Nur `DaemonEvent::Finalized(Err)` setzt diese Policy. Der vorangehende
   `ChildrenStopped`-Fehler darf nicht versehentlich gleich behandelt werden:
   beide verwenden bisher `finalize_failed`
   (`src/platform/linux/shelf/mod.rs:2866`, `src/platform/linux/shelf/mod.rs:2922`).
3. Resume in UI und Backend konsistent sperren, bevor Recorder gespawnt werden
   (`src/platform/linux/shelf/mod.rs:2654`, `src/platform/linux/shelf/mod.rs:2671`).
   Popup zeigt den eingeschränkten Zustand; Save/Discard bleiben klickbar.
4. Gemeinsame reine Popup-Helfer um selektive Resume-Deaktivierung ergänzen,
   bisherige APIs als Wrapper erhalten: Windows nutzt die alten Signaturen
   (`src/shelf/paint.rs:335`, `src/shelf/recording.rs:55`,
   `src/platform/windows/shelf.rs:647`, `src/platform/windows/shelf.rs:796`).
   Keine Windows-Implementierung ändern.
5. Bestehende öffentliche Statusnamen erhalten, Recovery über Fehlertext erklären.
   `actions_enabled` bedeutet weiterhin, dass Aktionen verfügbar sind; konkrete
   Resume-Anfragen werden nachvollziehbar abgelehnt. Kein neues öffentliches
   OS-spezifisches Enum erzwingen (`src/protocol.rs:90`).
6. Erneuter Save benutzt ausschließlich `recoverable_segments`. Bereits fertige
   `completed`-Clips nicht duplizieren; vorhandener Teil-Erfolg-Test ist die Basis
   (`src/record/finalize.rs:384`, `src/record/finalize.rs:1441`). Sidecar-Erhalt beim
   Retry explizit prüfen, ohne unnötiges erneutes Compose.

**Risiken:** Einschränkung des Nutzerverhaltens nach Fehlern, deshalb Entscheidung 1.
Normales Resume darf nicht gesperrt werden. Alle Session-Matches und Popup-
Aufrufer mitprüfen; keine globale Abschaltung aller Buttons durch das bisherige
eine `enabled`-Flag (`src/shelf/paint.rs:386`).

**Roter Test:** Repro von Combined->Disk-Fehler über echte Session-API erweitern:
Resume wird vor Spawn abgelehnt, anschließender Shelf-Save bzw. reparierter
Disk-Save gelingt, ursprüngliche Recovery-Datei bleibt bis Erfolg erhalten.
Der bestehende Repro fordert direktes Resume und muss für die beschlossene
Policy angepasst werden (`docs/reviews/2026-09-25-repros.patch:10`). Zusätzlich
normales Pause/Resume, Stop-Fehler, teilweise erfolgreiche separate Disk-Saves,
Sidecar-Retry und UI-Hit-/Renderzustand testen.

**Verifikation:** gemeinsame Checks; synthetische echte Videos mit zwei Outputs
und provoziertem Disk-Fehler speichern, Retry vollständig dekodieren. Manuell
Recovery-Popup prüfen. Keine Änderung von Capture-Timing oder Encoderwahl.

**Abschluss E1 (Entscheidung 1 A):** `RecordingSession::save_failed` ruft
`finalize_failed` und setzt `resume_blocked`; nur `DaemonEvent::Finalized(Err)`
verwendet es. Der `ChildrenStopped`-Fehler bleibt bei `finalize_failed` und damit
fortsetzbar. `can_accept(Resume)` und `resume` lehnen ab, bevor ein Recorder
startet; `handle_recording_action` meldet „cannot resume after a failed save“.
`last_error` erklärt den Zustand, Phase und öffentliche Statusnamen bleiben
(`Paused`, `actions_enabled` true). Popup: `popup_hit_with` und
`draw_recording_popup_with` nehmen `recovery`; Titel „SAVE FAILED“, Resume
gedimmt und ohne Treffer. Die alten Signaturen bleiben als Wrapper für die
unveränderte Windows-Shell. Tests: Save/Discard erlaubt, Resume abgelehnt und
Phase unverändert; Stop-Fehler bleibt fortsetzbar; Popup-Treffer im
Recovery-Zustand; echter `finalize_recording`-Retry nach Combined->Disk-Fehler
nur mit `recoverable_segments` landet bytegleich auf der Disk und nimmt
`cursor.json` mit. Ohne das Flag ist der Session-Test rot. Der Retry-Test war
schon vorher grün und schützt den Weg, der jetzt der einzige ist. Kein
UI-Render-Test; manueller Check des Recovery-Popups offen. README angepasst.

**Alternative bei Entscheidung 1:** Resume nach jedem Fehler erhalten. Dafür
Finalize transaktional gestalten, Originalgruppen und Sidecar-Zeitachsen bis zur
vollständigen Veröffentlichung behalten und Teil-Erfolge von separaten Saves
beim nächsten Resume sauber abgrenzen. Die jetzigen Helpers löschen nach
Concat/Compose bereits ihre Eingänge (`src/record/finalize.rs:263`,
`src/record/finalize.rs:315`). Das erfordert eine größere Dateilebensdaueränderung,
zusätzliche Disk-Kapazität sowie Multi-Output-/Sidecar-Integrationstests; dafür
vor Umsetzung E1 neu konkretisieren, nicht nebenbei erweitern.

### F1: Cursor-Handover beim Monitorwechsel

**Phase 1:** Unter dem bisherigen Vertrag kein Fix nötig. Leave setzt den
Spring-Target auf None; neues Enter startet an der neuen Position
(`src/record/cursor_motion.rs:158`, `src/record/cursor_motion.rs:349`). Der ältere
Plan nennt Output-Wechsel explizit als Snap-Fall
(`docs/plans/2026-09-23-performance-functionality.md:210`). Der rote Kontinuitäts-
Repro allein belegt deshalb keine verletzte freigegebene Anforderung.

**Phase 2:** keine Codeänderung, kein Fix-Commit. Den Repro als dokumentierte
Verhaltensprüfung aufbewahren und nicht unverändert als angebliche Regression
übernehmen. Bei Wunsch nach nahtlosem Übergang erst einen eigenen Plan für
Tracker-Reihenfolge, echtes Leave, mehrere Displays und Latenz erstellen.
Dann portable Motion-Tests, Plugin-Tests einschließlich GL und echte
Monitorwechsel prüfen; das ist ausdrücklich außerhalb dieser neun Fixes.

### G1: Daemon-Eigentümerschaft vor Socket- und Cache-Cleanup

**Priorität / Nachweis:** P1, auf `cf243b5` statisch durchgängig bestätigt.
Keine absichtliche Race gegen den laufenden Desktop ausgelöst.

**Ursache und Auswirkung:** `run_daemon` prüft zuerst `daemon_alive`, entfernt
dann den vermeintlich alten Socket und bereinigt Shelf-Dateien, Recording-Cache
und Audiomixe (`src/platform/linux/shelf/mod.rs:1053`). Erst nach Wayland-Aufbau
bindet es den Socket (`:1180`). `ipc::ensure_daemon` kann beim direkten Fallback
mehrere Prozesse starten (`src/platform/linux/ipc.rs:38`). Zwei Starts können
beide die anfängliche Prüfung passieren. Ein verzögerter zweiter Start kann
anschließend den Socket und temporäre Medien des inzwischen laufenden ersten
Daemons entfernen. Der erste Listener lebt ohne erreichbaren Pfad weiter;
der zweite Prozess kann später am bereits belegten Replay-Socket scheitern.
Ein erfolgreicher Ping ist kein exklusiver Eigentumsnachweis.

**Fix und Abwägung:** ein Linux-Lock über die gesamte Daemon-Lebensdauer, vor
jedem Cleanup und vor Logrotation. `flock` auf einer stabilen privaten Lockdatei
ist kleiner als ein neues Startprotokoll. Die Datei nach Freigabe nicht unlinken,
sonst können verschiedene Inodes gleichzeitig gelockt werden. Ein früheres
Socket-Bind allein löst die sichere Behandlung eines alten Socketpfads nicht.

**Implementierungsplan:**

1. In `src/platform/linux/ipc.rs` einen kleinen RAII-Lock mit bestehendem `libc`
   vorsehen; Pfadauflösung über `linux/paths.rs`. Kein OS-Code in portable Module.
   Eigenen privaten Laufzeitpfad, sichere Dateieröffnung und `CLOEXEC` verwenden.
2. In `run_daemon` zuerst Lock erwerben, dann Alive-/Stale-Prüfung und Cleanup.
   Verlierer starten keine Bereinigung. Fremde oder unerwartete Pfadtypen nicht
   als vermeintlich alten Socket entfernen. Bind-Fehler dürfen keinen aktiven
   Socket eines anderen Besitzers unlinken.
3. Lock bis zum tatsächlichen Daemon-Ende halten. Alle frühen Fehlerpfade geben
   nur den eigenen Lock frei. Aufrufer aus CLI, `ensure_daemon` und systemd
   prüfen; gleichzeitige Clients müssen den Gewinner weiter erreichen können.
4. Erst nach diesem Startschutz den Cleanup-Fix G2 umsetzen. Keine Audio-/Replay-
   Refaktoren neben diesem Startschutz.

**Regression und Abnahme:** zwei isolierte Prozesse per Barrier vor Lock-Erwerb
starten; nur einer erreicht einen injizierten Cleanup-Zähler und Bind. Ein
verzögerter Verlierer darf den Gewinner nicht beeinträchtigen. Lock-Freigabe bei
normalem Exit und Crash, Stale-Socket-Recovery und unerwarteten Pfadtyp prüfen.
Keine echten Benutzerdateien als Lösch-Fixtures. Gemeinsame Linux-Checks;
später separater Cold-Start-Test über CLI und systemd, kein Neustart während
anderer laufender Aufnahmen. **Status: umgesetzt, Cold-Start-Test offen.**

**Abschluss G1:** `ipc::lock_daemon` nimmt `flock(LOCK_EX|LOCK_NB)` auf
`boltsnap.lock` neben dem Socket (`O_NOFOLLOW`, `0600`, `CLOEXEC` über std).
`run_daemon` holt den Lock vor Log-Rotation, Alive-Prüfung und Cleanup; ein
Verlierer beendet sich ohne Nebenwirkung. Die Alive-Prüfung bleibt für Daemons
von vor dem Lock. `remove_stale_socket` entfernt nur echte Sockets; eine
Datei oder ein Symlink am Socketpfad bricht den Start mit Fehler ab. Tests:
zwei per Barrier gestartete Lock-Versuche ergeben genau einen Besitzer, nach
Freigabe ist der Lock wieder frei; alter Socket wird entfernt, Datei und
Symlink bleiben. Die Tests prüfen neue Funktionen und waren vorher nicht
kompilierbar statt rot. Die Verdrahtung in `run_daemon` braucht Wayland und
ist nur per Code geprüft.

### G2: Replay-Remux-Partials bei Neustart erkennen

**Priorität / Nachweis:** P2, reine Namensprüfung reproduziert.
`worker/src/export.rs:42` erzeugt mit `create_partial` Namen wie
`clip-123-456-1.mkv.123.0.partial`. Bei Remux-/Sync-/Publish-Fehler bleiben sie
absichtlich als Diagnoseartefakt erhalten (`:92`).
`linux/paths.rs:230` erkennt dagegen nur `boltsnap-*` oder `clip-*.mkv`.
Remux-Partials enden auf `.partial`, bleiben also auch nach Neustart liegen.
Die Cache-Quota zählt reguläre Dateien mit (`src/record/finalize.rs:657`);
wiederholte Fehler können dadurch die 2-GiB-Grenze dauerhaft belegen.
Crop-Partials mit Endung `.partial.mkv` sind von dieser Namenslücke nicht betroffen.

**Fix und Abwägung:** bestehende Remux-Namen eng erkennen. Nicht pauschal jedes
`clip-*` löschen und keine neue laufende Garbage Collection hinzufügen. Ein
einheitliches neues Namensschema wäre ebenfalls möglich, müsste aber auch bereits
zurückgelassene Dateien erkennen und würde den Fix unnötig vergrößern.

**Implementierungsplan:**

1. `is_orphan_rec_file` um genau das erzeugte Remux-Schema erweitern: numerische
   PID/Zeitstempel/Sequenz-Felder sowie exakte `.mkv.<pid>.<seq>.partial`-Suffixe.
   Produzenten in `worker/src/live.rs`, `export.rs` und `crop.rs` abgleichen.
2. Vorhandene fertige Clip-/Recording-Erkennung erhalten. Startbereinigung erst
   unter dem G1-Lock; Partials einer laufenden Session dürfen nicht erfasst werden.
3. README/Replay-Fehlerdokumentation zwischen Erhalt bis zum Neustart und danach
   stattfindender Bereinigung unterscheiden. Keine manuelle Reparatur behaupten.

**Regression und Abnahme:** der beigefügte G2-Test ist aktuell rot. Ergänzend
fertige MKV-, Crop-Partial-, Remux-Partial-Namen, kaputte Nummernfelder und
unbeteiligte `clip-notes.txt` abdecken. Synthetisch einen Exportfehler erzeugen,
Artefakt bis zum Ende der Session erhalten und nach kontrolliertem exklusivem
Neustart die vorgesehene Auswahl zur Bereinigung prüfen. Gemeinsame Linux- und
Worker-Checks. **Status: geplant.**

### G3: Prozess-Deadline auch für stdout/stderr einhalten

**Priorität / Nachweis:** P2, reproduziert. Bei einer Deadline von 40 ms kehrt
`output` mit `Ok` erst nach **1,201791424 s** zurück. Der gestartete Shell-Prozess
ist bereits beendet, sein endlicher `sleep`-Nachkomme hält die Pipes offen.

**Ursache und Auswirkung:** `replay/process.rs:66` begrenzt das Warten auf den
direkten Child-Prozess, ruft anschließend aber unbeschränkt `join` auf zwei
`read_to_end`-Threads auf (`:102`). Das Byte-Limit begrenzt keinen wartenden Read.
Ein Wrapper mit langlebigem Nachkommen kann Font-/Hyprland-Setup, Thumbnail- oder
Medien-Probes trotz Timeout unbegrenzt festhalten. Aufrufer liegen in
`linux/hypr.rs`, `shelf/font.rs`, `shelf/thumbnail.rs`, `gsr.rs`, `replay/mod.rs`
und `worker/src/crop.rs`; `output_setup` teilt denselben Fehler.

**Fix und Abwägung:** den vorhandenen kleinen Runner auf nichtblockierende
Ausgabepipes und `poll` mit einer absoluten Deadline umstellen. Erfolg benötigt
Prozessstatus und abgeschlossene Pipe-Lektüre. Ein bloßes zeitbegrenztes Warten
auf Reader-Threads würde blockierte Threads zurücklassen. Fremde Nachkommen
nicht anhand unbekannter PIDs töten; Prozessgruppen wären eine eigene Policy.

**Implementierungsplan:**

1. In `replay/process.rs` stdout/stderr gleichzeitig begrenzt lesen, Deadline
   über Child-Wait und beide Streams gemeinsam auswerten. Byte-Limits und
   `output_setup`-Latenz erhalten; große stderr-Ausgabe darf stdout nicht blockieren.
2. Bei Fristablauf eigene Pipe-Enden schließen und Timeout zurückgeben, auch
   wenn der direkte Child schon erfolgreich beendet ist. Noch laufenden eigenen
   Child mit klar begrenzter Eskalation reap-en. Diese zusätzliche Cleanup-Frist
   ausdrücklich dokumentieren; keine unbegrenzten Joins.
3. Alle genannten Aufrufer auf Fehlerweitergabe prüfen. Keine stillschweigende
   Verwendung unvollständiger Probe-Ausgabe als erfolgreiches Ergebnis.
4. Worker-Mitbenutzung desselben Moduls separat bauen und prüfen; keine zweite
   fast identische Prozessimplementierung hinzufügen.

**Regression und Abnahme:** beigefügter G3-Repro ist rot. stdout und stderr
jeweils durch einen Nachkommen offen halten; erfolgreicher direkter Child darf
die Deadline nicht umgehen. Normaler Output bleibt bytegleich; Output-Limit,
leerer Output, direkt hängender Prozess, sauberes Reaping und kein Thread-Leak
prüfen. Der Review-Repro benutzt einen endlichen 1,2-s-Child; produktiver Test
soll kontrollierte Synchronisation und großzügige Scheduler-Toleranz verwenden.
Gemeinsame Linux- und Worker-Checks. **Status: geplant.**

### G4: RecordingWatch-Verbindungen begrenzen und abräumen

**Priorität / Nachweis:** P2, durchgängiger Codepfad bestätigt; keine absichtliche
FD-Erschöpfung im laufenden Daemon erzeugt.

**Ursache und Auswirkung:** `RecordingWatch` legt nach der ersten Statuszeile
jeden Stream in `watchers` ab (`linux/shelf/mod.rs:1610`). Der Request-Permit
endet mit der Eventbearbeitung, begrenzt also keine dauerhaften Subscriptions.
Entfernt werden Watches erst beim Schreiben eines geänderten Recording-Status
(`:3144`). Im Idle bleibt dieser Status gleich. Wiederholtes Verbinden, Lesen
der ersten Zeile und Schließen sammelt deshalb daemonseitige FDs bis zum nächsten
Statuswechsel. Auch lebende Watches haben kein Limit. Bei genügend Wiederholungen
scheitern neue Socket-/Dateioperationen, nicht nur die Statusanzeige.

**Fix und Abwägung:** separates kleines Subscription-Limit plus Bereinigung
geschlossener Peers. Empfehlung für den kleinsten belastbaren Eingriff: vor jeder
neuen Subscription und bei Statuspublikation vorhandene Streams per nonblocking
Peek prüfen, EOF entfernen und anschließend das Limit anwenden. So bleiben auch
ohne Recording-Aktivität höchstens die begrenzte Anzahl Rest-FDs erhalten.
Sofortige Freigabe jedes Idle-Disconnects über calloop-HUP ist möglich, benötigt
aber zusätzliche Registrierungsverwaltung. Keinen periodischen Idle-Timer nur
für dieses Cleanup einführen.

**Implementierungsplan:**

1. Watch-Verwaltung als kleine Linux-interne Einheit mit festem Limit, z. B. 16,
   kapseln. Annehmen erst nach Peer-Prüfung und Kapazitätsprüfung; Überschreitung
   nachvollziehbar ablehnen. Watch-Permits von kurzlebigen Request-Limits trennen.
2. Peek muss `WouldBlock` als weiter verbunden, EOF als getrennt behandeln.
   Definieren, wie unerwartete Clientdaten behandelt werden; keine unbegrenzte
   Leseschleife. Bestehende Behandlung langsamer/fehlgeschlagener Writes erhalten.
3. `RecordingWatch`, `publish_recording_snapshot`, Watch-CLI und Protokolltests
   gemeinsam prüfen. Statusänderungen weiterhin sofort publizieren; kein Polling
   im UI-Thread und kein zusätzliches öffentliches OS-spezifisches Protokoll.

**Regression und Abnahme:** über echte Socketpaare viele Subscribe/Close-Runden
ohne Statuswechsel ausführen; Zahl gehaltener FDs bleibt begrenzt und ein neuer
Client kommt nach altem Disconnect hinein. Lebende Clients bis zum Limit,
Ablehnung darüber, BrokenPipe und partielle Writes testen. Gemeinsame Checks;
später Widget-Reconnect mit idle Daemon beobachten. **Status: geplant.**

### G5: Replay-Auswahl vor automatischem Restart schützen

**Priorität / Nachweis:** P2. Synthetischer Worker-Repro bestätigt einen noch
exportierbaren Snapshot bei `ready=false` und `busy=false`. Dass der Watchdog
diesen Zustand neu startet, folgt aus seinem geprüften Codepfad; der echte
Supervisor wurde für das Review nicht gestartet oder neugestartet.

**Ursache und Auswirkung:** `worker/src/live.rs:159` meldet `busy` während
Preview/Export, nach fertiger Preview aber wieder false (`:185`). `frozen`
bleibt für bis zu 120 Sekunden erhalten und kann auch nach Input-Ausfall noch
gespeichert werden (`:229`). Der Watchdog prüft nur `ready || busy`
(`linux/replay/mod.rs:253`). Nach drei negativen Prüfungen im 5-s-Raster verwirft
er die Session. Eine offene Auswahl verliert so vor ihrem regulären Ablauf die
einzige eingefrorene Aufnahme. Auch der manuelle Startpfad prüft nur `ready`
(`:637`) und muss dieselbe Eigentumspolitik beachten.

**Fix und Abwägung:** gehaltene Auswahl explizit im Worker-Status melden, etwa
als `selection_active`, und in der Supervisor-Restartentscheidung berücksichtigen.
Nicht `busy` für die gesamte Auswahl setzen: Save/Cancel-Dispatch benutzt diesen
Wert bereits als Ausschluss gegen laufende Jobs. Eingangsfehler bleiben sichtbar;
der Schutz muss mit Cancel, Verbrauch oder bestehender TTL enden.

**Implementierungsplan:**

1. `worker/src/live.rs` um das additive Statusfeld aus tatsächlichem `frozen`
   ergänzen. Lifecycle für Previewfehler, Save, Cancel, TTL und Stop durchgehen.
2. Supervisor-Watchdog schützt `ready || busy || selection_active`; der explizite
   Startpfad darf eine gültige Auswahl oder einen Export ebenfalls nicht unbemerkt
   ersetzen. Explizites Stop bleibt wirksam. Worker und Supervisor gemeinsam
   ausliefern; fehlendes Feld alter Worker als false behandeln und die verbleibende
   Einschränkung bei gemischten Versionen dokumentieren.
3. Schutz im Zeitpunkt der Ersetzung erneut prüfen: zwischen Statusabfrage und
   `guard.take()` kann ein Freeze eintreffen. Die Entscheidung mit der bestehenden
   Start-/Session-Verwaltung serialisieren; keine neue globale Polling-Schleife.
4. `docs/replay.md` um den Vertrag für Auswahl, Capture-Ausfall und TTL ergänzen.

**Regression und Abnahme:** beigefügter Python-Repro friert synthetisches Video
ein, wartet auf EOF und speichert den Snapshot erfolgreich, bevor die aktuell rote
Statusassertion ausgewertet wird. Anschließend Status/Restart-Policy testen:
Auswahl bleibt bis Save/Cancel/TTL geschützt; danach darf der Watchdog einen
toten Input ersetzen. Zeit in Tests injizieren, nicht 120 Sekunden schlafen.
Race Freeze-vor-Ersetzung gezielt synchronisieren. Worker-Checks und bestehende
`probe.py`/`live.py`; später tatsächliche Auswahl bei kontrolliertem Capture-Ausfall
prüfen. **Status: geplant.**

### H1: Nix-Paketversion an Cargo koppeln

**Priorität / Nachweis:** P3, `flake.nix:54` setzt für das Hauptpaket `0.4.3`,
`Cargo.toml` setzt `1.0.1`. Das Nix-Paket trägt dadurch veraltete Metadaten.
Kein Nix-Build zur Feststellung nötig; Runtime-Funktionsfehler nicht behauptet.

**Fix und Implementierungsplan:** beim Hauptpaket die Version mit
`(builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version` beziehen.
Kein neuer Versionsgenerator und keine Änderung der separaten Cursor-Plugin-
Version `0.1.0`. Die Nutzerreferenzen in README und Codebase-Inventar aktualisieren.

**Verifikation:** Nix-Evaluation von `.#packages.x86_64-linux.default.version`
mit Cargo vergleichen, danach vorhandenen Nix-Paketcheck/-Build ausführen, sofern
eine passende Nix-Umgebung vorhanden ist. Keine Hostinstallation als Ersatz für
eine Prüfung. Gemeinsame Projektchecks weiterhin ausführen.
**Status: geplant; Nix-Evaluation und -Build noch offen.**

## 4. Entscheidungen

1. **E1, Fortsetzen nach fehlgeschlagener Finalisierung:**
   A: Recovery nur Save/Discard, normales Pause/Resume unverändert.
   B: Resume auch nach Finalisierungsfehlern, mit transaktionaler Überarbeitung.
   **Empfehlung A**, geringster Eingriff und keine zusätzliche Originalkopie.
2. **A3, Close während Save:** A: Karte bis Abschluss anheften, Close merken;
   nach Fehler sichtbar behalten. B: sofort verstecken, Job übernimmt Medien und
   rekonstruiert die Karte auf Fehler. **Empfehlung A**, weniger Zustandsverwaltung.
3. **B1, eigener Videoimport:** A: unabhängige Kopie mit mehr I/O.
   B: Symlink korrekt auflösen, Hardlink erhalten und In-place-Änderungen des
   Originals ausdrücklich nicht absichern. **Empfehlung A** für unabhängigen Dateiinhalt. Die README dokumentiert
   inzwischen den vorhandenen Hardlink und verspricht keine unabhängige Kopie.
4. **C1, alte Daemons:** A: additive optionale Capability in der vorhandenen
   Statusantwort, eine Vorab-Roundtrip. B: mutierendes Retry entfernen und
   Thumbnail-Kompatibilität mit alten Daemons bis zu deren Neustart aufgeben.
   **Empfehlung A**, bestehendes Linux-Verhalten möglichst erhalten.
5. **F1, Cursor:** A: bestehendes Snap-Verhalten beibehalten, kein Fix.
   B: nahtlosen Handover separat spezifizieren. **Empfehlung A** für diesen Plan.

Alle übrigen technischen Entscheidungen im Plan sind kleine Implementierungs-
vorgaben und benötigen keine zusätzliche Einzelabstimmung. Bei einem späteren
Go die gewählten Optionen hier festhalten. Ändert eine Erkenntnis den Umfang
anderer Items, zuerst den Plan korrigieren und die Abweichung klären.

**Beschlossen vom Nutzer am 25.09.2026 (Umsetzungsauftrag für alle Items):**
1 A (nur Save/Discard), 2 A (Karte anheften), 3 A (unabhängige Kopie),
4 **B** (mutierendes Retry streichen, gegen die Empfehlung), 5 A (kein
Cursor-Fix).

## 5. Status

A1 ist implementiert und geprüft. Es bleibt bei einer Fähigkeit pro Commit,
ID im Commit und Projektchecks vor jedem Commit; keine AI-/Agent-Coauthor-Trailer.
Der Commit-Titel identifiziert den A1-Abschluss inklusive dieses Dokuments.
Alle anderen Items sind offen. F1 erhält unter Empfehlung A keinen Code-Commit.

| Welle | ID | Item | Commit |
| --- | --- | --- | --- |
| 1 | A1 | eigene Part-Dateien / atomare Veröffentlichung | `A1: isolate partial files for atomic saves` (dieser Commit) |
| 1 | A2 | Bild-Save ohne Überschreiben | `A2: never overwrite on image save` |
| 1 | A3 | Schutz laufender Saves vor Dismiss/Eviction | `A3: keep a card until its save ends` |
| 2 | E1 | sicherer Recovery-Zustand nach Finalisierung | `E1: allow only save or discard after a failed save` |
| 3 | A4 | Containerendung und Drag-MIME | `A4: keep the container on save and drag` |
| 3 | B1 | unabhängiger regulärer Video-Import | `B1: copy imported videos instead of linking them` |
| 3 | C1 | sichere IPC-Kompatibilitätswahl | `C1: send a screenshot to the shelf only once` |
| 4 | D1 | Streaming beim Drag | `D1: stream dragged files` |
| 4 | D2 | asynchroner Bild-Save | |
| keine | F1 | Cursor-Verhalten beibehalten | |
| 0 | G1 | exklusiver Daemon-Start vor Cleanup | `G1: lock the daemon before cleanup` |
| 5 | G2 | Remux-Partials bei exklusivem Neustart erkennen | |
| 5 | G3 | Deadline inklusive Prozessausgabe | |
| 5 | G4 | begrenzte und bereinigte Recording-Watches | |
| 5 | G5 | gehaltene Replay-Auswahl vor Restart schützen | |
| 6 | H1 | Nix-Version aus Cargo lesen | |

## 6. Vertieftes Review und tatsächlich ausgeführte Checks

**Scope:** Linux-Start und IPC vom CLI bis zum Eventloop, Dateiimport und
Shelf-Save/Close/Drag, Recording-Stop/Recovery/Finalisierung, Replay-Supervisor,
Worker-Freeze/Export/Fehlerpfade, Prozessrunner und Paketregistrierung. Betrachtet
wurden Aufrufer und Eigentums-/Fehlerübergänge, nicht nur einzelne Funktionen.
Kein vollständiges Audit aller vendorten Bibliotheken oder Windows-Implementierungen.

**Parallele Arbeit:** Der Nutzer bestätigte laufende Änderungen an
`src/record/session.rs`. Dieser Pfad wurde nicht bearbeitet. Alle Builds und Tests
dieser Vertiefung liefen im separaten Archiv-Snapshot
`/tmp/boltsnap-deep-review-20260925-7KTRFG`, mit eigenem Target-Verzeichnis.
Ausgangsbasis war `a430ceb`; nach dem konkurrierenden Commit wurde die feststehende
Revision `cf243b5` übernommen. Die neuen Stop-/Reaper-Pfade wurden mitgelesen und
deren vier Tests mitgeprüft. Kein Test gegen bewegliche gemeinsame Build-Eingaben.
Der gsr-Saved-Path-Fast-Path ist jetzt implementiert; reale Stop-Latenz und
Recorder-/Cursor-Abnahme wurden hier nicht gemessen.

| Prüfung | Ergebnis und Grenze |
| --- | --- |
| `cargo fmt --check`, `cargo test --locked -j 4` auf unverändertem Snapshot `cf243b5` | Grün: 23 Library- und 303 Binary-Tests, zwei standardmäßig ignoriert |
| Zusätzliche Rust-Repros G2/G3 im Snapshot | Zwei erwartete Fehler; keine Änderung am Produktcode im Arbeitsverzeichnis |
| Worker `cargo build --locked -j 4` im Snapshot | Debug-Build erfolgreich; bestehende Warnung zu unbenutztem `output_setup` |
| Synthetischer Freeze/EOF/Save-Repro G5 mit diesem Worker | Save erfolgreich, Statusassertion erwartungsgemäß rot; keine Desktop-Aufnahme |
| G1/G4 und H1 | Code-/Manifestbelege; keine Live-Race, FD-Erschöpfung oder Nix-Ausführung |
| Dokumentation und Review-Artefakte | `git diff --check`, lokale Markdown-Ziele/Anker und Python-Syntax geprüft; Rust-Repro-Patch formatiert und erneut mit denselben zwei erwarteten Fehlern ausgeführt |

Reproduzierbare Review-Artefakte sind versionierbar neben diesem Plan:
[Rust-Repros](2026-09-25-deep-review-repros.patch) und
[synthetischer Freeze-Repro](2026-09-25-replay-freeze-repro.py).
Der Patch wird ausschließlich auf einen separaten Checkout von `cf243b5`
angewendet. Dort `cargo test --locked --bin boltsnap deep_review_ -- --nocapture`
ausführen. Der Python-Repro benötigt FFmpeg mit libx264 und den separat gebauten
Worker: `python3 docs/plans/2026-09-25-replay-freeze-repro.py --worker /absoluter/pfad/boltsnap-replay-worker`.
Beide Aufrufe sollen vor den Fixes fehlschlagen. Sie sind Belegmaterial, noch
keine in die reguläre Testsuite aufgenommenen Regressionstests.

Logs liegen im Snapshot als `cf243b5-tests.log`, `repros.log`,
`repros-formatted.log`, `worker-build.log` und `freeze-repro.log`.
Der Freeze-Repro behält seine generierten Medien und
`report.json` unter `/tmp/boltsnap-freeze-review-mxr8cus4`. Keine dieser lokalen
Ausgabedateien ist Bestandteil des Repositories. Die Fixes bleiben geplant;
grüne bestehende Tests widerlegen die zusätzlich nachgewiesenen Lücken nicht.
