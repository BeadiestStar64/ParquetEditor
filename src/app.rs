use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use crate::{
    document::{COLUMN_TYPES, Document},
    i18n::{Language, Message, text},
    settings::Settings,
};

pub use crate::settings::{COLUMN_SIZES, ROW_SIZES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Cell,
    Row,
    Column,
}

impl Selection {
    pub fn label(self, language: Language) -> &'static str {
        text(
            language,
            match self {
                Self::Cell => "selection.cell",
                Self::Row => "selection.row",
                Self::Column => "selection.column",
            },
        )
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
    Column {
        name: Input,
        dtype: usize,
    },
    SavePath(Input),
    Overwrite(PathBuf),
    Quit,
    Help,
    DeleteCell {
        row: usize,
        column: usize,
    },
    Settings {
        draft: Settings,
        selected: usize,
        return_to: Option<Box<Modal>>,
    },
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
    pub status: Message,
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
    pub error: bool,
    pub should_quit: bool,
    pub hits: Vec<(Rect, Target)>,
    quit_after_save: bool,
}

impl App {
    pub fn new(document: Document) -> Self {
        Self::with_settings(document, Settings::default(), None)
    }

    pub fn with_settings(
        document: Document,
        settings: Settings,
        settings_path: Option<PathBuf>,
    ) -> Self {
        let row_size = ROW_SIZES
            .iter()
            .position(|&n| n == settings.rows_per_page)
            .unwrap_or(1);
        let column_size = COLUMN_SIZES
            .iter()
            .position(|&n| n == settings.columns_per_page)
            .unwrap_or(2);
        Self {
            document,
            row: 0,
            column: 0,
            selection: Selection::Cell,
            row_size,
            column_size,
            visible_rows: ROW_SIZES[row_size],
            visible_columns: COLUMN_SIZES[column_size],
            modal: None,
            status: Message::new("status.initial"),
            settings,
            settings_path,
            error: false,
            should_quit: false,
            hits: Vec::new(),
            quit_after_save: false,
        }
    }

    pub fn language(&self) -> Language {
        self.settings.language
    }

    pub fn message(&mut self, key: &'static str) {
        self.status = Message::new(key);
        self.error = false;
    }
    pub fn fail(&mut self, error: anyhow::Error) {
        self.status = Message::from_error(&error);
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
        if (control && key.code == KeyCode::Char(',')) || key.code == KeyCode::F(4) {
            if let Some(Modal::Settings { return_to, .. }) = self
                .modal
                .take_if(|modal| matches!(modal, Modal::Settings { .. }))
            {
                self.modal = return_to.map(|modal| *modal);
            } else {
                let return_to = self.modal.take().map(Box::new);
                let mut draft = self.settings.clone();
                draft.rows_per_page = ROW_SIZES[self.row_size];
                draft.columns_per_page = COLUMN_SIZES[self.column_size];
                self.modal = Some(Modal::Settings {
                    draft,
                    selected: 0,
                    return_to,
                });
                self.error = false;
            }
            return None;
        }
        // Confirmation and settings own their keystrokes; global commands must not bypass them.
        if matches!(
            self.modal,
            Some(Modal::DeleteCell { .. } | Modal::Settings { .. })
        ) {
            let modal = self.modal.take().unwrap();
            self.modal_key(modal, key);
            return None;
        }
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
                        self.message("status.undone");
                    } else {
                        self.message("status.no_undo");
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
                    self.message("status.row_added");
                }
                Err(error) => self.fail(error),
            },
            KeyCode::Char('c' | 'C') => {
                self.modal = Some(Modal::Column {
                    name: Input::default(),
                    dtype: 0,
                })
            }
            KeyCode::Delete => match self.document.cell(self.row, self.column) {
                Ok(_) => {
                    self.modal = Some(Modal::DeleteCell {
                        row: self.row,
                        column: self.column,
                    })
                }
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
                        self.fail(error);
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
        if let Modal::Settings {
            draft,
            selected,
            return_to,
        } = modal
        {
            self.settings_key(draft, selected, return_to, key);
            return;
        }
        if key.code == KeyCode::Esc {
            self.quit_after_save = false;
            self.message("status.cancelled");
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
                            self.message("status.cell_updated");
                            return;
                        }
                        Err(error) => self.fail(error),
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
                            self.message("status.column_added");
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
                        self.fail(Message::new("error.enter_path").into());
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
                    self.message("status.save_cancelled");
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
            Modal::DeleteCell { row, column } => match key.code {
                KeyCode::Char('y' | 'Y') => match self.document.set_cell(row, column, None) {
                    Ok(_) => self.message("status.cell_cleared"),
                    Err(error) => self.fail(error),
                },
                KeyCode::Char('n' | 'N') => self.message("status.cancelled"),
                _ => self.modal = Some(Modal::DeleteCell { row, column }),
            },
            Modal::Settings { .. } => unreachable!("Settings handled above"),
        }
    }

    fn settings_key(
        &mut self,
        mut draft: Settings,
        mut selected: usize,
        return_to: Option<Box<Modal>>,
        key: KeyEvent,
    ) {
        if key.code == KeyCode::Esc {
            self.modal = return_to.map(|modal| *modal);
            self.message("status.cancelled");
            return;
        }
        if key.code == KeyCode::Enter
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('s' | 'S')))
        {
            if let Some(path) = &self.settings_path
                && let Err(error) = draft.save(path)
            {
                self.fail(error);
                self.modal = Some(Modal::Settings {
                    draft,
                    selected,
                    return_to,
                });
                return;
            }
            self.row_size = ROW_SIZES
                .iter()
                .position(|&n| n == draft.rows_per_page)
                .unwrap_or(1);
            self.column_size = COLUMN_SIZES
                .iter()
                .position(|&n| n == draft.columns_per_page)
                .unwrap_or(2);
            self.settings = draft;
            self.modal = return_to.map(|modal| *modal);
            self.message(if self.settings_path.is_some() {
                "status.settings_saved"
            } else {
                "status.settings_applied"
            });
            return;
        }
        match key.code {
            KeyCode::Down | KeyCode::Tab => selected = (selected + 1) % 3,
            KeyCode::Up | KeyCode::BackTab => selected = (selected + 2) % 3,
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                let next = key.code != KeyCode::Left;
                match selected {
                    0 => draft.language = draft.language.next(),
                    1 => {
                        let index = ROW_SIZES
                            .iter()
                            .position(|&n| n == draft.rows_per_page)
                            .unwrap_or(1);
                        draft.rows_per_page = ROW_SIZES[step(index, ROW_SIZES.len(), next)];
                    }
                    _ => {
                        let index = COLUMN_SIZES
                            .iter()
                            .position(|&n| n == draft.columns_per_page)
                            .unwrap_or(2);
                        draft.columns_per_page =
                            COLUMN_SIZES[step(index, COLUMN_SIZES.len(), next)];
                    }
                }
            }
            _ => {}
        }
        self.modal = Some(Modal::Settings {
            draft,
            selected,
            return_to,
        });
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
                self.status =
                    Message::with("status.saved", &[("path", path.display().to_string())]);
                self.error = false;
                if self.quit_after_save {
                    self.should_quit = true;
                }
                self.quit_after_save = false;
                true
            }
            Err(error) => {
                self.fail(error);
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
                Ok(_) => self.message("status.pasted"),
                Err(error) => self.fail(error),
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
