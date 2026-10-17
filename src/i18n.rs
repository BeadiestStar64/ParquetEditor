use std::{collections::BTreeMap, fmt, sync::OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Language {
    #[default]
    #[serde(rename = "ja_JP")]
    Japanese,
    #[serde(rename = "en_US")]
    English,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::Japanese => "ja_JP",
            Self::English => "en_US",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Japanese => Self::English,
            Self::English => Self::Japanese,
        }
    }
}

type Catalog = BTreeMap<String, String>;

fn catalog(language: Language) -> &'static Catalog {
    static JAPANESE: OnceLock<Catalog> = OnceLock::new();
    static ENGLISH: OnceLock<Catalog> = OnceLock::new();
    let (cache, json) = match language {
        Language::Japanese => (&JAPANESE, include_str!("../translate/ja_JP.json")),
        Language::English => (&ENGLISH, include_str!("../translate/en_US.json")),
    };
    cache
        .get_or_init(|| serde_json::from_str(json).expect("Bundled translation JSON must be valid"))
}

pub fn text(language: Language, key: &'static str) -> &'static str {
    catalog(language)
        .get(key)
        .or_else(|| catalog(Language::English).get(key))
        .map_or(key, String::as_str)
}

/// Substitute named placeholders once, without interpreting braces in user data.
pub fn format(language: Language, key: &'static str, arguments: &[(&str, String)]) -> String {
    let mut remaining = text(language, key);
    let mut result = String::new();
    while let Some(start) = remaining.find('{') {
        result.push_str(&remaining[..start]);
        let Some(end) = remaining[start..].find('}').map(|end| start + end) else {
            result.push_str(&remaining[start..]);
            return result;
        };
        let name = &remaining[start + 1..end];
        if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push_str(&remaining[start..=end]);
        }
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    result
}

/// Retain message keys so changing language also updates the current status/error.
#[derive(Clone, Debug)]
pub struct Message {
    pub key: &'static str,
    pub arguments: Vec<(&'static str, String)>,
    detail: Option<String>,
}

impl Message {
    pub fn new(key: &'static str) -> Self {
        Self::with(key, &[])
    }
    pub fn with(key: &'static str, arguments: &[(&'static str, String)]) -> Self {
        Self {
            key,
            arguments: arguments.to_vec(),
            detail: None,
        }
    }
    pub fn render(&self, language: Language) -> String {
        let translated = format(language, self.key, &self.arguments);
        match &self.detail {
            Some(detail) => format!("{translated}: {detail}"),
            None => translated,
        }
    }
    pub fn from_error(error: &anyhow::Error) -> Self {
        // anyhow's downcast also sees typed context values; std::error::Error::source
        // does not expose those values as standalone errors.
        let mut message = error
            .downcast_ref::<Self>()
            .cloned()
            .unwrap_or_else(|| Self::new("error.unexpected"));
        if error.root_cause().downcast_ref::<Self>().is_none() {
            message.detail = Some(error.root_cause().to_string());
        }
        message
    }
}

impl fmt::Display for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render(Language::English))
    }
}

impl std::error::Error for Message {}
