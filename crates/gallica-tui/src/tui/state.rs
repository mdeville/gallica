//! UI-only state: focus, edits, dialogs, scroll positions.

use gallica_core::{LogBuffer, LogEntry};
use ratatui::widgets::TableState;

/// Which panel has the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// The target input.
    #[default]
    Target,
    /// The settings list.
    Settings,
    /// The queue table.
    Queue,
    /// The log.
    Log,
}

impl Focus {
    /// The focus after `Tab`.
    pub fn next(self) -> Focus {
        match self {
            Focus::Target => Focus::Settings,
            Focus::Settings => Focus::Queue,
            Focus::Queue => Focus::Log,
            Focus::Log => Focus::Target,
        }
    }

    /// The focus after `Shift-Tab`.
    pub fn previous(self) -> Focus {
        match self {
            Focus::Target => Focus::Log,
            Focus::Settings => Focus::Target,
            Focus::Queue => Focus::Settings,
            Focus::Log => Focus::Queue,
        }
    }
}

/// Rows of the settings panel, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    /// Year range.
    Years,
    /// Max issues.
    MaxIssues,
    /// Image variant.
    Variant,
    /// Output directory.
    OutputDir,
    /// Overwrite flag.
    Overwrite,
    /// Save text flag.
    SaveText,
    /// Contact address.
    Contact,
}

impl SettingsRow {
    /// All rows in order.
    pub const ALL: [SettingsRow; 7] = [
        SettingsRow::Years,
        SettingsRow::MaxIssues,
        SettingsRow::Variant,
        SettingsRow::OutputDir,
        SettingsRow::Overwrite,
        SettingsRow::SaveText,
        SettingsRow::Contact,
    ];

    /// Label shown in the panel.
    pub fn label(self) -> &'static str {
        match self {
            SettingsRow::Years => "Years",
            SettingsRow::MaxIssues => "Max issues",
            SettingsRow::Variant => "Images",
            SettingsRow::OutputDir => "Output",
            SettingsRow::Overwrite => "Overwrite",
            SettingsRow::SaveText => "Save text",
            SettingsRow::Contact => "Contact",
        }
    }
}

/// A text field being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The target URL/ARK.
    Target,
    /// Years, as `1921-1925` or empty.
    Years,
    /// Max issues, as a number or empty.
    MaxIssues,
    /// Output directory.
    OutputDir,
    /// Contact address.
    Contact,
    /// Width for the `width:<px>` variant.
    Width,
}

/// An edit in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Which field.
    pub field: Field,
    /// Current text.
    pub text: String,
    /// Cursor position in characters.
    pub cursor: usize,
}

impl Edit {
    /// Starts editing with the given text, cursor at the end.
    pub fn new(field: Field, text: String) -> Edit {
        let cursor = text.chars().count();
        Edit {
            field,
            text,
            cursor,
        }
    }

    /// Inserts a character at the cursor.
    pub fn insert(&mut self, c: char) {
        let idx = byte_index(&self.text, self.cursor);
        self.text.insert(idx, c);
        self.cursor += 1;
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = byte_index(&self.text, self.cursor - 1);
        let end = byte_index(&self.text, self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// Deletes the character under the cursor.
    pub fn delete(&mut self) {
        let len = self.text.chars().count();
        if self.cursor >= len {
            return;
        }
        let start = byte_index(&self.text, self.cursor);
        let end = byte_index(&self.text, self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    /// Moves the cursor left.
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Moves the cursor right.
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    /// Moves the cursor to the start.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the cursor to the end.
    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// Clears the text.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
}

fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// A yes/no question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// Start a full resolution run.
    StartFullRes,
    /// Quit while a run is active.
    QuitRunning,
    /// Cancel the run.
    Cancel,
}

impl Confirm {
    /// The question.
    pub fn message(self) -> &'static str {
        match self {
            Confirm::StartFullRes => {
                "Full resolution is limited to 5 images per minute by the BnF.\nThe estimate in Details applies. Start anyway?"
            }
            Confirm::QuitRunning => "A run is in progress. Quit and cancel it?",
            Confirm::Cancel => "Cancel the run after the current views?",
        }
    }
}

/// All UI-only state.
#[derive(Debug, Default)]
pub struct UiState {
    /// Focused panel.
    pub focus: Focus,
    /// Text edit in progress.
    pub edit: Option<Edit>,
    /// Selected settings row.
    pub settings_row: usize,
    /// Queue table state (selection, scroll).
    pub table: TableState,
    /// Where the log output appends.
    pub log_buffer: LogBuffer,
    /// The log as of the last refresh, oldest first.
    pub log: Vec<LogEntry>,
    /// Lines scrolled up from the end of the log.
    pub log_scroll: usize,
    /// Help overlay shown.
    pub show_help: bool,
    /// Pending question.
    pub confirm: Option<Confirm>,
}

impl UiState {
    /// Copies the latest log lines from the buffer.
    pub fn refresh_log(&mut self) {
        self.log = self.log_buffer.snapshot();
        self.log_scroll = self.log_scroll.min(self.log.len().saturating_sub(1));
    }

    /// The selected settings row.
    pub fn settings_row(&self) -> SettingsRow {
        SettingsRow::ALL[self.settings_row.min(SettingsRow::ALL.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_handles_unicode() {
        let mut e = Edit::new(Field::Target, "été".into());
        assert_eq!(e.cursor, 3);
        e.backspace();
        assert_eq!(e.text, "ét");
        e.home();
        e.insert('x');
        assert_eq!(e.text, "xét");
        e.right();
        e.delete();
        assert_eq!(e.text, "xé");
        e.end();
        e.right();
        assert_eq!(e.cursor, 2);
    }
}
