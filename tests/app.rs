use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use parquet_editor::{
    app::{App, Effect, Input, Modal, Selection, Target},
    document::Document,
    ui,
};
use polars::prelude::DataType;
use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Color};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn app(rows: usize, columns: usize) -> App {
    let mut doc = Document::new(None);
    for i in 0..columns {
        doc.add_column(&format!("column_{i}"), DataType::String)
            .unwrap();
    }
    for _ in 0..rows {
        doc.add_row().unwrap();
    }
    App::new(doc)
}

#[test]
fn keyboard_creates_columns_rows_and_edits_then_undoes() {
    let mut app = app(0, 0);
    app.key(key(KeyCode::Char('c')));
    app.paste(Some("日本語"));
    app.key(key(KeyCode::Enter));
    app.key(key(KeyCode::Char('r')));
    app.key(key(KeyCode::Enter));
    app.paste(Some("長い本文\n二行目🌸"));
    app.key(key(KeyCode::Enter));
    assert_eq!(
        app.document.cell(0, 0).unwrap(),
        Some("長い本文\n二行目🌸".into())
    );
    app.key(ctrl('z'));
    assert_eq!(app.document.cell(0, 0).unwrap(), None);
    app.key(ctrl('z'));
    assert_eq!(app.document.rows(), 0);
    app.key(ctrl('z'));
    assert_eq!(app.document.columns(), 0);
    assert!(!app.document.is_dirty());
}

#[test]
fn clipboard_effects_paste_entire_cell_and_preserve_null_and_empty() {
    let mut app = app(2, 1);
    app.paste(Some("line1\nline2\t日本語"));
    assert_eq!(app.key(ctrl('c')), Some(Effect::Copy));
    let copied = app.copy_cell().unwrap();
    app.key(key(KeyCode::Down));
    assert_eq!(app.key(ctrl('v')), Some(Effect::Paste));
    app.paste(copied.as_deref());
    assert_eq!(app.document.cell(1, 0).unwrap(), copied);
    app.paste(None);
    assert_eq!(app.document.cell(1, 0).unwrap(), None);
    app.paste(Some(""));
    assert_eq!(app.document.cell(1, 0).unwrap(), Some("".into()));
    app.key(key(KeyCode::Delete));
    assert_eq!(app.document.cell(1, 0).unwrap(), None);
}

#[test]
fn movement_pages_and_windows_key_release_are_handled() {
    let mut app = app(45, 10);
    app.visible_rows = 10;
    app.visible_columns = 3;
    app.key(key(KeyCode::Up));
    app.key(key(KeyCode::Left));
    assert_eq!((app.row, app.column), (0, 0));
    app.key(key(KeyCode::PageDown));
    app.key(key(KeyCode::Tab));
    assert_eq!((app.row, app.column), (10, 3));
    app.key(KeyEvent::new_with_kind(
        KeyCode::Down,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));
    assert_eq!(app.row, 10);
    app.key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL));
    assert_eq!((app.row, app.column), (44, 9));
    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Right));
    assert_eq!((app.row, app.column), (44, 9));
    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
    assert_eq!((app.row, app.column), (0, 0));
}

#[test]
fn quit_yes_saves_to_a_new_path_before_exiting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new.parquet");
    let mut app = app(1, 1);
    app.paste(Some("saved before quit"));
    app.key(ctrl('q'));
    assert!(matches!(app.modal, Some(Modal::Quit)));
    app.key(key(KeyCode::Char('y')));
    assert!(matches!(app.modal, Some(Modal::SavePath(_))));
    app.key(ctrl('u'));
    app.paste(path.to_str());
    app.key(key(KeyCode::Enter));
    assert!(app.should_quit);
    assert_eq!(
        Document::open(&path).unwrap().cell(0, 0).unwrap(),
        Some("saved before quit".into())
    );
}

#[test]
fn quit_no_discards_without_writing_and_escape_cancels() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new.parquet");
    let mut app = app(1, 1);
    app.document.path = Some(path.clone());
    app.key(ctrl('q'));
    app.key(key(KeyCode::Esc));
    assert!(!app.should_quit);
    assert!(app.modal.is_none());
    app.key(ctrl('q'));
    app.key(key(KeyCode::Char('n')));
    assert!(app.should_quit);
    assert!(!path.exists());
}

#[test]
fn ctrl_s_overwrites_a_loaded_file_and_clean_quit_needs_no_prompt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.parquet");
    let mut original = app(1, 1);
    original.document.save(&path, false).unwrap();
    let mut app = App::new(Document::open(&path).unwrap());
    app.paste(Some("changed"));
    app.key(ctrl('s'));
    assert!(app.modal.is_none());
    assert!(!app.document.is_dirty());
    assert_eq!(
        Document::open(&path).unwrap().cell(0, 0).unwrap(),
        Some("changed".into())
    );
    app.key(ctrl('q'));
    assert!(app.should_quit);
}

