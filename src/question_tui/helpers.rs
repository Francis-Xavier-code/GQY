//! Text/cursor helpers and shared panel constants.
use crate::question::MAX_CUSTOM_ANSWER_CHARS;
use anyhow::Result;
use std::io::{self, Write};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};


pub(crate) const MAX_PANEL_LINES: u16 = 16;
pub(crate) const CANCEL_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);
pub(crate) const BAR: &str = "\x1b[1m\x1b[35m┃\x1b[0m";
pub(crate) const ANSWERED_BAR: &str = "\x1b[2m\x1b[90m┃\x1b[0m";

pub(crate) fn wrap_display_text(value: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if current_width > 0 && current_width.saturating_add(char_width) > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width = current_width.saturating_add(char_width);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

pub(crate) fn reserve_space(lines: u16) -> Result<()> {
    for _ in 1..lines {
        println!();
    }
    io::stdout().flush()?;
    Ok(())
}

pub(crate) fn insert_text(value: &mut String, cursor: &mut usize, text: &str) {
    let remaining = MAX_CUSTOM_ANSWER_CHARS.saturating_sub(value.chars().count());
    if remaining == 0 {
        return;
    }
    let sanitized = text
        .chars()
        .flat_map(|ch| {
            if ch == '\t' {
                "  ".chars().collect::<Vec<_>>()
            } else if ch == '\n' || !ch.is_control() {
                vec![ch]
            } else {
                Vec::new()
            }
        })
        .take(remaining)
        .collect::<String>();
    let byte = byte_index(value, *cursor);
    value.insert_str(byte, &sanitized);
    *cursor += sanitized.chars().count();
}

pub(crate) fn display_inline(value: &str) -> String {
    value
        .chars()
        .filter_map(|ch| match ch {
            '\n' | '\r' => Some('↵'),
            '\t' => Some(' '),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect()
}

pub(crate) fn editor_view(value: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let display = display_inline(value);
    let before = display_inline(&value.chars().take(cursor).collect::<String>());
    let cursor_width = UnicodeWidthStr::width(before.as_str());
    if UnicodeWidthStr::width(display.as_str()) <= width {
        return (display, cursor_width.min(width));
    }
    if cursor_width < width {
        return (truncate_plain_width(&display, width), cursor_width);
    }

    let tail_budget = width.saturating_sub(1);
    let mut tail = String::new();
    let mut tail_width = 0usize;
    for ch in before.chars().rev() {
        let ch_width = ch.width().unwrap_or(0);
        if tail_width + ch_width > tail_budget {
            break;
        }
        tail.insert(0, ch);
        tail_width += ch_width;
    }
    let after = display
        .chars()
        .skip(before.chars().count())
        .collect::<String>();
    let mut view = format!("…{tail}");
    let remaining = width.saturating_sub(1 + tail_width);
    view.push_str(&truncate_plain_width(&after, remaining));
    (view, (1 + tail_width).min(width))
}

pub(crate) fn truncate_plain_width(value: &str, max_width: usize) -> String {
    let mut output = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output
}

pub(crate) fn remove_before_cursor(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let start = byte_index(value, *cursor - 1);
    let end = byte_index(value, *cursor);
    value.replace_range(start..end, "");
    *cursor -= 1;
}

pub(crate) fn remove_at_cursor(value: &mut String, cursor: usize) {
    if cursor >= value.chars().count() {
        return;
    }
    let start = byte_index(value, cursor);
    let end = byte_index(value, cursor + 1);
    value.replace_range(start..end, "");
}

pub(crate) fn byte_index(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

pub(crate) fn truncate_width(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(strip_ansi(value).as_str()) <= max_width {
        return value.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let budget = max_width.saturating_sub(3);
    let mut output = String::new();
    let mut width = 0usize;
    let mut in_escape = false;
    for ch in value.chars() {
        if ch == '\x1b' {
            in_escape = true;
            output.push(ch);
            continue;
        }
        if in_escape {
            output.push(ch);
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        let char_width = ch.width().unwrap_or(0);
        if width + char_width > budget {
            break;
        }
        output.push(ch);
        width += char_width;
    }
    output.push_str("...\x1b[0m");
    output
}

pub(crate) fn strip_ansi(value: &str) -> String {
    let mut output = String::new();
    let mut in_escape = false;
    for ch in value.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
        } else {
            output.push(ch);
        }
    }
    output
}

