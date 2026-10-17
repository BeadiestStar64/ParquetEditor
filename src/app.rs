use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use crate::document::{COLUMN_TYPES, Document};

pub const ROW_SIZES: [usize; 5] = [10, 20, 30, 50, 100];
pub const COLUMN_SIZES: [usize; 5] = [2, 3, 4, 6, 8];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Cell,
    Row,
    Column,
}

impl Selection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cell => "CELL",
            Self::Row => "ROW",
            Self::Column => "COLUMN",
        }
    }
}

/// A UTF-8 text buffer with a byte cursor that always lies on a character boundary.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub text: String,
    pub cursor: usize,
}

impl Input {
    pub fn new(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }
    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }
    pub fn handle(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('u' | 'U') => {
                    self.text.clear();
                    self.cursor = 0;
                    return;
                }
                KeyCode::Home => {
                    self.cursor = 0;
                    return;
                }
                KeyCode::End => {
                    self.cursor = self.text.len();
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Left => {
                self.cursor = self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i)
            }
            KeyCode::Right => {
                self.cursor += self.text[self.cursor..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
            }
            KeyCode::Home => {
                self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
            }
            KeyCode::End => {
                self.cursor += self.text[self.cursor..]
                    .find('\n')
                    .unwrap_or(self.text.len() - self.cursor)
            }
            KeyCode::Up => self.vertical(false),
            KeyCode::Down => self.vertical(true),
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .unwrap()
                    .0;
                self.text.drain(previous..self.cursor);
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                self.text.remove(self.cursor);
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            _ => {}
        }
    }

    fn vertical(&mut self, down: bool) {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        let position = self.text[start..self.cursor].chars().count();
        let next = if down {
            self.text[self.cursor..]
                .find('\n')
                .map(|i| self.cursor + i + 1)
        } else if start > 0 {
            Some(self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1))
        } else {
            None
        };
        if let Some(next) = next {
            let line = self.text[next..].split('\n').next().unwrap_or("");
            self.cursor = next
                + line
                    .char_indices()
                    .nth(position)
                    .map_or(line.len(), |(i, _)| i);
        }
    }
}

#[derive(Clone, Debug)]
pub enum Modal {
    Edit(Input),
    Column { name: Input, dtype: usize },
    SavePath(Input),
    Overwrite(PathBuf),
    Quit,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Copy,
    Paste,
}

#[derive(Clone, Copy, Debug)]
pub enum Target {
    Cell(usize, usize),
    Row(usize),
    Column(usize),
    RowSize(bool),
    ColumnSize(bool),
    RowPage(bool),
    ColumnPage(bool),
}

pub struct App {
    pub document: Document,
    pub row: usize,
    pub column: usize,
    pub selection: Selection,
    pub row_size: usize,
    pub column_size: usize,
    pub visible_rows: usize,
    pub visible_columns: usize,
    pub modal: Option<Modal>,
    pub status: String,
    pub error: bool,
    pub should_quit: bool,
    pub hits: Vec<(Rect, Target)>,
    quit_after_save: bool,
}

