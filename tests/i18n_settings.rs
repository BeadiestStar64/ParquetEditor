use std::{
    collections::{BTreeMap, BTreeSet},
    process::Command,
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use parquet_editor::{
    app::{App, Input, Modal, Selection},
    document::Document,
    i18n::{Language, Message, format, text},
    settings::Settings,
    ui,
};
use polars::prelude::DataType;
use ratatui::{Terminal, backend::TestBackend};
use unicode_width::UnicodeWidthStr;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}
fn dataset() -> Document {
    let mut document = Document::new(None);
    document.add_column("text", DataType::String).unwrap();
    document.add_column("number", DataType::Int64).unwrap();
    document.add_row().unwrap();
    document.add_row().unwrap();
    document.set_cell(0, 0, Some("日本語\n本文🌸")).unwrap();
    document.set_cell(0, 1, Some("42")).unwrap();
    document.set_cell(1, 0, Some("next row")).unwrap();
    document
}

#[test]
fn ctrl_delete_requires_confirmation_only_clears_one_cell_and_can_be_undone() {
    for selection in [Selection::Cell, Selection::Row, Selection::Column] {
        let mut app = App::new(dataset());
        app.selection = selection;
        let history = app.document.undo_len();
        app.key(ctrl(KeyCode::Delete));
        assert!(matches!(
            app.modal,
            Some(Modal::DeleteCell { row: 0, column: 0 })
        ));
        assert_eq!(
            app.document.cell(0, 0).unwrap(),
            Some("日本語\n本文🌸".into())
        );
        app.key(key(KeyCode::Down));
        app.key(ctrl(KeyCode::Char('s')));
        assert_eq!(app.document.undo_len(), history);
        app.key(key(KeyCode::Char('Y')));
        assert!(app.modal.is_none());
        assert_eq!((app.document.rows(), app.document.columns()), (2, 2));
        assert_eq!(app.document.cell(0, 0).unwrap(), None);
        assert_eq!(app.document.cell(0, 1).unwrap(), Some("42".into()));
        assert_eq!(app.document.cell(1, 0).unwrap(), Some("next row".into()));
        assert_eq!(app.document.undo_len(), history + 1);
        app.key(ctrl(KeyCode::Char('z')));
        assert_eq!(
            app.document.cell(0, 0).unwrap(),
            Some("日本語\n本文🌸".into())
        );
    }
}

#[test]
fn cancelling_deletion_keeps_saved_data_and_history_intact() {
    let directory = tempfile::tempdir().unwrap();
    let mut document = dataset();
    document
        .save(&directory.path().join("data.parquet"), false)
        .unwrap();
    let mut app = App::new(document);
    let history = app.document.undo_len();
    for cancel in [KeyCode::Char('n'), KeyCode::Char('N'), KeyCode::Esc] {
        app.key(ctrl(KeyCode::Delete));
        app.key(key(cancel));
        assert!(app.modal.is_none());
        assert_eq!(app.document.undo_len(), history);
        assert!(!app.document.is_dirty());
        assert_eq!(
            app.document.cell(0, 0).unwrap(),
            Some("日本語\n本文🌸".into())
        );
    }
    // Already-null cells do not add a redundant undo revision.
    app.row = 1;
    app.column = 1;
    app.key(ctrl(KeyCode::Delete));
    app.key(key(KeyCode::Char('y')));
    assert_eq!(app.document.undo_len(), history);
    assert!(!app.document.is_dirty());
}

#[test]
fn deletion_handles_empty_data_release_events_and_text_editor_delete() {
    let mut app = App::new(Document::new(None));
    app.key(ctrl(KeyCode::Delete));
    assert!(app.modal.is_none());
    assert_eq!(app.status.key, "error.no_cell");
    let mut app = App::new(dataset());
    app.key(KeyEvent::new_with_kind(
        KeyCode::Delete,
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    ));
    assert!(app.modal.is_none());
    app.key(key(KeyCode::Enter));
    app.key(ctrl(KeyCode::Home));
    app.key(key(KeyCode::Delete));
    assert!(matches!(&app.modal, Some(Modal::Edit(input)) if input.text == "本語\n本文🌸"));
    assert_eq!(
        app.document.cell(0, 0).unwrap(),
        Some("日本語\n本文🌸".into())
    );
}

