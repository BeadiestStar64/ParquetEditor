use std::{
    collections::VecDeque,
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use polars::prelude::{
    AnyValue, Column, DataFrame, DataType, ParquetReader, ParquetWriter, SerReader,
};
use tempfile::NamedTempFile;

use crate::i18n::Message;

const UNDO_LIMIT: usize = 100;

#[derive(Clone)]
struct Revision {
    frame: DataFrame,
    id: u64,
}

/// A typed dataset. Snapshots share Polars' immutable column buffers.
pub struct Document {
    current: Revision,
    history: VecDeque<Revision>,
    saved_id: u64,
    next_id: u64,
    has_saved_file: bool,
    pub path: Option<PathBuf>,
}

impl Document {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            current: Revision {
                frame: DataFrame::default(),
                id: 0,
            },
            history: VecDeque::new(),
            saved_id: 0,
            next_id: 1,
            has_saved_file: false,
            path,
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        // Disable mmap: Windows must allow replacing the source on save.
        let context = || Message::with("error.read", &[("path", path.display().to_string())]);
        let bytes = std::fs::read(path).with_context(context)?;
        let frame = ParquetReader::new(Cursor::new(bytes))
            .finish()
            .with_context(context)?;
        let mut doc = Self::new(Some(path.to_path_buf()));
        doc.current.frame = frame;
        doc.has_saved_file = true;
        Ok(doc)
    }

    pub fn frame(&self) -> &DataFrame {
        &self.current.frame
    }
    pub fn rows(&self) -> usize {
        self.frame().height()
    }
    pub fn columns(&self) -> usize {
        self.frame().width()
    }
    pub fn is_dirty(&self) -> bool {
        self.current.id != self.saved_id
    }
    pub fn undo_len(&self) -> usize {
        self.history.len()
    }
    pub fn has_saved_file(&self) -> bool {
        self.has_saved_file
    }

    fn commit(&mut self, frame: DataFrame) {
        let previous = std::mem::replace(
            &mut self.current,
            Revision {
                frame,
                id: self.next_id,
            },
        );
        self.next_id += 1;
        self.history.push_back(previous);
        if self.history.len() > UNDO_LIMIT {
            self.history.pop_front();
        }
    }

    pub fn undo(&mut self) -> bool {
        if let Some(previous) = self.history.pop_back() {
            self.current = previous;
            true
        } else {
            false
        }
    }

    pub fn add_column(&mut self, name: &str, dtype: DataType) -> Result<()> {
        let name = name.trim();
        ensure!(!name.is_empty(), Message::new("error.column_empty"));
        ensure!(
            self.frame().column(name).is_err(),
            Message::with("error.column_duplicate", &[("name", name.to_string())])
        );
        let mut frame = self.frame().clone();
        frame.with_column(Column::full_null(name.into(), self.rows(), &dtype))?;
        self.commit(frame);
        Ok(())
    }

    pub fn add_row(&mut self) -> Result<()> {
        ensure!(self.columns() > 0, Message::new("error.add_column_first"));
        let mut frame = self.frame().clone();
        let row = DataFrame::full_null(self.frame().schema().as_ref(), 1);
        frame.vstack_mut(&row)?;
        self.commit(frame);
        Ok(())
    }

    pub fn cell(&self, row: usize, column: usize) -> Result<Option<String>> {
        ensure!(
            row < self.rows() && column < self.columns(),
            Message::new("error.no_cell")
        );
        let value = self.frame().columns()[column].get(row)?;
        Ok(match value {
            AnyValue::Null => None,
            AnyValue::String(value) => Some(value.to_owned()),
            AnyValue::StringOwned(value) => Some(value.to_string()),
            // AnyValue's pretty Display may round floats; editing must use round-trip text.
            AnyValue::Float32(value) => Some(value.to_string()),
            AnyValue::Float64(value) => Some(value.to_string()),
            value => Some(value.to_string()),
        })
    }

    pub fn set_cell(&mut self, row: usize, column: usize, value: Option<&str>) -> Result<bool> {
        ensure!(
            row < self.rows() && column < self.columns(),
            Message::new("error.no_cell")
        );
        let original = &self.frame().columns()[column];
        let name = original.name().clone();
        let dtype = original.dtype();
        let replacement = match value {
            None => Column::full_null(name.clone(), 1, dtype),
            Some(text) => {
                ensure!(
                    is_editable(dtype),
                    Message::with("error.unsupported_type", &[("type", dtype.to_string())])
                );
                if dtype == &DataType::Boolean {
                    let parsed = match text.trim().to_ascii_lowercase().as_str() {
                        "true" | "1" => true,
                        "false" | "0" => false,
                        _ => bail!(Message::new("error.boolean")),
                    };
                    Column::new(name.clone(), [parsed])
                } else {
                    let parsed = if dtype == &DataType::String {
                        text
                    } else {
                        text.trim()
                    };
                    Column::new(name.clone(), [parsed])
                        .strict_cast(dtype)
                        .with_context(|| {
                            Message::with("error.invalid_value", &[("type", dtype.to_string())])
                        })?
                }
            }
        };
        if original.slice(row as i64, 1).equals_missing(&replacement) {
            return Ok(false);
        }
        let mut updated = original.slice(0, row);
        updated.append(&replacement)?;
        updated.append(&original.slice((row + 1) as i64, self.rows() - row - 1))?;
        let mut frame = self.frame().clone();
        frame.replace_column(column, updated)?;
        self.commit(frame);
        Ok(true)
    }

    /// Write beside the destination, then atomically replace it after a successful flush.
    /// Saving a failed edit never truncates an existing file.
    pub fn save(&mut self, path: &Path, overwrite: bool) -> Result<()> {
        ensure!(self.columns() > 0, Message::new("error.save_no_columns"));
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = NamedTempFile::new_in(parent).context(Message::new("error.save_temp"))?;
        let mut frame = self.frame().clone();
        ParquetWriter::new(temp.as_file_mut())
            .finish(&mut frame)
            .context(Message::new("error.save_encode"))?;
        temp.as_file()
            .sync_all()
            .context(Message::new("error.save_flush"))?;
        if overwrite {
            temp.persist(path)
                .map_err(|e| e.error)
                .context(Message::new("error.save_replace"))?;
        } else {
            temp.persist_noclobber(path)
                .map_err(|e| e.error)
                .context(Message::new("error.save_create"))?;
        }
        self.path = Some(path.to_path_buf());
        self.saved_id = self.current.id;
        self.has_saved_file = true;
        Ok(())
    }
}

pub fn is_editable(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::String
            | DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::Int128
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::UInt128
            | DataType::Float32
            | DataType::Float64
    )
}

pub const COLUMN_TYPES: [(&str, DataType); 5] = [
    ("String", DataType::String),
    ("Int64", DataType::Int64),
    ("Float64", DataType::Float64),
    ("Boolean", DataType::Boolean),
    ("UInt64", DataType::UInt64),
];