#[test]
fn saving_and_quitting_do_not_drop_text_still_in_the_editor() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pending.parquet");
    let mut app = app(1, 1);
    app.document.path = Some(path.clone());
    app.key(key(KeyCode::Enter));
    app.paste(Some("未確定"));
    app.key(ctrl('s'));
    assert_eq!(
        Document::open(&path).unwrap().cell(0, 0).unwrap(),
        Some("未確定".into())
    );
    app.key(key(KeyCode::Enter));
    app.paste(Some("追加"));
    app.key(ctrl('q'));
    assert!(matches!(app.modal, Some(Modal::Quit)));
    assert!(!app.should_quit);
    app.key(key(KeyCode::Char('y')));
    assert!(app.should_quit);
    assert_eq!(
        Document::open(&path).unwrap().cell(0, 0).unwrap(),
        Some("未確定追加".into())
    );
}

#[test]
fn save_failure_and_invalid_pending_input_never_exit() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = app(1, 1);
    app.document.path = Some(directory.path().join("missing/save.parquet"));
    app.key(ctrl('q'));
    app.key(key(KeyCode::Char('y')));
    assert!(!app.should_quit);
    assert!(app.error);
    assert!(matches!(app.modal, Some(Modal::Quit)));
    app.key(key(KeyCode::Esc));
    app.document.add_column("int", DataType::Int64).unwrap();
    app.column = 1;
    app.key(key(KeyCode::Enter));
    app.paste(Some("invalid"));
    app.key(ctrl('q'));
    assert!(app.error);
    assert!(matches!(app.modal, Some(Modal::Edit(_))));
    assert!(!app.should_quit);
}

#[test]
fn a_new_path_that_already_exists_requires_overwrite_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("exists.parquet");
    std::fs::write(&path, b"existing file").unwrap();
    let mut app = app(1, 1);
    app.document.path = Some(path.clone());
    app.key(ctrl('s'));
    assert!(matches!(app.modal, Some(Modal::Overwrite(_))));
    app.key(key(KeyCode::Char('n')));
    assert_eq!(std::fs::read(&path).unwrap(), b"existing file");
    app.key(ctrl('s'));
    app.key(key(KeyCode::Char('y')));
    assert!(Document::open(&path).is_ok());
}

#[test]
fn unicode_editor_keeps_cursor_on_boundaries_and_navigates_lines() {
    let mut input = Input::new("日本語\n猫🌸\nthird".into());
    input.handle(key(KeyCode::Home));
    input.handle(key(KeyCode::Up));
    assert_eq!(&input.text[input.cursor..], "猫🌸\nthird");
    input.handle(key(KeyCode::Right));
    input.handle(key(KeyCode::Delete));
    assert_eq!(input.text, "日本語\n猫\nthird");
    input.handle(key(KeyCode::Up));
    input.handle(key(KeyCode::Backspace));
    assert_eq!(input.text, "本語\n猫\nthird");
    input.handle(ctrl('u'));
    assert_eq!(input.text, "");
    assert_eq!(input.cursor, 0);
}

#[test]
fn ui_highlights_an_entire_selected_row_or_column_and_mouse_selects_them() {
    let mut app = app(4, 3);
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    app.key(key(KeyCode::F(6)));
    assert_eq!(app.selection, Selection::Row);
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let (rect, _) = app
        .hits
        .iter()
        .find(|(_, t)| matches!(t, Target::Cell(0, 1)))
        .unwrap();
    let background = terminal.backend().buffer()[(rect.x, rect.y)].bg;
    assert_eq!(background, Color::Rgb(40, 63, 88));
    let (rect, _) = app
        .hits
        .iter()
        .find(|(_, t)| matches!(t, Target::Column(1)))
        .unwrap();
    let position = (rect.x, rect.y);
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: position.0,
        row: position.1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.selection, Selection::Column);
    assert_eq!(app.column, 1);
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let (rect, _) = app
        .hits
        .iter()
        .find(|(_, t)| matches!(t, Target::Cell(2, 1)))
        .unwrap();
    assert_eq!(terminal.backend().buffer()[(rect.x, rect.y)].bg, background);
    let (rect, _) = app
        .hits
        .iter()
        .find(|(_, t)| matches!(t, Target::Row(2)))
        .unwrap();
    let position = (rect.x, rect.y);
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: position.0,
        row: position.1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!((app.selection, app.row), (Selection::Row, 2));
}

#[test]
fn page_controls_change_sizes_and_small_terminals_do_not_panic() {
    let mut app = app(35, 9);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let (rect, _) = app
        .hits
        .iter()
        .find(|(_, t)| matches!(t, Target::RowSize(true)))
        .unwrap();
    let point = (rect.x, rect.y);
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.row_size, 2);
    app.key(key(KeyCode::Char('-')));
    assert_eq!(app.column_size, 1);
    app.row = 34;
    app.column = 8;
    for (width, height) in [(120, 40), (80, 24), (55, 18), (30, 10), (1, 1)] {
        terminal.backend_mut().resize(width, height);
        terminal.resize(Rect::new(0, 0, width, height)).unwrap();
        for modal in [
            None,
            Some(Modal::Edit(Input::new("日本語\n🌸".repeat(40)))),
            Some(Modal::Quit),
            Some(Modal::Help),
        ] {
            app.modal = modal;
            terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        }
    }
}
