use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, COLUMN_SIZES, Input, Modal, ROW_SIZES, Selection, Target},
    document::{COLUMN_TYPES, is_editable},
};

const BG: Color = Color::Rgb(15, 21, 32);
const PANEL: Color = Color::Rgb(22, 31, 45);
const FG: Color = Color::Rgb(218, 227, 239);
const MUTED: Color = Color::Rgb(133, 153, 177);
const ACCENT: Color = Color::Rgb(92, 224, 203);
const BORDER: Color = Color::Rgb(50, 71, 94);
const SELECTED: Color = Color::Rgb(40, 63, 88);

fn base() -> Style {
    Style::default().fg(FG).bg(BG)
}
fn muted() -> Style {
    base().fg(MUTED)
}
fn active() -> Style {
    Style::default()
        .fg(BG)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}
fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(title)
        .style(base())
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(base()), area);
    app.hits.clear();
    if area.width < 55 || area.height < 18 {
        frame.render_widget(
            Paragraph::new("Enlarge terminal to at least 55 x 18.\nCtrl+S saves / Ctrl+Q quits.")
                .style(base())
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    let filename = app
        .document
        .path
        .as_ref()
        .map_or_else(|| "Untitled.parquet".into(), |p| p.display().to_string());
    let dirty = if app.document.is_dirty() {
        "  ● UNSAVED"
    } else if !app.document.has_saved_file() {
        "  NEW"
    } else {
        "  SAVED"
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" PARQUET STUDIO ", active()),
        Span::styled(format!("  {}", clean_inline(&filename)), base()),
        Span::styled(
            dirty,
            base().fg(if app.document.is_dirty() {
                Color::Yellow
            } else {
                MUTED
            }),
        ),
    ]))
    .block(panel(" Dataset editor "));
    frame.render_widget(header, sections[0]);
    draw_grid(frame, app, sections[1]);
    draw_preview(frame, app, sections[2]);
    draw_pagination(frame, app, sections[3]);
    let hints = Text::from(vec![
        Line::styled(
            " Enter Edit  C Column  R Row  ^S Save  ^Q Quit  ^Z Undo  ^C/^V Copy/Paste  F1 Help",
            muted(),
        ),
        Line::styled(
            clean_inline(&app.status),
            base().fg(if app.error { Color::LightRed } else { ACCENT }),
        ),
    ]);
    frame.render_widget(Paragraph::new(hints), sections[4]);
    if let Some(modal) = &app.modal {
        draw_modal(frame, app, modal);
    }
}

