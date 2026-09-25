# Review-Fixes: Spezifikation und Implementierungsplan

Stand: 25.09.2026. Status: A1 freigegeben und umgesetzt; übrige Items geplant.
Basis: `7e736ad`. Erstellt mit dem lokalen Skill
`/home/mt/.codex/skills/spec-plan/SKILL.md`.

## 1. Zusammenfassung und Grenzen

Dieser Plan behandelt alle neun Befunde aus
[dem Linux-Review](../reviews/2026-09-25-linux-review.md) sowie die dort separat
aufgeführte Cursor-Verhaltensfrage. Die
[isolierten Repros](../reviews/2026-09-25-repros.patch) sind Ausgangspunkte für
Regressionstests, kein fertig übernehmbarer Test-Patch. Insbesondere enthält
der Container-Test Platzhalterbytes, keine echte Matroska-Datei
(`docs/reviews/2026-09-25-repros.patch:33`).

Die Empfehlungen in Abschnitt 4 sind Planungsannahmen, keine bereits
getroffenen Nutzerentscheidungen. Nach Erläuterung der lokalen atomaren
Dateiveröffentlichung wurde A1 zur Umsetzung freigegeben. Die übrigen Items
bleiben geplant. Keine Installation und kein Eingriff in den laufenden Daemon.
Die Arbeit erfolgt ohne Subagents gemäß der Projektvorgabe des Nutzers.
Ursachenbelege und unveränderte Planungsabschnitte zitieren die Basisrevision;
der A1-Abschluss unten nennt die aktuellen Stellen.

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

## 2. Wellen und Abhängigkeiten

| Welle | IDs in Reihenfolge | Ergebnis |
| --- | --- | --- |
| 1 | A1, A2, A3 | sichere Dateiveröffentlichung, kein Überschreiben, Schutz laufender Saves |
| 2 | E1 | fehlgeschlagene Finalisierung bleibt bedienbar und speicherbar |
| 3 | A4, B1, C1 | richtige Mediennamen, zuverlässiger Import, kein mutierendes IPC-Retry |
| 4 | D1, D2 | begrenzter Transferspeicher und responsive Bild-Saves |
| keine | F1 | Entscheidung dokumentieren, vorerst kein Produktcode |

A1 stellt die Kopier-/Veröffentlichungsprimitive für A2 bereit. A3 muss vor D2
stehen, damit asynchrone Bild-Saves nicht dieselbe Dateilebensdauer-Race erhalten.
D2 verwendet die A2-Operation unverändert weiter. A4 und D1 bearbeiten denselben
Drag-Pfad nacheinander, sind fachlich aber getrennte Fähigkeiten. B1 kann die
Dateioperationen aus A1 wiederverwenden. E1 braucht eine Produktentscheidung und
gezielte UI-/Session-Tests, wird wegen P1 vor die übrigen Komfort-/Performance-
Korrekturen gezogen. Keine Welle als vollständige Fehlerbereinigung ausgeben,
solange andere Befunde offen sind.

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

### A4: Containerendung und Drag-MIME erhalten

**Phase 1: Verhalten und Ursache**

`promote_recording` wählt immer einen `.mp4`-Namen und verschiebt unveränderte Bytes
(`src/record/finalize.rs:340`, `src/platform/linux/paths.rs:278`). Der Drag bietet
für jedes Video `video/mp4` an (`src/platform/linux/shelf/mod.rs:1907`). Ziel:
Replay bleibt `.mkv`, MP4 bleibt `.mp4`; beim reinen Save findet keine Umwandlung
statt. Bekannte Endungen bekommen den passenden angebotenen MIME-Typ, unbekannte
Dateitypen mindestens `text/uri-list`, ohne falsches MP4-Versprechen.

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

**Risiken:** normale Recording-Finalisierung erzeugt ausdrücklich MP4
(`src/record/finalize.rs:240`); diese Pfade behalten ihre Endung. Bestehende falsch
benannte Saves nicht automatisch umbenennen. Die MIME-Angabe für Bytes und der
Dateiname in `text/uri-list` müssen übereinstimmen.

**Roter Test:** vorhandener Endungs-Repro; ergänzend echte synthetische MKV-Datei
vor/nach Promotion auf identische Bytes und ffprobe-Container prüfen. MIME-Auswahl
für MP4/MKV/WebM/unbekannt und Sidecar-Kollision testen.

**Verifikation:** gemeinsame und Replay-Checks; manuell gespeicherten Replay-Clip
öffnen und in ein Ziel mit Datei- sowie Byte-Import ziehen.

### B1: Video-Import übernimmt reguläre Dateiinhalte

**Phase 1: Verhalten und Ursache**

Die Prüfung folgt einem Symlink, `hard_link` übernimmt anschließend unter Linux
den Link selbst (`src/platform/linux/shelf/mod.rs:483`,
`src/platform/linux/shelf/mod.rs:500`). Der isolierte Symlink-Repro schlägt fehl
(`docs/reviews/2026-09-25-repros.patch:50`).

Empfehlung: den in `README.md:272` zugesagten eigenen Dateiinhalt unabhängig vom
Original übernehmen. Relative/absolute Symlinks auf gültige reguläre Dateien sind
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
   Originals ausdrücklich nicht absichern. **Empfehlung A**, passend zur Zusage
   einer eigenen Kopie (`README.md:272`).
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

## 5. Status

A1 ist implementiert und geprüft. Es bleibt bei einer Fähigkeit pro Commit,
ID im Commit und Projektchecks vor jedem Commit; keine AI-/Agent-Coauthor-Trailer.
Der Commit-Titel identifiziert den A1-Abschluss inklusive dieses Dokuments.
Alle anderen Items sind offen. F1 erhält unter Empfehlung A keinen Code-Commit.

| Welle | ID | Item | Commit |
| --- | --- | --- | --- |
| 1 | A1 | eigene Part-Dateien / atomare Veröffentlichung | `A1: isolate partial files for atomic saves` (dieser Commit) |
| 1 | A2 | Bild-Save ohne Überschreiben | |
| 1 | A3 | Schutz laufender Saves vor Dismiss/Eviction | |
| 2 | E1 | sicherer Recovery-Zustand nach Finalisierung | |
| 3 | A4 | Containerendung und Drag-MIME | |
| 3 | B1 | unabhängiger regulärer Video-Import | |
| 3 | C1 | sichere IPC-Kompatibilitätswahl | |
| 4 | D1 | Streaming beim Drag | |
| 4 | D2 | asynchroner Bild-Save | |
| keine | F1 | Cursor-Verhalten beibehalten | |