#[test]
fn settings_save_language_and_page_sizes_without_changing_dataset() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config/settings.json");
    let mut app = App::with_settings(dataset(), Settings::default(), Some(path.clone()));
    let history = app.document.undo_len();
    app.key(ctrl(KeyCode::Char(',')));
    app.key(key(KeyCode::Right));
    assert_eq!(app.language(), Language::Japanese); // Still a draft.
    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Right));
    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Left));
    app.key(key(KeyCode::Enter));
    assert_eq!(app.language(), Language::English);
    assert_eq!((app.row_size, app.column_size), (2, 1));
    assert_eq!(app.document.undo_len(), history);
    let loaded = Settings::load(&path).unwrap();
    assert_eq!(loaded.language, Language::English);
    assert_eq!((loaded.rows_per_page, loaded.columns_per_page), (30, 3));
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["language"], "en_US");
    let restarted = App::with_settings(dataset(), loaded, Some(path));
    assert_eq!(restarted.language(), Language::English);
    assert_eq!((restarted.row_size, restarted.column_size), (2, 1));
}

#[test]
fn cancelling_settings_keeps_preferences_and_pending_text() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    Settings::default().save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut app = App::with_settings(dataset(), Settings::default(), Some(path.clone()));
    app.key(key(KeyCode::Enter));
    app.paste(Some("未確定"));
    for close in [key(KeyCode::Esc), ctrl(KeyCode::Char(','))] {
        app.key(ctrl(KeyCode::Char(',')));
        app.key(key(KeyCode::Right));
        app.key(close);
        assert_eq!(app.language(), Language::Japanese);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(matches!(&app.modal, Some(Modal::Edit(input)) if input.text.ends_with("未確定")));
    }
    app.key(ctrl(KeyCode::Char(',')));
    app.key(key(KeyCode::Right));
    app.key(ctrl(KeyCode::Char('s')));
    assert_eq!(app.language(), Language::English);
    assert!(matches!(&app.modal, Some(Modal::Edit(input)) if input.text.ends_with("未確定")));
    assert_eq!(
        app.document.cell(0, 0).unwrap(),
        Some("日本語\n本文🌸".into())
    );
    app.key(key(KeyCode::Enter));
    assert_eq!(
        app.document.cell(0, 0).unwrap(),
        Some("日本語\n本文🌸未確定".into())
    );
}

#[test]
fn settings_write_failure_keeps_old_preferences_and_the_draft() {
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("not-a-directory");
    std::fs::write(&blocker, b"keep").unwrap();
    let mut app = App::with_settings(
        dataset(),
        Settings::default(),
        Some(blocker.join("settings.json")),
    );
    app.key(ctrl(KeyCode::Char(',')));
    app.key(key(KeyCode::Right));
    app.key(key(KeyCode::Enter));
    assert_eq!(app.language(), Language::Japanese);
    assert!(app.error);
    assert_eq!(app.status.key, "error.config_write");
    assert!(
        matches!(&app.modal, Some(Modal::Settings { draft, .. }) if draft.language == Language::English)
    );
    assert_eq!(std::fs::read(&blocker).unwrap(), b"keep");
    app.key(ctrl(KeyCode::Char('q')));
    assert!(!app.should_quit);
    assert!(matches!(app.modal, Some(Modal::Settings { .. })));
}

