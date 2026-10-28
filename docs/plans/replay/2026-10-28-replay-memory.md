# Replay-Speicher: ehrliche Anzeige, dann weniger (2026-10-28)

**Anlass:** Vergleich mit Rewa (60 MB unter Windows), Befund im Nachtrag von
[2026-10-28-replay-both-displays.md](2026-10-28-replay-both-displays.md).
Nutzer 28.10.: „okay dann machen wir 1 und planen danach 2“.

1. Die Last-Anzeige zählt nur, was dem Puffer gehört (dieser Plan, Teil 1).
2. Konstante Qualität mit Byte-Deckel statt konstanter Bitrate (Teil 2, erst
   geplant, dann geprobt).

## Teil 1: Speicher als privater Speicher

Stand: **gebaut 28.10., wie geplant.** `load.rs` `private` und
`private_of_rollup`; Tests `private_memory_is_the_clean_and_dirty_pages_no_one_else_maps`,
Prozessbaum-Test mit Rollup und ohne (statm-Rückfall). `replay_load_now`
gegen den laufenden Puffer (beide Monitore): 33 % eines Kerns, **336 MB**
(gsr 158, Worker 170, kms-Helfer per statm); vorher zeigte die Zeile ~560 MB
RSS. fmt, clippy `-D warnings`, `cargo test` 164 + 415 grün. Live im Fenster
offen (neues Binary, Fenster neu öffnen).

### Befund

`linux/app/load.rs` `resident` summiert RSS aus `/proc/<pid>/statm`. RSS
zählt jede eingeblendete Seite, auch geteilte Bibliotheken, die andere
Prozesse genauso nutzen und die beim Beenden des Puffers nicht frei würden.
Live mit beiden Monitoren (`smaps_rollup`, 28.10.):

| Prozess | RSS | privat (Clean + Dirty) | davon anonym |
| --- | --- | --- | --- |
| gsr | 250 MB | 159 MB | 19 MB |
| Worker | 187 MB | 170 MB | 166 MB |
| `gsr-kms-server` | 1,7 MB | nicht lesbar (Capabilities) | |

Die 121 MB `/dev/nvidiactl` von gsr sind privat eingeblendeter Treiber-
speicher; sie zählen als privat mit (der Treiber gibt sie beim Beenden frei).

### Verhalten

- Die Zahl rechts in „Load“ ist die Summe des **privaten** Speichers der
  Puffer-Prozesse: `Private_Clean + Private_Dirty` aus
  `/proc/<pid>/smaps_rollup` (USS, das, was beim Beenden frei wird; wie der
  private Working Set unter Windows). Text und Layout bleiben gleich.
- Nicht lesbares `smaps_rollup` (der `gsr-kms-server` mit Capabilities, ein
  Kernel ohne die Datei): für diesen Prozess `statm` resident − shared (sein
  anonymer Teil), damit er nicht verschwindet.
- Prozess weg zwischen Suche und Lesen: fehlt in der Summe wie heute.
- Kosten: ein Lesen von `smaps_rollup` für gsr dauert ~3 ms (gemessen
  28.10.), einmal je Sekunde für drei Prozesse.

### Code

- `src/platform/linux/app/load.rs`: `resident` → `private` (Rollup lesen und
  parsen, Rückfall auf `statm`), Doku-Kommentare oben und an `Meter`.
- `docs/replay.md` Abschnitt Load: „private memory“.

### Abnahme

- Test: Rollup-Zeilen → Summe von Private_Clean und Private_Dirty in Bytes;
  ohne Rollup → statm (resident − shared) × Seite; der bestehende
  Prozessbaum-Test liest über `private`.
- `replay_load_now` (ignoriert, liest nur `/proc` des laufenden Puffers):
  Zahl ≈ Summe der privaten Spalte oben (~330 MB statt ~440 MB RSS).
- fmt, clippy, `cargo test`.

**Macht nicht:** keine andere CPU-Messung, kein VRAM, keine neue Zeile oder
Erklärung im UI, kein Sparen (das ist Teil 2).

## Teil 2: Konstante Qualität mit Byte-Deckel

Stand: **zu planen** (nach Teil 1).
