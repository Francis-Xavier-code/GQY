//! Session auth and CSRF origin checks.
use axum::http::header::{COOKIE, HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::error::ApiError;
use super::util::{
    lock_mutex, random_token, AUTH_COOKIE, LOGIN_ATTEMPT_LIMIT, LOGIN_WINDOW,
};

#[derive(Clone)]
pub(crate) struct WebAuth {
    pub(crate) password_digest: Option<[u8; 32]>,
    pub(crate) sessions: Arc<Mutex<HashSet<String>>>,
    pub(crate) attempts: Arc<Mutex<HashMap<IpAddr, LoginAttempt>>>,
}

#[derive(Clone, Copy)]
pub(crate) struct LoginAttempt {
    pub(crate) window_started: Instant,
    pub(crate) failures: u8,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LoginFailure {
    Invalid,
    RateLimited,
}

impl WebAuth {
    pub(crate) fn new(password: Option<&str>) -> Self {
        let password_digest = password.map(|password| {
            let mut digest = Sha256::new();
            digest.update(password.as_bytes());
            digest.finalize().into()
        });
        Self {
            password_digest,
            sessions: Arc::new(Mutex::new(HashSet::new())),
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn required(&self) -> bool {
        self.password_digest.is_some()
    }

    pub(crate) fn is_authenticated(&self, supplied: Option<&str>) -> bool {
        if !self.required() {
            return true;
        }
        supplied.is_some_and(|token| lock_mutex(&self.sessions).contains(token))
    }

    pub(crate) fn login(&self, peer: IpAddr, password: &str) -> std::result::Result<String, LoginFailure> {
        let Some(expected) = self.password_digest else {
            return Ok(String::new());
        };
        let now = Instant::now();
        {
            let mut attempts = lock_mutex(&self.attempts);
            let entry = attempts.entry(peer).or_insert(LoginAttempt {
                window_started: now,
                failures: 0,
            });
            if now.duration_since(entry.window_started) >= LOGIN_WINDOW {
                entry.window_started = now;
                entry.failures = 0;
            }
            if entry.failures >= LOGIN_ATTEMPT_LIMIT {
                return Err(LoginFailure::RateLimited);
            }
        }

        let mut digest = Sha256::new();
        digest.update(password.as_bytes());
        let supplied: [u8; 32] = digest.finalize().into();
        if !constant_time_eq(&supplied, &expected) {
            let mut attempts = lock_mutex(&self.attempts);
            if let Some(entry) = attempts.get_mut(&peer) {
                entry.failures = entry.failures.saturating_add(1);
            }
            return Err(LoginFailure::Invalid);
        }

        let token = random_token(32);
        let mut sessions = lock_mutex(&self.sessions);
        sessions.insert(token.clone());
        if sessions.len() > 64 {
            sessions.clear();
            sessions.insert(token.clone());
        }
        Ok(token)
    }
}

pub(crate) fn require_auth(headers: &HeaderMap, auth: &WebAuth) -> std::result::Result<(), ApiError> {
    if auth.is_authenticated(cookie_value(headers, AUTH_COOKIE)) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "authentication required",
        ))
    }
}

pub(crate) fn require_mutation(headers: &HeaderMap, auth: &WebAuth) -> std::result::Result<(), ApiError> {
    require_auth(headers, auth)?;
    if origin_is_allowed(headers) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "request origin is not allowed",
        ))
    }
}

pub(crate) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for header in headers.get_all(COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        for pair in header.split(';') {
            let Some((key, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if key.trim() == name {
                return Some(value.trim());
            }
        }
    }
    None
}

pub(crate) fn origin_is_allowed(headers: &HeaderMap) -> bool {
    let mut origins = headers.get_all(ORIGIN).iter();
    let Some(origin) = origins.next() else {
        return true;
    };
    if origins.next().is_some() {
        return false;
    }
    let Some(host) = headers.get(HOST).and_then(|host| host.to_str().ok()) else {
        return false;
    };
    let expected = format!("http://{host}");
    origin.to_str().is_ok_and(|origin| origin == expected)
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left = left.get(index).copied().unwrap_or(0);
        let right = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

