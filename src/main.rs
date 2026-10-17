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
    i18n::{Message, text},
    settings::Settings,
    ui,
};

fn main() -> Result<()> {
    let (settings_path, mut warning) = match Settings::path() {
        Ok(path) => (Some(path), None),
        Err(error) => (None, Some(error)),
    };
    let settings = settings_path
        .as_deref()
        .map(Settings::load)
        .transpose()
        .unwrap_or_else(|error| {
            warning = Some(error);
            None
        })
        .unwrap_or_default();
    let language = settings.language;
    run(settings, settings_path, warning)
        .map_err(|error| anyhow::anyhow!(Message::from_error(&error).render(language)))
}

fn run(
    settings: Settings,
    settings_path: Option<PathBuf>,
    warning: Option<anyhow::Error>,
) -> Result<()> {
    let language = settings.language;
    let mut args = std::env::args_os().skip(1);
    let path = args.next();
    if let Some(arg) = &path {
        if arg == "--help" || arg == "-h" {
            println!("{}", text(language, "cli.help"));
            return Ok(());
        }
        if arg == "--version" || arg == "-V" {
            println!("parquet-editor {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }
    if args.next().is_some() {
        bail!(Message::new("cli.usage"));
    }
    let path = path.map(PathBuf::from);
    let document = match path {
        Some(path) if path.exists() => Document::open(&path)?,
        path => Document::new(path),
    };
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(Message::new("error.terminal"));
    }
    let mut app = App::with_settings(document, settings, settings_path);
    if let Some(error) = warning {
        app.fail(error);
    }
    let mut clipboard = Clipboard::default();
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
        // Modern Unix terminals can distinguish Ctrl+comma with the enhanced protocol.
        // Windows uses native key records instead.
        #[cfg(not(windows))]
        execute!(
            io::stdout(),
            event::PushKeyboardEnhancementFlags(
                event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        )?;
        while !app.should_quit {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            match event::read()? {
                Event::Key(key) => match app.key(key) {
                    Some(Effect::Copy) => match app.copy_cell().and_then(|v| clipboard.copy(v)) {
                        Ok(system) => app.message(if system {
                            "status.copied"
                        } else {
                            "status.copied_local"
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
    #[cfg(not(windows))]
    let keyboard_cleanup = execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
    ratatui::restore();
    result?;
    cleanup.context(Message::new("error.terminal_cleanup"))?;
    #[cfg(not(windows))]
    keyboard_cleanup.context(Message::new("error.terminal_cleanup"))?;
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
            return self
                .local
                .clone()
                .context(Message::new("error.clipboard_empty"));
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
            .context(Message::new("error.clipboard_unavailable"))
    }
}
