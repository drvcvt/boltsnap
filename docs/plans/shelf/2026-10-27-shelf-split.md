# Shelf-Daemon aufteilen (2026-10-27)

**Anlass:** `src/platform/linux/shelf/mod.rs` hat 3478 Zeilen (Limit 1000,
Ziel ≤ 500) und darf nicht wachsen. Replay C1 (Karte sofort im Shelf,
[Replay-Plan §13](../replay/2026-09-20-replay-implementation.md)) braucht
neuen Code genau dort. Der Nutzer hat am 27.10. „erst planen, dann umsetzen“
gesagt.

## Verhalten

Keins ändert sich. Reine Verschiebung: kein Umbenennen, keine Logik, keine
neuen Tests, kein Pixel anders. Was sich ändern darf: `use`-Listen pro Datei
und `pub(super)` an Funktionen und Methoden, die nun in einem Nachbarmodul
stehen.

## Entscheidung: Ordner für Dateien, Module flach

Alle Teile sind `impl Daemon` (ein Zustand, ein Thread). Echte Untermodule
(`shelf::recording::controls`) würden für jede Methode, die von außen
gerufen wird, eine weitere Sichtbarkeit erzwingen (`pub(crate)` oder
`pub(in crate::platform::linux::shelf)`, etwa 40 Stellen), und die schon
vorhandenen `anim.rs`, `cards.rs`, `thumbnail.rs`, `save.rs`, `watch.rs`
müssten ihre `super::`-Pfade ändern. Deshalb bleiben alle Module direkte
Kinder von `shelf` und werden mit `#[path = "…"]` aus Ordnern geladen
(Vorbild im Repo: der Replay-Worker, `worker/src/main.rs:4`). `pub(super)`
reicht dann überall, die vorhandenen Dateien bleiben byte-gleich.

## Aufteilung (Zeilen aus `mod.rs` vor dem Split)

| Datei (Modul) | Inhalt | ca. Zeilen |
| --- | --- | --- |
| `mod.rs` | Moduldeklarationen, `Daemon`, `AfterStop`, `notify`, `dispatch_timeout`, `prep_shelf_compositor_rules`, `log_to_file_when_detached`, `run_daemon`, `mod tests` | 470 |
| `events.rs` (`events`) | `DaemonEvent`, `handle_daemon_event` | 270 |
| `clients.rs` (`clients`) | Socket-Leser/-Schreiber, `reject_busy`, `handle_client_request`, `write_response`, Video-Import (`prepare_shelf_video`, `import_shelf_video`, `validate_recording_dimensions`) | 370 |
| `placement.rs` (`placement`) | Fokus/Monitore (`parse_focus_snapshot` … `focused_monitor_origin`, `monitor_for_geometry`, `card_output`), `cached_monitors`, `target_output`, `place_on_output`, `relayout` | 220 |
| `tray.rs` (`tray`) | `tray_snapshot`, `start_tray`, `publish_tray_snapshot`, `apply_show_frame_pref`, `handle_tray_action` | 140 |
| `cards/actions.rs` (`actions`) | `PreparedAdd`, Cache-Grenzen und `shelf_cache_evictions_with_limits`, `add_prepared`, `trim_shelf_cache`, `remove_card_now`, `copy_card`, `save_card`, `on_click`, Save-Flash, `hit_test`, `draw` | 340 |
| `cards/{cards,anim,thumbnail,save}.rs` | unverändert, nur verschoben | – |
| `recording/start.rs` (`start`) | `prepare_recording_cache`, `default_start_plan`, `resolve_output_name`, `transformed_mode_size`, `requested_audio`, `spawn_initial_segment`, `cleanup_audio_async`, `start_recording*` | 275 |
| `recording/session.rs` (`session`) | `tick_recording`, `refresh_focus`, ausstehende Controls, `handle_recording_action`, `stop_children` | 250 |
| `recording/controls.rs` (`controls`) | Marker und Popup (anlegen, zeichnen, entfernen, Klicks, Tasten, Verwerfen-Bestätigung), `ControlsOpen`, `recording_controls_visible`, `recording_action_label` | 340 |
| `recording/finish.rs` (`finish`) | Watch-Clients (`MAX_RECORDING_WATCHERS`, `watcher_connected`), `start_finalize_worker`, `finish_discard`, `add_finalized_cards`, `recording_snapshot`, `publish_recording_snapshot` | 170 |
| `recording/watch.rs` | unverändert, nur verschoben | – |
| `wayland/handlers.rs` (`handlers`) | Compositor, Output, LayerShell, Seat, Shm, Registry, `delegate_*` | 205 |
| `wayland/input.rs` (`input`) | `PressState`, Pointer, Keyboard | 195 |
| `wayland/dnd.rs` (`dnd`) | `send_file`, `drag_mime_types`, `begin_drag`, `clear_drag`, DataSource/-Device/-Offer | 240 |

