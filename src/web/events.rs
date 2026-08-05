//! SSE event hub and question broker.
use crate::question::{QuestionAnswers, QuestionRequest, QuestionResponse};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, oneshot};

use super::types::normalize_answers;
use super::util::{lock_mutex, random_id, EVENT_CAPACITY};

#[derive(Clone, Debug)]
pub(crate) struct EventRecord {
    pub(crate) id: u64,
    pub(crate) kind: String,
    pub(crate) data: String,
}

#[derive(Clone)]
pub(crate) struct EventHub {
    pub(crate) inner: Arc<Mutex<EventHubInner>>,
    pub(crate) sender: broadcast::Sender<EventRecord>,
}

pub(crate) struct EventHubInner {
    pub(crate) next_id: u64,
    pub(crate) records: VecDeque<EventRecord>,
}

pub(crate) struct EventSubscription {
    pub(crate) pending: VecDeque<EventRecord>,
    pub(crate) receiver: broadcast::Receiver<EventRecord>,
}

impl EventHub {
    pub(crate) fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(EventHubInner {
                next_id: 1,
                records: VecDeque::with_capacity(EVENT_CAPACITY),
            })),
            sender,
        }
    }

    pub(crate) fn publish(&self, kind: impl Into<String>, data: Value) -> u64 {
        let mut inner = lock_mutex(&self.inner);
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1);
        let record = EventRecord {
            id,
            kind: kind.into(),
            data: serde_json::to_string(&data)
                .unwrap_or_else(|_| "{\"error\":\"event serialization failed\"}".to_string()),
        };
        if inner.records.len() == EVENT_CAPACITY {
            inner.records.pop_front();
        }
        inner.records.push_back(record.clone());
        let _ = self.sender.send(record);
        id
    }

    pub(crate) fn latest_id(&self) -> u64 {
        lock_mutex(&self.inner).next_id.saturating_sub(1)
    }

    pub(crate) fn subscribe_after(&self, after: u64) -> EventSubscription {
        let mut inner = lock_mutex(&self.inner);
        let receiver = self.sender.subscribe();
        let pending = replay_records(&mut inner, after);
        EventSubscription { pending, receiver }
    }

    pub(crate) fn replay_after(&self, after: u64) -> VecDeque<EventRecord> {
        replay_records(&mut lock_mutex(&self.inner), after)
    }
}

pub(crate) fn replay_records(inner: &mut EventHubInner, after: u64) -> VecDeque<EventRecord> {
    if after > inner.next_id.saturating_sub(1) {
        return resync_record(inner);
    }
    let Some(oldest) = inner.records.front().map(|record| record.id) else {
        return VecDeque::new();
    };
    if after < oldest.saturating_sub(1) {
        return resync_record(inner);
    }
    inner
        .records
        .iter()
        .filter(|record| record.id > after)
        .cloned()
        .collect()
}

pub(crate) fn resync_record(inner: &mut EventHubInner) -> VecDeque<EventRecord> {
    let id = inner.next_id;
    inner.next_id = inner.next_id.saturating_add(1);
    VecDeque::from([EventRecord {
        id,
        kind: "resync_required".to_string(),
        data: json!({ "latest_event_id": id }).to_string(),
    }])
}

#[derive(Clone)]
pub(crate) struct QuestionBroker {
    pub(crate) pending: Arc<Mutex<HashMap<String, PendingQuestion>>>,
}

pub(crate) struct PendingQuestion {
    pub(crate) run_id: String,
    pub(crate) request: QuestionRequest,
    pub(crate) responder: oneshot::Sender<QuestionResponse>,
}

#[derive(Debug)]
pub(crate) enum AnswerFailure {
    NotFound,
    Invalid(String),
    Gone,
}

impl QuestionBroker {
    pub(crate) fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn insert(
        &self,
        run_id: &str,
        request: QuestionRequest,
        responder: oneshot::Sender<QuestionResponse>,
    ) -> String {
        let mut pending = lock_mutex(&self.pending);
        loop {
            let question_id = random_id("question", 18);
            if !pending.contains_key(&question_id) {
                pending.insert(
                    question_id.clone(),
                    PendingQuestion {
                        run_id: run_id.to_string(),
                        request,
                        responder,
                    },
                );
                return question_id;
            }
        }
    }

    pub(crate) fn answer<F>(
        &self,
        question_id: &str,
        answers: QuestionAnswers,
        before_resume: F,
    ) -> std::result::Result<(), AnswerFailure>
    where
        F: FnOnce(&str, &QuestionAnswers),
    {
        let mut all_pending = lock_mutex(&self.pending);
        let request = all_pending
            .get(question_id)
            .map(|pending| pending.request.clone())
            .ok_or(AnswerFailure::NotFound)?;
        let answers = normalize_answers(&request, answers).map_err(AnswerFailure::Invalid)?;
        let pending = all_pending
            .remove(question_id)
            .ok_or(AnswerFailure::NotFound)?;
        let run_id = pending.run_id;
        pending
            .responder
            .send(QuestionResponse::Answered(answers.clone()))
            .map_err(|_| AnswerFailure::Gone)?;
        before_resume(&run_id, &answers);
        Ok(())
    }

    pub(crate) fn cancel_run(&self, run_id: &str) {
        let cancelled = {
            let mut pending = lock_mutex(&self.pending);
            let ids = pending
                .iter()
                .filter(|(_, question)| question.run_id == run_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| pending.remove(&id))
                .collect::<Vec<_>>()
        };
        for pending in cancelled {
            let _ = pending.responder.send(QuestionResponse::Cancelled);
        }
    }
}

