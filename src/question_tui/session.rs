//! Terminal session lifecycle for the question panel.
use super::helpers::{display_inline, truncate_width, ANSWERED_BAR, BAR, MAX_PANEL_LINES};
use crate::i18n::text as t;
use crate::question::{QuestionAnswers, QuestionRequest};
use anyhow::Result;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use std::io::{self, Write};

pub(crate) struct QuestionSession {
    pub(crate) stdout: io::Stdout,
    pub(crate) anchor_y: u16,
    pub(crate) panel_lines: u16,
}

impl QuestionSession {
    pub(crate) fn start(panel_lines: u16) -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(err) = execute!(stdout, EnableBracketedPaste, Hide) {
            let _ = execute!(stdout, DisableBracketedPaste, Show);
            let _ = terminal::disable_raw_mode();
            return Err(err.into());
        }
        let (_, cursor_y) =
            crossterm::cursor::position().unwrap_or((0, panel_lines.saturating_sub(1)));
        let anchor_y = cursor_y.saturating_sub(panel_lines.saturating_sub(1));
        Ok(Self {
            stdout,
            anchor_y,
            panel_lines,
        })
    }

    pub(crate) fn finish_answered(
        &mut self,
        request: &QuestionRequest,
        answers: &QuestionAnswers,
    ) -> Result<()> {
        self.clear()?;
        let width = terminal::size().map(|(cols, _)| cols).unwrap_or(80) as usize;
        let content_width = width.saturating_sub(3).max(1);
        let keeps_blank_line = self.panel_lines > 1;
        let content_rows = self
            .panel_lines
            .saturating_sub(u16::from(keeps_blank_line))
            .max(1);
        let answer_capacity = content_rows.saturating_sub(1) as usize;
        let omitted = request.questions.len().saturating_sub(answer_capacity);
        let mut row = 0u16;
        let heading = if omitted == 0 {
            format!(
                "{} {} {}",
                t("Answered", "已回答"),
                request.questions.len(),
                t("questions", "个问题")
            )
        } else {
            format!(
                "{} {} {} · {} {}",
                t("Answered", "已回答"),
                request.questions.len(),
                t("questions", "个问题"),
                t("omitted", "省略"),
                omitted
            )
        };
        self.write_answered_line(row, &heading, content_width)?;
        row += 1;
        for (question, selected) in request.questions.iter().zip(answers).take(answer_capacity) {
            self.write_answered_line(
                row,
                &format!(
                    "{}: {}",
                    question.header,
                    display_inline(&selected.join("、"))
                ),
                content_width,
            )?;
            row += 1;
        }
        if keeps_blank_line {
            queue!(
                self.stdout,
                MoveTo(0, self.anchor_y.saturating_add(row)),
                Clear(ClearType::CurrentLine),
                crossterm::style::Print("\r\n")
            )?;
        } else {
            queue!(
                self.stdout,
                MoveTo(0, self.anchor_y.saturating_add(row.saturating_sub(1))),
                crossterm::style::Print("\r\n")
            )?;
        }
        queue!(self.stdout, Clear(ClearType::CurrentLine), Show)?;
        self.stdout.flush()?;
        Ok(())
    }

    pub(crate) fn finish_cancelled(&mut self) -> Result<()> {
        self.clear()?;
        queue!(
            self.stdout,
            MoveTo(0, self.anchor_y),
            crossterm::style::Print(format!(
                "{BAR} \x1b[2m{}\x1b[0m",
                t("Question cancelled", "已取消提问")
            )),
            MoveTo(0, self.anchor_y.saturating_add(1)),
            Clear(ClearType::CurrentLine),
            Show
        )?;
        self.stdout.flush()?;
        Ok(())
    }

    fn write_answered_line(&mut self, row: u16, text: &str, width: usize) -> Result<()> {
        queue!(
            self.stdout,
            MoveTo(0, self.anchor_y.saturating_add(row)),
            Clear(ClearType::CurrentLine),
            crossterm::style::Print(ANSWERED_BAR),
            crossterm::style::Print(" \x1b[2m\x1b[90m"),
            crossterm::style::Print(truncate_width(text, width)),
            crossterm::style::Print("\x1b[0m")
        )?;
        Ok(())
    }

    pub(crate) fn clear(&mut self) -> Result<()> {
        for row in 0..self.panel_lines {
            queue!(
                self.stdout,
                MoveTo(0, self.anchor_y.saturating_add(row)),
                Clear(ClearType::CurrentLine)
            )?;
        }
        Ok(())
    }

    pub(crate) fn resize_to_terminal(&mut self, rows: u16) {
        self.panel_lines = rows.saturating_sub(1).clamp(1, MAX_PANEL_LINES);
        self.anchor_y = self.anchor_y.min(rows.saturating_sub(self.panel_lines));
    }
}

impl Drop for QuestionSession {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, DisableBracketedPaste, Show);
        let _ = terminal::disable_raw_mode();
    }
}

