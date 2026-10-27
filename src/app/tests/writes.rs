//! Rows write what changed, and only that (settings plan, Phase 5b).

use std::time::{Duration, Instant};

use super::{SETTLED, app_on, block, control, focus, key, prefs, press};
use crate::app::pages::{self, Notice};
use crate::app::{App, Block, Write};
use crate::settings::{self, schema};
use crate::ui::text::Fonts;
use crate::ui::widgets::Value;
use crate::ui::{Effect, Input, Key, Rect};

const SIZE: (f32, f32) = (1000.0, 700.0);

fn changed(effects: &[Effect]) -> Vec<usize> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Changed(block) => Some(*block),
            _ => None,
        })
        .collect()
}

fn middle(rect: Rect) -> (f32, f32) {
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

#[test]
fn a_switch_writes_its_key_once() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    let row = block(&app, "Show frame");
    let at = middle(control(&app, "Show frame", now));
    let effects = press(&mut app, at, now);
    assert_eq!(changed(&effects), vec![row]);
    assert_eq!(
        app.write(row),
        Some(Write::Key(
            "record_show_frame",
            settings::Value::Bool(false)
        ))
    );
    // Hovering, focusing and leaving do not write again.
    let later = now + SETTLED;
    assert!(changed(&app.handle(Input::Move { x: 1.0, y: 1.0 }, later)).is_empty());
    assert!(changed(&key(&mut app, Key::Tab, later)).is_empty());
}

#[test]
fn a_dropdown_writes_the_value_it_lists_and_a_segment_its_word() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    focus(&mut app, "Frame rate", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Down, now);
    let effects = key(&mut app, Key::Enter, now);
    let fps = block(&app, "Frame rate");
    assert_eq!(changed(&effects), vec![fps]);
    // 120 in the test prefs, so the next entry is 144.
    assert_eq!(
        app.write(fps),
        Some(Write::Key("record_fps", settings::Value::Int(144)))
    );
    focus(&mut app, "Cursor", now);
    key(&mut app, Key::Right, now);
    assert_eq!(
        app.write(block(&app, "Cursor")),
        Some(Write::Key(
            "record_cursor",
            settings::Value::Text("mellow".into())
        ))
    );
}

#[test]
fn a_dragged_slider_writes_at_most_every_100_ms_and_its_last_value_on_release() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let row = block(&app, "Card size");
    let track = control(&app, "Card size", now);
    let y = track.y + track.h / 2.0;
    let mut writes = 0;
    let press = Input::Press {
        x: track.x + 10.0,
        y,
        clicks: 1,
    };
    writes += changed(&app.handle(press, now)).len();
    for step in 1..=10 {
        let at = now + Duration::from_millis(10 * step);
        let x = track.x + 10.0 + 8.0 * step as f32;
        writes += changed(&app.handle(Input::Move { x, y }, at)).len();
    }
    assert!(writes <= 2, "throttled, {writes} writes in 100 ms");
    let last = app.value(row);
    let effects = app.handle(Input::Release, now + Duration::from_millis(101));
    let wrote = changed(&effects) == vec![row] || app.value(row) == last;
    assert!(wrote, "the last value goes out on release");
    let Some(Write::Key("shelf.card_width", settings::Value::Int(width))) = app.write(row) else {
        panic!("card size writes shelf.card_width");
    };
    assert_eq!(Some(Value::Int(width)), app.value(row));
}

#[test]
fn a_text_field_writes_when_its_edit_is_committed() {
    let now = Instant::now();
    let mut app = app_on("screenshots", SIZE, now);
    focus(&mut app, "Save folder", now);
    let typed = app.handle(Input::Text("x".into()), now);
    assert!(changed(&typed).is_empty(), "not while typing");
    let effects = key(&mut app, Key::Enter, now);
    let row = block(&app, "Save folder");
    assert_eq!(changed(&effects), vec![row]);
    assert_eq!(
        app.write(row),
        Some(Write::Key(
            "save_dir",
            settings::Value::Text("~/Pictures/boltsnapx".into())
        ))
    );
}

#[test]
fn closing_commits_the_text_being_edited() {
    let now = Instant::now();
    let mut app = app_on("screenshots", SIZE, now);
    focus(&mut app, "Save folder", now);
    app.handle(Input::Text("y".into()), now);
    assert_eq!(changed(&app.finish(now)), vec![block(&app, "Save folder")]);
}