Danach: `shelf/` 9 Dateien und 4 Ordner, `cards/` 5, `recording/` 5,
`wayland/` 3; jede Datei unter 500 Zeilen.

## Umsetzung

1. Ein Skript schneidet `mod.rs` an den Item-Grenzen (Doc-Kommentare und
   Attribute gehören zum folgenden Item) in die Dateien oben; Methoden landen
   in je einem `impl Daemon`-Block pro Datei.
2. `git mv` für die fünf vorhandenen Dateien, `#[path]` in `mod.rs`.
3. Compiler-geführt: fehlende `use` pro Datei aus den „consider importing“-
   Hinweisen, `pub(super)` wo ein Nachbar ruft, ungenutzte `use` in `mod.rs`
   weg. Tests (`tests/mod.rs`, `use super::*`) bekommen ihre Pfade über
   Importe in `mod.rs` oder eigene `use`-Zeilen.

**Was brechen kann:** private Items, die Tests direkt nutzen (Compiler
meldet es); `delegate_*`-Makros, die am Typ in `mod.rs` hängen müssen
(gleiches Modul nicht nötig, Prüfung durch Build); Doc-Kommentare, die beim
Schneiden am falschen Item landen (Prüfung 3).

## Abnahme

1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` ohne
   Befund, `cargo test` mit denselben Zahlen wie vorher (158 + 396 grün,
   2 + 6 ignoriert).
2. Erhaltung: jede nichtleere Zeile des alten `mod.rs` steht, bis auf ein
   vorangestelltes `pub(super) ` und `use`-Zeilen, unverändert in genau
   einer neuen Datei (Skript, Abweichungen einzeln angesehen).
3. Zeilen- und Dateigrenzen wie oben.
4. Keine Live-Prüfung nötig (kein Verhalten), kein Windows-Code berührt.

## Macht nicht

Kein Umbenennen, keine Zusammenlegung doppelter Logik, kein Umzug der Tests
in die neuen Dateien, keine Änderung an `cards.rs`, `anim.rs`,
`thumbnail.rs`, `save.rs`, `watch.rs`, `settings.rs`, `surface.rs`,
`font.rs`, `limits.rs`, `tests/`.

## Stand

Umgesetzt 27.10. wie geplant, mit zwei Abweichungen: `mod.rs` hat 527 statt
~470 Zeilen (die `#[cfg(test)]`-Importe für die Tests und die Re-Exporte für
die unveränderten Geschwister kamen dazu; unter 1000, über dem Ziel 500, der
größte Rest ist `run_daemon`), und die Importe wurden aus der alten
`use`-Liste plus den verschobenen Items erzeugt, nicht aus den
Compiler-Hinweisen (ungenutzte danach mit `cargo fix` im Test-Profil
entfernt, `PopupButton` nur unter `cfg(test)`).

Checks: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`
ohne Befund; `cargo check` in Dev- und Test-Profil ohne Warnung; `cargo
test` 158 + 396 grün, 2 + 6 ignoriert (gleich wie vorher). Erhaltung: 3247
Code-Zeilen des alten `mod.rs` normiert verglichen; 12 Zeilen fehlen nur
scheinbar, alle sind von rustfmt umbrochen, weil `pub(super) ` oder
`super::super::` sie verlängert haben (Gegenstücke vorhanden); dazu kommen
nur Modul-Deklarationen, Kommentare, `impl Daemon {`/`}` je Datei.
`cards/{cards,anim,thumbnail,save}.rs` und `recording/watch.rs` byte-gleich
verschoben (`git status`: `R` ohne Änderung). Größte Datei `clients.rs` 384,
Ordner 9/5/5/3 Dateien.
