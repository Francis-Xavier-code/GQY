//! Question interaction state machine.
use super::helpers::{insert_text, remove_at_cursor, remove_before_cursor};
use crate::question::{
    validate_answers, QuestionAnswers, QuestionPrompt, QuestionRequest,
};
use anyhow::{bail, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Instant;

pub(crate) struct QuestionState {
    pub(crate) tab: usize,
    pub(crate) selected: Vec<usize>,
    pub(crate) scroll_starts: Vec<usize>,
    pub(crate) answers: QuestionAnswers,
    pub(crate) custom_answers: Vec<String>,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
    pub(crate) edit_cursor: usize,
    pub(crate) cancel_armed_until: Option<Instant>,
}

impl QuestionState {
    pub(crate) fn new(request: &QuestionRequest) -> Self {
        Self {
            tab: 0,
            selected: vec![0; request.questions.len()],
            scroll_starts: vec![0; request.questions.len() + usize::from(request.needs_review())],
            answers: vec![Vec::new(); request.questions.len()],
            custom_answers: vec![String::new(); request.questions.len()],
            editing: false,
            edit_buffer: String::new(),
            edit_cursor: 0,
            cancel_armed_until: None,
        }
    }

    pub(crate) fn on_confirm(&self, request: &QuestionRequest) -> bool {
        request.needs_review() && self.tab == request.questions.len()
    }

    pub(crate) fn tab_count(&self, request: &QuestionRequest) -> usize {
        request.questions.len() + usize::from(request.needs_review())
    }

    pub(crate) fn previous_tab(&mut self, request: &QuestionRequest) {
        let count = self.tab_count(request);
        self.tab = (self.tab + count - 1) % count;
    }

    pub(crate) fn next_tab(&mut self, request: &QuestionRequest) {
        self.tab = (self.tab + 1) % self.tab_count(request);
    }

    pub(crate) fn previous_option(&mut self, question: &QuestionPrompt) {
        let count = option_count(question);
        if count > 0 {
            let selected = &mut self.selected[self.tab];
            *selected = (*selected + count - 1) % count;
        }
    }

    pub(crate) fn next_option(&mut self, question: &QuestionPrompt) {
        let count = option_count(question);
        if count > 0 {
            self.selected[self.tab] = (self.selected[self.tab] + 1) % count;
        }
    }

    pub(crate) fn activate_current(&mut self, request: &QuestionRequest) -> Result<()> {
        let question = &request.questions[self.tab];
        let selected = self.selected[self.tab];
        if selected == question.options.len() && question.custom {
            self.editing = true;
            self.edit_buffer = self.custom_answers[self.tab].clone();
            self.edit_cursor = self.edit_buffer.chars().count();
            return Ok(());
        }
        let Some(option) = question.options.get(selected) else {
            bail!("selected question option is out of range");
        };
        if question.multiple {
            toggle_answer(&mut self.answers[self.tab], &option.label);
        } else {
            self.answers[self.tab] = vec![option.label.clone()];
            self.advance_after_single(request);
        }
        Ok(())
    }

    pub(crate) fn toggle_current(&mut self, request: &QuestionRequest) -> Result<()> {
        let question = &request.questions[self.tab];
        let selected = self.selected[self.tab];
        if selected == question.options.len() && question.custom {
            let custom = self.custom_answers[self.tab].trim();
            if custom.is_empty() {
                return self.activate_current(request);
            }
            toggle_answer(&mut self.answers[self.tab], custom);
            return Ok(());
        }
        self.activate_current(request)
    }

    pub(crate) fn advance_after_single(&mut self, request: &QuestionRequest) {
        if request.questions.len() == 1 && !request.needs_review() {
            return;
        }
        self.tab = (self.tab + 1).min(self.tab_count(request) - 1);
    }

    pub(crate) fn go_to_first_unanswered(&mut self, request: &QuestionRequest) {
        if let Some(index) = self.answers.iter().position(Vec::is_empty) {
            self.tab = index.min(request.questions.len().saturating_sub(1));
        }
    }
}

pub(crate) fn handle_editing_key(
    request: &QuestionRequest,
    state: &mut QuestionState,
    key: KeyEvent,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_text(&mut state.edit_buffer, &mut state.edit_cursor, "\n");
        }
        KeyCode::Esc => {
            state.editing = false;
            state.edit_buffer.clear();
            state.edit_cursor = 0;
        }
        KeyCode::Enter => {
            let value = state.edit_buffer.trim().to_string();
            if value.is_empty() {
                let previous = std::mem::take(&mut state.custom_answers[state.tab]);
                state.answers[state.tab].retain(|answer| answer != &previous);
                state.editing = false;
                state.edit_buffer.clear();
                state.edit_cursor = 0;
                return Ok(false);
            }
            let question = &request.questions[state.tab];
            let previous = std::mem::replace(&mut state.custom_answers[state.tab], value.clone());
            if !previous.is_empty() {
                state.answers[state.tab].retain(|answer| answer != &previous);
            }
            if question.multiple {
                if !state.answers[state.tab].contains(&value) {
                    state.answers[state.tab].push(value);
                }
            } else {
                state.answers[state.tab] = vec![value];
            }
            state.editing = false;
            state.edit_buffer.clear();
            state.edit_cursor = 0;
            if !question.multiple {
                state.advance_after_single(request);
            }
            return Ok(true);
        }
        KeyCode::Left => state.edit_cursor = state.edit_cursor.saturating_sub(1),
        KeyCode::Right => {
            state.edit_cursor = (state.edit_cursor + 1).min(state.edit_buffer.chars().count())
        }
        KeyCode::Home => state.edit_cursor = 0,
        KeyCode::End => state.edit_cursor = state.edit_buffer.chars().count(),
        KeyCode::Backspace => remove_before_cursor(&mut state.edit_buffer, &mut state.edit_cursor),
        KeyCode::Delete => remove_at_cursor(&mut state.edit_buffer, state.edit_cursor),
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_text(
                &mut state.edit_buffer,
                &mut state.edit_cursor,
                &ch.to_string(),
            );
        }
        _ => {}
    }
    Ok(false)
}

pub(crate) fn submitted_answers(
    request: &QuestionRequest,
    state: &QuestionState,
) -> Result<Option<QuestionAnswers>> {
    if state.editing || state.answers.iter().any(Vec::is_empty) {
        return Ok(None);
    }
    if request.needs_review() && !state.on_confirm(request) {
        return Ok(None);
    }
    validate_answers(request, &state.answers)?;
    Ok(Some(state.answers.clone()))
}

pub(crate) fn option_count(question: &QuestionPrompt) -> usize {
    question.options.len() + usize::from(question.custom)
}

pub(crate) fn toggle_answer(answers: &mut Vec<String>, value: &str) {
    if let Some(index) = answers.iter().position(|answer| answer == value) {
        answers.remove(index);
    } else {
        answers.push(value.to_string());
    }
}