impl App {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            row: 0,
            column: 0,
            selection: Selection::Cell,
            row_size: 1,
            column_size: 2,
            visible_rows: 20,
            visible_columns: 4,
            modal: None,
            status: "C: add column  R: add row  Enter: edit  F1: help".into(),
            error: false,
            should_quit: false,
            hits: Vec::new(),
            quit_after_save: false,
        }
    }

    pub fn message(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.error = false;
    }
    pub fn fail(&mut self, error: impl std::fmt::Display) {
        self.status = error.to_string();
        self.error = true;
    }

    pub fn clamp_selection(&mut self) {
        self.row = self.row.min(self.document.rows().saturating_sub(1));
        self.column = self.column.min(self.document.columns().saturating_sub(1));
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<Effect> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control {
            match key.code {
                KeyCode::Char('q' | 'Q') => {
                    if self.accept_pending() {
                        self.quit_after_save = false;
                        if self.document.is_dirty() {
                            self.modal = Some(Modal::Quit);
                        } else {
                            self.should_quit = true;
                        }
                    }
                    return None;
                }
                KeyCode::Char('s' | 'S') => {
                    if matches!(self.modal, Some(Modal::SavePath(_))) {
                        let modal = self.modal.take().unwrap();
                        self.modal_key(modal, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        return None;
                    }
                    if self.accept_pending() {
                        self.save_current();
                    }
                    return None;
                }
                KeyCode::Char('c' | 'C') if self.modal.is_none() => return Some(Effect::Copy),
                KeyCode::Char('v' | 'V') => return Some(Effect::Paste),
                KeyCode::Char('z' | 'Z') if self.modal.is_none() => {
                    if self.document.undo() {
                        self.clamp_selection();
                        self.message("Undone");
                    } else {
                        self.message("Nothing to undo");
                    }
                    return None;
                }
                KeyCode::Char(' ') | KeyCode::Null if self.modal.is_none() => {
                    self.selection = Selection::Column;
                    return None;
                }
                _ => {}
            }
        }
        if let Some(modal) = self.modal.take() {
            self.modal_key(modal, key);
            return None;
        }
        match key.code {
            KeyCode::Left => self.column = self.column.saturating_sub(1),
            KeyCode::Right => {
                self.column = (self.column + 1).min(self.document.columns().saturating_sub(1))
            }
            KeyCode::Up => self.row = self.row.saturating_sub(1),
            KeyCode::Down => self.row = (self.row + 1).min(self.document.rows().saturating_sub(1)),
            KeyCode::PageUp => self.page_rows(false),
            KeyCode::PageDown => self.page_rows(true),
            KeyCode::Home => {
                self.column = 0;
                if control {
                    self.row = 0;
                }
            }
            KeyCode::End => {
                self.column = self.document.columns().saturating_sub(1);
                if control {
                    self.row = self.document.rows().saturating_sub(1);
                }
            }
            KeyCode::Enter | KeyCode::F(2) => self.start_edit(),
            KeyCode::Char('r' | 'R') => match self.document.add_row() {
                Ok(()) => {
                    self.row = self.document.rows() - 1;
                    self.message("Added a NULL row");
                }
                Err(error) => self.fail(error),
            },
            KeyCode::Char('c' | 'C') => {
                self.modal = Some(Modal::Column {
                    name: Input::default(),
                    dtype: 0,
                })
            }
            KeyCode::Delete => match self.document.set_cell(self.row, self.column, None) {
                Ok(_) => self.message("Cell set to NULL"),
                Err(error) => self.fail(error),
            },
            KeyCode::F(6) => {
                self.selection = match self.selection {
                    Selection::Cell => Selection::Row,
                    Selection::Row => Selection::Column,
                    Selection::Column => Selection::Cell,
                }
            }
            KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.selection = Selection::Row
            }
            KeyCode::Esc => self.selection = Selection::Cell,
            KeyCode::Char('[') => self.resize_rows(false),
            KeyCode::Char(']') => self.resize_rows(true),
            KeyCode::Char('-') => self.resize_columns(false),
            KeyCode::Char('+' | '=') => self.resize_columns(true),
            KeyCode::Tab => self.page_columns(true),
            KeyCode::BackTab => self.page_columns(false),
            KeyCode::F(1) | KeyCode::Char('?') => self.modal = Some(Modal::Help),
            _ => {}
        }
        None
    }

    fn start_edit(&mut self) {
        match self.document.cell(self.row, self.column) {
            Ok(value) => self.modal = Some(Modal::Edit(Input::new(value.unwrap_or_default()))),
            Err(error) => self.fail(error),
        }
    }

    // Commit text before global save/quit so pending input cannot silently disappear.
    fn accept_pending(&mut self) -> bool {
        match self.modal.take() {
            Some(Modal::Edit(input)) => {
                match self
                    .document
                    .set_cell(self.row, self.column, Some(&input.text))
                {
                    Ok(_) => true,
                    Err(error) => {
                        self.modal = Some(Modal::Edit(input));
                        self.fail(format!("{error:#}"));
                        false
                    }
                }
            }
            Some(Modal::Column { name, dtype }) => match self
                .document
                .add_column(&name.text, COLUMN_TYPES[dtype].1.clone())
            {
                Ok(()) => {
                    self.column = self.document.columns() - 1;
                    true
                }
                Err(error) => {
                    self.modal = Some(Modal::Column { name, dtype });
                    self.fail(error);
                    false
                }
            },
            Some(modal @ (Modal::SavePath(_) | Modal::Overwrite(_) | Modal::Quit)) => {
                self.modal = Some(modal);
                false
            }
            _ => true,
        }
    }

    fn modal_key(&mut self, modal: Modal, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.quit_after_save = false;
            self.message("Cancelled");
            return;
        }
        match modal {
            Modal::Edit(mut input) => {
                if key.code == KeyCode::Enter
                    && key
                        .modifiers
                        .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
                {
                    input.insert("\n");
                } else if key.code == KeyCode::Enter {
                    match self
                        .document
                        .set_cell(self.row, self.column, Some(&input.text))
                    {
                        Ok(_) => {
                            self.message("Cell updated");
                            return;
                        }
                        Err(error) => self.fail(format!("{error:#}")),
                    }
                } else {
                    input.handle(key);
                }
                self.modal = Some(Modal::Edit(input));
            }
            Modal::Column {
                mut name,
                mut dtype,
            } => {
                match key.code {
                    KeyCode::Tab | KeyCode::Down => dtype = (dtype + 1) % COLUMN_TYPES.len(),
                    KeyCode::BackTab | KeyCode::Up => {
                        dtype = (dtype + COLUMN_TYPES.len() - 1) % COLUMN_TYPES.len()
                    }
                    KeyCode::Enter => match self
                        .document
                        .add_column(&name.text, COLUMN_TYPES[dtype].1.clone())
                    {
                        Ok(()) => {
                            self.column = self.document.columns() - 1;
                            self.message("Column added; R adds a row");
                            return;
                        }
                        Err(error) => self.fail(error),
                    },
                    _ => name.handle(key),
                }
                self.modal = Some(Modal::Column { name, dtype });
            }
            Modal::SavePath(mut input) => {
                if key.code == KeyCode::Enter {
                    if input.text.trim().is_empty() {
                        self.fail("Enter a file path");
                    } else {
                        let path = PathBuf::from(&input.text);
                        if path.exists() {
                            self.modal = Some(Modal::Overwrite(path));
                        } else if !self.write(&path, false) {
                            self.modal = Some(Modal::SavePath(input));
                        }
                        return;
                    }
                } else {
                    input.handle(key);
                }
                self.modal = Some(Modal::SavePath(input));
            }
            Modal::Overwrite(path) => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    if !self.write(&path, true) {
                        self.modal = Some(Modal::Overwrite(path));
                    }
                }
                KeyCode::Char('n' | 'N') => {
                    self.quit_after_save = false;
                    self.message("Save cancelled");
                }
                _ => self.modal = Some(Modal::Overwrite(path)),
            },
            Modal::Quit => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.quit_after_save = true;
                    self.save_current();
                }
                KeyCode::Char('n' | 'N') => self.should_quit = true,
                _ => self.modal = Some(Modal::Quit),
            },
            Modal::Help => {
                if key.code != KeyCode::F(1) && key.code != KeyCode::Enter {
                    self.modal = Some(Modal::Help);
                }
            }
        }
    }

    fn save_current(&mut self) {
        if let Some(path) = self.document.path.clone() {
            if !self.document.has_saved_file() && path.exists() {
                self.modal = Some(Modal::Overwrite(path));
            } else if !self.write(&path, self.document.has_saved_file()) && self.quit_after_save {
                self.modal = Some(Modal::Quit);
            }
        } else {
            self.modal = Some(Modal::SavePath(Input::new("dataset.parquet".into())));
        }
    }

    fn write(&mut self, path: &std::path::Path, overwrite: bool) -> bool {
        match self.document.save(path, overwrite) {
            Ok(()) => {
                self.message(format!("Saved {}", path.display()));
                if self.quit_after_save {
                    self.should_quit = true;
                }
                self.quit_after_save = false;
                true
            }
            Err(error) => {
                self.fail(format!("{error:#}"));
                false
            }
        }
    }

    pub fn copy_cell(&self) -> Result<Option<String>> {
        self.document.cell(self.row, self.column)
    }

    pub fn paste(&mut self, value: Option<&str>) {
        if let Some(modal) = &mut self.modal {
            match modal {
                Modal::Edit(input) | Modal::SavePath(input) | Modal::Column { name: input, .. } => {
                    if let Some(value) = value {
                        input.insert(value);
                    }
                }
                _ => {}
            }
        } else {
            match self.document.set_cell(self.row, self.column, value) {
                Ok(_) => self.message("Cell pasted"),
                Err(error) => self.fail(format!("{error:#}")),
            }
        }
    }

    pub fn resize_rows(&mut self, next: bool) {
        self.row_size = step(self.row_size, ROW_SIZES.len(), next);
    }
    pub fn resize_columns(&mut self, next: bool) {
        self.column_size = step(self.column_size, COLUMN_SIZES.len(), next);
    }
    pub fn page_rows(&mut self, next: bool) {
        self.row = if next {
            self.row.saturating_add(self.visible_rows)
        } else {
            self.row.saturating_sub(self.visible_rows)
        };
        self.clamp_selection();
    }
    pub fn page_columns(&mut self, next: bool) {
        self.column = if next {
            self.column.saturating_add(self.visible_columns)
        } else {
            self.column.saturating_sub(self.visible_columns)
        };
        self.clamp_selection();
    }

    pub fn mouse(&mut self, event: MouseEvent) {
        if self.modal.is_some() {
            return;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let target = self
                    .hits
                    .iter()
                    .find(|(r, _)| r.contains((event.column, event.row).into()))
                    .map(|(_, t)| *t);
                if let Some(target) = target {
                    match target {
                        Target::Cell(row, column) => {
                            self.row = row;
                            self.column = column;
                            self.selection = Selection::Cell;
                        }
                        Target::Row(row) => {
                            self.row = row;
                            self.selection = Selection::Row;
                        }
                        Target::Column(column) => {
                            self.column = column;
                            self.selection = Selection::Column;
                        }
                        Target::RowSize(next) => self.resize_rows(next),
                        Target::ColumnSize(next) => self.resize_columns(next),
                        Target::RowPage(next) => self.page_rows(next),
                        Target::ColumnPage(next) => self.page_columns(next),
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                self.row = self.row.saturating_add(3);
                self.clamp_selection();
            }
            MouseEventKind::ScrollUp => self.row = self.row.saturating_sub(3),
            _ => {}
        }
    }
}

fn step(index: usize, length: usize, next: bool) -> usize {
    if next {
        (index + 1).min(length - 1)
    } else {
        index.saturating_sub(1)
    }
}