#[test]
fn the_replay_buffer_switch_starts_and_stops_the_buffer() {
    let now = Instant::now();
    let mut app = app_on("replay", SIZE, now);
    let row = block(&app, "Replay buffer");
    let at = middle(control(&app, "Replay buffer", now));
    press(&mut app, at, now);
    assert_eq!(app.write(row), Some(Write::Replay(true)));
}

#[test]
fn every_bound_row_writes_a_value_its_key_accepts() {
    let now = Instant::now();
    let mut checked = 0;
    for page in pages::pages(&prefs(), None, now) {
        let mut app = App::new(Fonts::fallback(), vec![page], SIZE, now);
        for index in 0..app.page().blocks.len() {
            let Some(Write::Key(path, value)) = app.write(index) else {
                continue;
            };
            let key = schema::find(path).unwrap_or_else(|| panic!("{path} is no key"));
            settings::load::check(key, &value).unwrap_or_else(|e| panic!("{path}: {e}"));
            checked += 1;
        }
        // Nothing has changed yet, so nothing went out.
        assert!(changed(&app.finish(now)).is_empty());
    }
    // Rows on their default with nothing to write (open with, replay display
    // and bitrate) remove their key instead and are not counted.
    assert_eq!(checked, 27, "every row but the replay buffer has a key");
}

#[test]
fn a_locked_notice_turns_every_row_off_and_a_reload_keeps_the_page() {
    let now = Instant::now();
    let notice = Notice {
        text: "config.toml has an error on line 3, fix it to change settings here".into(),
        locked: true,
    };
    let mut app = app_on("recording", SIZE, now);
    app.reload(pages::pages(&prefs(), Some(&notice), now), now);
    assert_eq!(app.page().name, "Recording");
    assert!(matches!(app.page().blocks[0], Block::Banner { .. }));
    for index in 0..app.page().blocks.len() {
        assert_eq!(app.write(index), None, "block {index} is off");
    }
}

#[test]
fn a_click_on_a_switch_rows_name_switches_and_other_names_do_nothing() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    let name_of = |app: &App, label: &str| {
        let index = block(app, label);
        let Block::Row(row) = &app.page().blocks[index] else {
            unreachable!()
        };
        let parts = app.row_parts(app.block_rects(now)[index], row);
        (index, middle(parts.label))
    };
    let (show_frame, at) = name_of(&app, "Show frame");
    // Hovering the name lights the switch it would flip.
    app.handle(Input::Move { x: at.0, y: at.1 }, now);
    assert!(
        app.fade(
            crate::app::Target::Control(show_frame),
            crate::app::Channel::Hover,
            now + SETTLED
        ) > 0.99
    );
    let effects = press(&mut app, at, now);
    assert_eq!(changed(&effects), vec![show_frame]);
    assert_eq!(
        app.write(show_frame),
        Some(Write::Key(
            "record_show_frame",
            settings::Value::Bool(false)
        ))
    );
    let (_, at) = name_of(&app, "Frame rate");
    assert!(
        changed(&press(&mut app, at, now)).is_empty(),
        "a dropdown's name does nothing"
    );
    assert_eq!(app.focus(), None);
}

#[test]
fn a_set_key_offers_a_reset_that_leaves_the_control_where_it_is() {
    let now = Instant::now();
    let mut set = prefs();
    set.set_keys = vec!["record_fps".into()];
    let mut app = App::new(Fonts::fallback(), pages::pages(&set, None, now), SIZE, now);
    app.show_named("recording", now);
    let plain = app_on("recording", SIZE, now);
    let fps = block(&app, "Frame rate");
    let rows: Vec<(usize, bool)> = app
        .page()
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            Block::Row(row) => Some((i, row.reset)),
            _ => None,
        })
        .collect();
    assert!(
        rows.iter().all(|&(i, reset)| reset == (i == fps)),
        "only the set key"
    );
    // The control keeps its place; the button sits left of it.
    let parts = |app: &App| {
        let Block::Row(row) = &app.page().blocks[fps] else {
            unreachable!()
        };
        app.row_parts(app.block_rects(now)[fps], row)
    };
    let (with, without) = (parts(&app), parts(&plain));
    assert_eq!(with.control, without.control);
    let reset = with.reset.expect("a reset button");
    assert_eq!(reset.right() + crate::ui::theme::GAP_S, with.control.x);
    // Tab reaches it before the control; Space and a click ask for the reset.
    let order = app.tab_order();
    let at = order
        .iter()
        .position(|t| *t == crate::app::Target::Reset(fps))
        .unwrap();
    assert_eq!(order[at + 1], crate::app::Target::Control(fps));
    let effects = press(&mut app, middle(reset), now);
    assert!(effects.contains(&Effect::Reset(fps)));
    assert_eq!(app.reset_write(fps), Some(Write::Reset("record_fps")));
    // Back to the default: the button fades out.
    app.update(pages::pages(&prefs(), None, now), now, |_, _| false);
    let shown = app.fade(
        crate::app::Target::Reset(fps),
        crate::app::Channel::Shown,
        now + Duration::from_millis(60),
    );
    assert!(shown > 0.0 && shown < 1.0, "fading, {shown}");
    assert_eq!(
        app.fade(
            crate::app::Target::Reset(fps),
            crate::app::Channel::Shown,
            now + SETTLED
        ),
        0.0
    );
}