#[test]
fn settings_load_defaults_validate_inputs_and_replace_existing_json() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    std::fs::write(&path, r#"{"language":"en_US"}"#).unwrap();
    assert_eq!(
        Settings::load(&path).unwrap(),
        Settings {
            language: Language::English,
            ..Settings::default()
        }
    );
    for json in [
        "{",
        r#"{"language":"invalid"}"#,
        r#"{"rows_per_page":0}"#,
        r#"{"columns_per_page":999}"#,
    ] {
        std::fs::write(&path, json).unwrap();
        assert!(Settings::load(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), json);
    }
    Settings::default().save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

fn placeholders(message: &str) -> BTreeSet<&str> {
    message
        .split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

#[test]
fn translation_json_keys_placeholders_and_source_references_are_complete() {
    let japanese: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../translate/ja_JP.json")).unwrap();
    let english: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../translate/en_US.json")).unwrap();
    assert_eq!(
        japanese.keys().collect::<Vec<_>>(),
        english.keys().collect::<Vec<_>>()
    );
    for (key, value) in &english {
        assert!(!value.trim().is_empty(), "Empty translation: {key}");
        assert!(!japanese[key].trim().is_empty(), "Empty translation: {key}");
        assert_eq!(
            placeholders(value),
            placeholders(&japanese[key]),
            "Placeholder mismatch: {key}"
        );
    }
    for source in [
        include_str!("../src/app.rs"),
        include_str!("../src/ui.rs"),
        include_str!("../src/main.rs"),
        include_str!("../src/document.rs"),
        include_str!("../src/settings.rs"),
    ] {
        for key in source.split('"').skip(1).step_by(2) {
            if key.ends_with(".json") {
                continue;
            }
            if [
                "app.",
                "ui.",
                "selection.",
                "edit.",
                "column.",
                "save.",
                "overwrite.",
                "quit.",
                "delete.",
                "settings.",
                "language.",
                "help.",
                "status.",
                "error.",
                "cli.",
            ]
            .iter()
            .any(|prefix| key.starts_with(prefix))
            {
                assert!(english.contains_key(key), "Missing translation key: {key}");
            }
        }
    }
}

#[test]
fn messages_errors_and_interpolation_follow_language_without_reinterpreting_user_data() {
    let formatted = format(
        Language::Japanese,
        "ui.cell_title",
        &[
            ("row", "1".into()),
            ("column", "2".into()),
            ("name", "{type}".into()),
            ("type", "str".into()),
            ("readonly", "".into()),
        ],
    );
    assert!(formatted.contains("{type}"));
    assert!(formatted.contains("str"));
    let mut app = App::new(dataset());
    app.document.add_column("flag", DataType::Boolean).unwrap();
    app.column = 2;
    app.paste(Some("invalid"));
    assert!(
        app.status
            .render(Language::Japanese)
            .contains("入力してください")
    );
    assert!(
        app.status
            .render(Language::English)
            .contains("Expected true / false")
    );
    let error = Settings::load(std::path::Path::new("\0invalid")).unwrap_err();
    let message = Message::from_error(&error);
    assert!(
        message
            .render(Language::Japanese)
            .starts_with(text(Language::Japanese, "error.config_read"))
    );
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut result = String::new();
    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            result.push_str(buffer[(x, y)].symbol());
            x += buffer[(x, y)].symbol().width().max(1) as u16;
        }
        result.push('\n');
    }
    result
}

#[test]
fn ui_switches_language_for_grid_settings_confirmations_help_and_errors() {
    let mut app = App::new(dataset());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(screen(&terminal).contains("データセットエディター"));
    app.key(ctrl(KeyCode::Char(',')));
    app.key(key(KeyCode::Right));
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(screen(&terminal).contains("Language"));
    app.key(key(KeyCode::Enter));
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    assert!(screen(&terminal).contains("Dataset editor"));
    for (language, expected) in [
        (Language::Japanese, "セルの内容を削除"),
        (Language::English, "Delete cell content"),
    ] {
        app.settings.language = language;
        app.key(ctrl(KeyCode::Delete));
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        let output = screen(&terminal);
        assert!(output.contains(expected));
        assert!(output.contains("Y") && output.contains("N"));
        app.key(key(KeyCode::Esc));
        for (width, height) in [(55, 18), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for modal in [
                Some(Modal::Settings {
                    draft: app.settings.clone(),
                    selected: 2,
                    return_to: None,
                }),
                Some(Modal::DeleteCell { row: 0, column: 0 }),
                Some(Modal::Help),
                Some(Modal::Edit(Input::new("本文🌸\n".repeat(50)))),
            ] {
                app.modal = modal;
                terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
            }
            app.modal = None;
        }
    }
}

#[test]
fn cli_help_uses_saved_language_from_an_isolated_settings_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    for (language, expected) in [(Language::Japanese, "使い方"), (Language::English, "Usage")] {
        Settings {
            language,
            ..Settings::default()
        }
        .save(&path)
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_parquet-editor"))
            .arg("--help")
            .env("PARQUET_EDITOR_CONFIG", &path)
            .output()
            .unwrap();
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains(expected));
        assert!(help.contains("Ctrl+Delete") && help.contains("Ctrl+,"));
    }
}
