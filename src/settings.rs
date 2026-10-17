use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::i18n::{Language, Message};

pub const ROW_SIZES: [usize; 5] = [10, 20, 30, 50, 100];
pub const COLUMN_SIZES: [usize; 5] = [2, 3, 4, 6, 8];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub rows_per_page: usize,
    pub columns_per_page: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::Japanese,
            rows_per_page: 20,
            columns_per_page: 4,
        }
    }
}

impl Settings {
    pub fn path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("PARQUET_EDITOR_CONFIG") {
            ensure!(!path.is_empty(), Message::new("error.config_path"));
            return Ok(PathBuf::from(path));
        }
        ProjectDirs::from("", "", "parquet-editor")
            .map(|dirs| dirs.config_dir().join("settings.json"))
            .context(Message::new("error.config_path"))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            ROW_SIZES.contains(&self.rows_per_page)
                && COLUMN_SIZES.contains(&self.columns_per_page),
            Message::new("error.config_values")
        );
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context(Message::new("error.config_read")),
        };
        let settings: Self =
            serde_json::from_slice(&bytes).context(Message::new("error.config_parse"))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).context(Message::new("error.config_write"))?;
        let mut file = NamedTempFile::new_in(parent).context(Message::new("error.config_write"))?;
        let json = serde_json::to_vec_pretty(self).context(Message::new("error.config_write"))?;
        file.write_all(&json)
            .context(Message::new("error.config_write"))?;
        file.as_file()
            .sync_all()
            .context(Message::new("error.config_write"))?;
        file.persist(path)
            .map_err(|error| error.error)
            .context(Message::new("error.config_write"))?;
        Ok(())
    }
}