#[test]
fn replay_length_steps_and_what_the_memory_buys() {
    let now = Instant::now();
    let page_with = |change: &dyn Fn(&mut crate::app::pages::Prefs)| {
        let mut p = prefs();
        change(&mut p);
        let mut app = App::new(Fonts::fallback(), pages::pages(&p, None, now), SIZE, now);
        app.show_named("replay", now);
        app
    };
    let row = |app: &App, label: &str| -> (Vec<String>, Option<String>) {
        let Block::Row(row) = &app.page().blocks[block(app, label)] else {
            unreachable!()
        };
        let labels = row.control.labels().iter().map(|s| s.to_string()).collect();
        (labels, row.hint.clone())
    };
    let app = page_with(&|_| {});
    assert_eq!(
        row(&app, "Length").0,
        [
            "15\u{a0}s",
            "30\u{a0}s",
            "1\u{a0}min",
            "2\u{a0}min",
            "5\u{a0}min",
            "10\u{a0}min"
        ]
    );
    // 536870912 / 3 · 8 / 61 / 1000 · 0.9 − 128 = 20 994 kbit/s, the rate the
    // recorder starts with (`gsr … -q 20994`).
    assert_eq!(
        row(&app, "Memory").1.as_deref(),
        Some("Video at about 21\u{a0}Mbit/s")
    );
    // A length from the file that is no step stays a choice of its own.
    let odd = page_with(&|p| p.replay_seconds = 45);
    assert_eq!(
        row(&odd, "Length").0.last().map(String::as_str),
        Some("45\u{a0}s")
    );
    // Even the lowest bitrate does not fit ten minutes in 256 MiB.
    let tight = page_with(&|p| {
        p.replay_seconds = 600;
        p.replay_memory_mib = 256;
    });
    assert_eq!(
        row(&tight, "Memory").1.as_deref(),
        Some("Only about 5\u{a0}min fit, give it more memory")
    );
    let fixed = page_with(&|p| p.replay_bitrate = Some(4000));
    assert_eq!(
        row(&fixed, "Memory").1.as_deref(),
        Some("Video at about 4.0\u{a0}Mbit/s")
    );
    // Choosing a step writes its seconds.
    let mut app = page_with(&|_| {});
    let length = block(&app, "Length");
    focus(&mut app, "Length", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(
        app.write(length),
        Some(Write::Key(
            "replay.duration_seconds",
            settings::Value::Int(120)
        ))
    );
}

#[test]
fn display_and_codec_offer_what_this_machine_has() {
    let now = Instant::now();
    let rows = |p: &crate::app::pages::Prefs| {
        let mut app = App::new(Fonts::fallback(), pages::pages(p, None, now), SIZE, now);
        app.show_named("recording", now);
        app
    };
    let labels = |app: &App, label: &str| -> Vec<String> {
        let Block::Row(row) = &app.page().blocks[block(app, label)] else {
            unreachable!()
        };
        row.control.labels().iter().map(|s| s.to_string()).collect()
    };
    let mut p = prefs();
    p.display = crate::app::pages::Display::Output("HDMI-A-1".into());
    let mut app = rows(&p);
    assert_eq!(
        labels(&app, "Display"),
        [
            "Focused display",
            "BenQ ZOWIE XL LCD (DP-3)",
            "AOC 27G4HRE (DP-1)",
            "HDMI-A-1 (disconnected)",
            "Both displays"
        ]
    );
    // HDR and 10-bit variants gpu-screen-recorder lists stay out; the CPU
    // encoder is `libx264` in the file.
    assert_eq!(
        labels(&app, "Codec"),
        [
            "Auto",
            "H.264",
            "H.264 on the CPU",
            "HEVC",
            "AV1",
            "H.264 via Vulkan"
        ]
    );
    let display = block(&app, "Display");
    // From HDMI-A-1 one up is the AOC on DP-1.
    focus(&mut app, "Display", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Up, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(
        app.write(display),
        Some(Write::Key(
            "record_default_target",
            settings::Value::Text("output:DP-1".into())
        ))
    );
    let codec = block(&app, "Codec");
    focus(&mut app, "Codec", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(
        app.write(codec),
        Some(Write::Key(
            "record_codec",
            settings::Value::Text("libx264".into())
        ))
    );
    // Before gpu-screen-recorder has answered: Auto and the file's value.
    let mut p = prefs();
    p.codecs = None;
    p.codec = "hevc".into();
    assert_eq!(labels(&rows(&p), "Codec"), ["Auto", "hevc"]);
}

#[test]
fn the_font_row_offers_the_desktop_font_and_the_installed_families() {
    let now = Instant::now();
    let mut app = app_on("general", SIZE, now);
    let font = block(&app, "Font");
    let Block::Row(row) = &app.page().blocks[font] else {
        unreachable!()
    };
    assert_eq!(
        row.control.labels(),
        [
            "Desktop font",
            "DejaVu Sans",
            "Geist",
            "Noto Sans",
            "Noto Sans Mono",
            "Outfit"
        ]
    );
    focus(&mut app, "Font", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::End, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(
        app.write(font),
        Some(Write::Key(
            "ui_font",
            settings::Value::Text("Outfit".into())
        ))
    );
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Home, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(
        app.write(font),
        Some(Write::Reset("ui_font")),
        "the desktop's font: the key goes"
    );
}

#[test]
fn a_folder_field_asks_for_the_dialog_and_saves_what_it_picks() {
    let now = Instant::now();
    let mut app = app_on("screenshots", SIZE, now);
    let folder = block(&app, "Save folder");
    let field = control(&app, "Save folder", now);
    // The button sits at the field's right end; the text before it.
    let button = (field.right() - 15.0, field.y + 15.0);
    assert!(press(&mut app, button, now).contains(&Effect::PickFolder(folder)));
    let text = (field.x + 20.0, field.y + 15.0);
    assert!(!press(&mut app, text, now).contains(&Effect::PickFolder(folder)));
    // From the keyboard too.
    let ctrl_o = Input::Key {
        key: Key::Char('o'),
        mods: crate::ui::Mods {
            ctrl: true,
            shift: false,
        },
    };
    assert!(
        app.handle(ctrl_o, now)
            .contains(&Effect::PickFolder(folder))
    );
    // What the dialog picks shows and goes out like a confirmed edit.
    let effects = app.pick(folder, "~/Pictures/elsewhere", now);
    assert_eq!(changed(&effects), vec![folder]);
    assert_eq!(
        app.write(folder),
        Some(Write::Key(
            "save_dir",
            settings::Value::Text("~/Pictures/elsewhere".into())
        ))
    );
    // Without a dialog on the desktop there is no button.
    let mut p = prefs();
    p.can_browse = false;
    let mut app = App::new(Fonts::fallback(), pages::pages(&p, None, now), SIZE, now);
    app.show_named("screenshots", now);
    assert!(!press(&mut app, button, now).contains(&Effect::PickFolder(folder)));
}

#[test]
fn like_recordings_and_auto_remove_their_replay_key_and_a_monitor_sets_it() {
    let now = Instant::now();
    let mut set = prefs();
    set.replay_output = Some("DP-1".into());
    set.replay_bitrate = Some(40_000);
    let mut app = App::new(Fonts::fallback(), pages::pages(&set, None, now), SIZE, now);
    assert!(app.show_named("Replay", now));
    for (label, removed) in [
        ("Display", "replay.output"),
        ("Bitrate", "replay.bitrate_kbps"),
    ] {
        let row = super::block(&app, label);
        let control = app.control(row).unwrap();
        // The first entry: "Like recordings", "Auto".
        control.adopt(&crate::ui::widgets::Value::Choice(0), now);
        assert_eq!(app.write(row), Some(Write::Reset(removed)), "{label}");
    }
    let row = super::block(&app, "Display");
    app.control(row)
        .unwrap()
        .adopt(&crate::ui::widgets::Value::Choice(1), now);
    assert_eq!(
        app.write(row),
        Some(Write::Key(
            "replay.output",
            settings::Value::Text("DP-3".into())
        ))
    );
}