fn draw_grid(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(format!(
        " {} rows × {} columns  |  {} selection  |  F6 changes selection ",
        app.document.rows(),
        app.document.columns(),
        app.selection.label()
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let gutter = 6_u16
        .max(app.document.rows().to_string().len() as u16 + 2)
        .min(inner.width / 3);
    let available = inner.width.saturating_sub(gutter);
    app.visible_rows = ROW_SIZES[app.row_size]
        .min(inner.height.saturating_sub(2) as usize)
        .max(1);
    app.visible_columns = COLUMN_SIZES[app.column_size]
        .min((available / 10) as usize)
        .max(1);
    let start_row = app.row / app.visible_rows * app.visible_rows;
    let start_column = app.column / app.visible_columns * app.visible_columns;
    if app.document.columns() == 0 {
        frame.render_widget(Paragraph::new("\n  Create your dataset\n\n  C  Add the first column (choose a name and type)\n  R  Add a row, then Enter to edit a cell\n\n  Open an existing file: parquet-editor <file.parquet>").style(muted()), inner);
        return;
    }
    let slots = app.visible_columns.min(app.document.columns());
    let width = available / slots as u16;
    let column_rect = |index: usize, y: u16, height: u16| {
        let offset = index as u16 * width;
        let width = if index == slots - 1 {
            available - offset
        } else {
            width
        };
        Rect::new(inner.x + gutter + offset, y, width, height)
    };
    for (i, column) in (start_column..app.document.columns())
        .take(slots)
        .enumerate()
    {
        let data = &app.document.frame().columns()[column];
        let style = if app.selection == Selection::Column && column == app.column {
            active()
        } else {
            base().fg(ACCENT).bg(PANEL)
        };
        let rect = column_rect(i, inner.y, 2);
        let title = Text::from(vec![
            Line::styled(
                fit(&format!(" {}", data.name()), rect.width as usize),
                style.add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                fit(&format!(" {}", data.dtype()), rect.width as usize),
                style,
            ),
        ]);
        frame.render_widget(Paragraph::new(title).style(style), rect);
        app.hits.push((rect, Target::Column(column)));
    }
    for (i, row) in (start_row..app.document.rows())
        .take(app.visible_rows)
        .enumerate()
    {
        let y = inner.y + 2 + i as u16;
        let rect = Rect::new(inner.x, y, gutter, 1);
        let row_style = if row == app.row && app.selection == Selection::Row {
            active()
        } else if row == app.row {
            base().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            muted()
        };
        frame.render_widget(
            Paragraph::new(format!("{:>width$} ", row + 1, width = gutter as usize - 1))
                .style(row_style),
            rect,
        );
        app.hits.push((rect, Target::Row(row)));
        for (j, column) in (start_column..app.document.columns())
            .take(slots)
            .enumerate()
        {
            let rect = column_rect(j, y, 1);
            let value = app.document.cell(row, column).ok().flatten();
            let highlight = match app.selection {
                Selection::Cell => row == app.row && column == app.column,
                Selection::Row => row == app.row,
                Selection::Column => column == app.column,
            };
            let style = if row == app.row && column == app.column {
                active()
            } else if highlight {
                base().bg(SELECTED)
            } else if value.is_none() {
                muted().add_modifier(Modifier::ITALIC)
            } else {
                base().bg(if row % 2 == 0 { BG } else { PANEL })
            };
            let text = value.map_or_else(
                || "∅ NULL".into(),
                |v| {
                    if v.is_empty() {
                        "\"\"".into()
                    } else {
                        clean_inline(&v)
                    }
                },
            );
            frame.render_widget(
                Paragraph::new(fit(
                    &format!(" {text}"),
                    rect.width.saturating_sub(1) as usize,
                ))
                .style(style),
                rect,
            );
            app.hits.push((rect, Target::Cell(row, column)));
        }
    }
    if app.document.rows() == 0 {
        frame.render_widget(
            Paragraph::new("  No rows yet. Press R to add a NULL row.").style(muted()),
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
        );
    }
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let value = app.document.cell(app.row, app.column).ok();
    let title = if let Some(column) = app.document.frame().columns().get(app.column) {
        format!(
            " Cell {}, {} · {} · {}{} ",
            app.row + 1,
            app.column + 1,
            clean_inline(column.name()),
            column.dtype(),
            if is_editable(column.dtype()) {
                ""
            } else {
                " · read only"
            }
        )
    } else {
        " Cell preview ".into()
    };
    let text = match value {
        Some(Some(text)) if text.is_empty() => "\"\" (empty string)".into(),
        Some(Some(text)) => clean_multiline(&text),
        Some(None) => "∅ NULL  —  Delete sets NULL; an empty edit creates an empty string.".into(),
        None => "Select a cell to preview its value.".into(),
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(muted())
            .block(panel(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_pagination(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " Pages · Rows {}/{} · Cols {}/{} ",
        if app.document.rows() == 0 {
            0
        } else {
            app.row / app.visible_rows + 1
        },
        app.document.rows().div_ceil(app.visible_rows),
        if app.document.columns() == 0 {
            0
        } else {
            app.column / app.visible_columns + 1
        },
        app.document.columns().div_ceil(app.visible_columns)
    );
    let block = panel(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Controls are real mouse targets as well as keyboard shortcuts.
    let controls = [
        (" Rows ".to_string(), None),
        ("[-]".into(), Some(Target::RowSize(false))),
        (format!(" {} ", ROW_SIZES[app.row_size]), None),
        ("[+]".into(), Some(Target::RowSize(true))),
        ("  Cols ".into(), None),
        ("[-]".into(), Some(Target::ColumnSize(false))),
        (format!(" {} ", COLUMN_SIZES[app.column_size]), None),
        ("[+]".into(), Some(Target::ColumnSize(true))),
        ("  ".into(), None),
        ("<R".into(), Some(Target::RowPage(false))),
        (" R>".into(), Some(Target::RowPage(true))),
        ("  <C".into(), Some(Target::ColumnPage(false))),
        (" C>".into(), Some(Target::ColumnPage(true))),
    ];
    let mut x = inner.x;
    for (label, target) in controls {
        let width = label.width() as u16;
        if x + width > inner.right() {
            break;
        }
        let rect = Rect::new(x, inner.y, width, 1);
        frame.render_widget(
            Paragraph::new(label).style(if target.is_some() {
                base().fg(ACCENT)
            } else {
                muted()
            }),
            rect,
        );
        if let Some(target) = target {
            app.hits.push((rect, target));
        }
        x += width;
    }
    let stats = format!("  view {}r × {}c", app.visible_rows, app.visible_columns);
    if x + stats.width() as u16 <= inner.right() {
        frame.render_widget(
            Paragraph::new(stats).style(muted()),
            Rect::new(x, inner.y, inner.right() - x, 1),
        );
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn draw_modal(frame: &mut Frame, app: &App, modal: &Modal) {
    let (title, hint, input, detail, height) = match modal {
        Modal::Edit(input) => (
            " Edit cell ",
            "Enter Apply · Alt/Shift+Enter Newline · Esc Cancel",
            Some(input),
            "Empty text = empty string. Use Delete in the grid for NULL.".into(),
            12,
        ),
        Modal::Column { name, dtype } => (
            " Add column ",
            "Enter Add · Tab/↑/↓ Change type · Esc Cancel",
            Some(name),
            format!(
                "Type: {}   (String / Int64 / Float64 / Boolean / UInt64)",
                COLUMN_TYPES[*dtype].0
            ),
            9,
        ),
        Modal::SavePath(input) => (
            " Save new dataset ",
            "Enter Save · Esc Cancel",
            Some(input),
            "File path (parent directory must exist):".into(),
            9,
        ),
        Modal::Overwrite(path) => (
            " Overwrite file? ",
            "Y Overwrite · N Cancel · Esc Cancel",
            None,
            format!("{} already exists. Replace it?", path.display()),
            8,
        ),
        Modal::Quit => (
            " Unsaved changes ",
            "Y Save and quit · N Discard and quit · Esc Return",
            None,
            "Save your changes before quitting?".into(),
            8,
        ),
        Modal::Help => {
            draw_help(frame);
            return;
        }
    };
    let area = centered(frame.area(), 84, height);
    frame.render_widget(Clear, area);
    let block = panel(title).border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(clean_inline(&detail)).style(muted()),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(hint).style(base().fg(ACCENT)),
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
    );
    if app.error {
        frame.render_widget(
            Paragraph::new(clean_inline(&app.status)).style(base().fg(Color::LightRed)),
            Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
        );
    }
    if let Some(input) = input {
        draw_input(
            frame,
            input,
            Rect::new(
                inner.x,
                inner.y + 2,
                inner.width,
                inner.height.saturating_sub(5).max(1),
            ),
        );
    }
}

fn draw_input(frame: &mut Frame, input: &Input, area: Rect) {
    let before = &input.text[..input.cursor];
    let line = before.chars().filter(|&c| c == '\n').count();
    let col = clean_multiline(before.rsplit('\n').next().unwrap_or("")).width();
    let top = line.saturating_sub(area.height.saturating_sub(1) as usize);
    // Scroll each line on character boundaries, including wide Japanese characters.
    let left = col.saturating_sub(area.width.saturating_sub(2) as usize);
    let lines: Vec<Line<'_>> = input
        .text
        .split('\n')
        .skip(top)
        .take(area.height as usize)
        .map(|text| {
            let text = clean_multiline(text);
            let mut offset = 0;
            let mut result = String::new();
            for c in text.chars() {
                let width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                if offset >= left {
                    result.push(c);
                } else if offset + width > left {
                    result.push(' ');
                }
                offset += width;
            }
            Line::raw(result)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(base().bg(PANEL)), area);
    frame.set_cursor_position((
        area.x + (col - left).min(area.width.saturating_sub(1) as usize) as u16,
        area.y + (line - top) as u16,
    ));
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame.area(), 70, 18);
    frame.render_widget(Clear, area);
    let help = "C  Add column       R  Add NULL row\nEnter/F2  Edit      Delete  Set NULL\nAlt/Shift+Enter  Newline   ^U  Clear editor\n^S Save   ^Q Quit (Y save / N discard)\n^Z Undo   ^C Copy cell   ^V Paste cell\nArrows  Move     PgUp/PgDn  Row page\nTab/Shift+Tab  Next/previous column page\nHome/End  First/last column; ^ also row\nF6  Cycle cell / row / column selection\nShift+Space Row    Ctrl+Space Column\n[/] Rows per page   -/+ Columns per page\nMouse: cells, headers and footer buttons\nComplex/temporal values: view + preserve\nF1/Enter/Esc Close    ^ means Ctrl";
    frame.render_widget(
        Paragraph::new(help)
            .style(base())
            .block(panel(" Keyboard & mouse "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

fn clean_inline(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => '↵',
            '\r' => '␍',
            '\t' => '⇥',
            c if c.is_control() => '�',
            c => c,
        })
        .collect()
}

fn clean_multiline(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => '\n',
            '\t' => '⇥',
            '\r' => '␍',
            c if c.is_control() => '�',
            c => c,
        })
        .collect()
}
