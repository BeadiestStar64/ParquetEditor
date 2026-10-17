use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event,
    },
    execute,
};
use parquet_editor::{
    app::{App, Effect},
    document::Document,
    ui,
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next();
    if let Some(arg) = &path {
        if arg == "--help" || arg == "-h" {
            println!(
                "Parquet Studio — a typed dataset editor\n\nUsage: parquet-editor [FILE.parquet]\n\nAn existing file is opened; a new path starts an empty dataset.\nWithout a path, Ctrl+S asks where to save.\n\nC: column  R: row  Enter: edit  Ctrl+S: save  Ctrl+Q: quit  F1: help"
            );
            return Ok(());
        }
        if arg == "--version" || arg == "-V" {
            println!("parquet-editor {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }
    if args.next().is_some() {
        bail!("Usage: parquet-editor [FILE.parquet]");
    }
    let path = path.map(PathBuf::from);
    let document = match path {
        Some(path) if path.exists() => Document::open(&path)?,
        path => Document::new(path),
    };
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Run this app in an interactive terminal. Use --help for usage.");
    }
    let mut app = App::new(document);
    let mut clipboard = Clipboard::default();
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
        while !app.should_quit {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            match event::read()? {
                Event::Key(key) => match app.key(key) {
                    Some(Effect::Copy) => match app.copy_cell().and_then(|v| clipboard.copy(v)) {
                        Ok(system) => app.message(if system {
                            "Cell copied to clipboard"
                        } else {
                            "Cell copied internally (system clipboard unavailable)"
                        }),
                        Err(error) => app.fail(error),
                    },
                    Some(Effect::Paste) => match clipboard.paste() {
                        Ok(value) => app.paste(value.as_deref()),
                        Err(error) => app.fail(error),
                    },
                    None => {}
                },
                Event::Paste(text) => app.paste(Some(&text)),
                Event::Mouse(event) => app.mouse(event),
                _ => {}
            }
        }
        Ok(())
    })();
    // Restore terminal even when draw, input, or clipboard handling fails.
    let cleanup = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    result?;
    cleanup.context("Could not disable terminal input extensions")?;
    Ok(())
}

/// Use the system clipboard when available, with a nullable in-process fallback.
#[derive(Default)]
struct Clipboard {
    system: Option<arboard::Clipboard>,
    local: Option<Option<String>>,
    exported: Option<String>,
    local_only: bool,
}

impl Clipboard {
    fn copy(&mut self, value: Option<String>) -> Result<bool> {
        let text = value.clone().unwrap_or_default();
        self.local = Some(value);
        self.exported = None;
        self.local_only = true;
        if self.system.is_none() {
            self.system = arboard::Clipboard::new().ok();
        }
        if let Some(system) = &mut self.system
            && system.set_text(text.clone()).is_ok()
        {
            self.exported = Some(text);
            self.local_only = false;
            return Ok(true);
        }
        Ok(false)
    }

    fn paste(&mut self) -> Result<Option<String>> {
        if self.local_only {
            return self.local.clone().context("No cell copied yet");
        }
        if self.system.is_none() {
            self.system = arboard::Clipboard::new().ok();
        }
        if let Some(system) = &mut self.system
            && let Ok(text) = system.get_text()
        {
            if self.exported.as_deref() == Some(&text)
                && let Some(local) = &self.local
            {
                return Ok(local.clone());
            }
            return Ok(Some(text));
        }
        self.local
            .clone()
            .context("Clipboard unavailable and no cell copied yet")
    }
}
