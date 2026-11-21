// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tranquillity's remote console forms.
//!
//! `RemoteConsole.cs` at the pin: `POST /StartSession/` with `USER` and `PASS`,
//! `POST /SessionCommand/` with `ID` and `COMMAND`, `POST /ReadResponses/{id}/`,
//! and `POST /CloseSession/` with `ID`. The console is not registered unless
//! `[Network] ConsoleUser` is set.
//!
//! `RemoteConsole.cs` `DoExpire` drops a connection idle for more than 500
//! seconds. `last` moves when the session starts and when a poll checks or
//! reads responses, not when a command arrives. The C# timer fires every 5
//! seconds. A session past `console.sessions` is refused.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ferroan_diag::{CONSOLE_SESSIONS, Kind};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::post;
use uuid::Uuid;

use crate::level::{self, RuntimeLevel};

const SCROLL_LIMIT: usize = 1000;
/// `Environment.TickCount - last > 500000` in `DoExpire`.
const IDLE_LIMIT_MS: u64 = 500_000;
const MAX_SESSIONS: usize = 32;

pub(crate) struct ConsoleState {
    user: String,
    pass: String,
    origin: Option<HeaderValue>,
    version: String,
    level: RuntimeLevel,
    sessions: Mutex<HashMap<Uuid, ConsoleSession>>,
    scroll: Mutex<Scroll>,
    started: Instant,
    /// When set, [`ConsoleState::now_ms`] ignores the monotonic clock.
    manual: AtomicBool,
    manual_ms: std::sync::atomic::AtomicU64,
    sweeper: AtomicBool,
}

struct ConsoleSession {
    seen: u64,
    last_ms: u64,
}

struct Scroll {
    lines: VecDeque<Line>,
}

struct Line {
    number: u64,
    level: String,
    prompt: bool,
    text: String,
}

pub(crate) fn state(
    user: String,
    pass: String,
    origin: Option<String>,
    version: String,
    level: RuntimeLevel,
) -> Result<Arc<ConsoleState>, crate::LogError> {
    let origin = match origin {
        Some(value) => Some(HeaderValue::from_str(&value).map_err(|_| crate::LogError::Origin)?),
        None => None,
    };
    Ok(Arc::new(ConsoleState {
        user,
        pass,
        origin,
        version,
        level,
        sessions: Mutex::new(HashMap::new()),
        scroll: Mutex::new(Scroll {
            lines: VecDeque::new(),
        }),
        started: Instant::now(),
        manual: AtomicBool::new(false),
        manual_ms: std::sync::atomic::AtomicU64::new(0),
        sweeper: AtomicBool::new(false),
    }))
}

pub(crate) fn router(state: Arc<ConsoleState>) -> Router {
    Router::new()
        .route("/StartSession/", post(start_session))
        .route("/StartSession", post(start_session))
        .route("/SessionCommand/", post(session_command))
        .route("/SessionCommand", post(session_command))
        .route("/ReadResponses/{id}/", post(read_responses))
        .route("/ReadResponses/{id}", post(read_responses))
        .route("/CloseSession/", post(close_session))
        .route("/CloseSession", post(close_session))
        .with_state(state)
}

impl ConsoleState {
    pub(crate) fn push_plain(&self, level_name: &str, text: &str) {
        let mut scroll = self
            .scroll
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let number = scroll.lines.back().map(|line| line.number + 1).unwrap_or(1);
        scroll.lines.push_back(Line {
            number,
            level: level_name.to_owned(),
            prompt: false,
            text: text.to_owned(),
        });
        while scroll.lines.len() > SCROLL_LIMIT {
            scroll.lines.pop_front();
        }
    }
}

async fn start_session(State(state): State<Arc<ConsoleState>>, body: String) -> Response {
    let form = form_pairs(&body);
    let user = form_get(&form, "USER");
    let pass = form_get(&form, "PASS");
    if user != Some(state.user.as_str()) || pass != Some(state.pass.as_str()) {
        return empty(StatusCode::UNAUTHORIZED, "text/plain", &state);
    }
    let id = Uuid::new_v4();
    if !state.try_open(id) {
        return empty(StatusCode::SERVICE_UNAVAILABLE, "text/plain", &state);
    }
    ensure_sweeper(&state);
    state.push_prompt();
    let xml = format!(
        "<?xml version=\"1.0\"?><ConsoleSession><SessionID>{id}</SessionID><Prompt>Ferroan# </Prompt><HelpTree>{help}</HelpTree></ConsoleSession>",
        help = help_tree(),
    );
    xml_response(StatusCode::OK, "text/xml", xml, false, &state)
}

async fn session_command(State(state): State<Arc<ConsoleState>>, body: String) -> Response {
    let form = form_pairs(&body);
    let Some(id) = form_get(&form, "ID").and_then(|text| Uuid::parse_str(text).ok()) else {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    };
    state.expire_idle();
    let known = state
        .sessions
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .contains_key(&id);
    if !known {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    }
    let Some(command) = form_get(&form, "COMMAND") else {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    };
    run_command(&state, command);
    xml_response(
        StatusCode::OK,
        "text/xml",
        "<?xml version=\"1.0\"?><ConsoleSession><Result>OK</Result></ConsoleSession>",
        false,
        &state,
    )
}

async fn read_responses(
    State(state): State<Arc<ConsoleState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = Uuid::parse_str(&id).ok() else {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    };
    state.expire_idle();
    let now = state.now_ms();
    let mut sessions = state
        .sessions
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(session) = sessions.get_mut(&id) else {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    };
    session.last_ms = now;
    let seen = &mut session.seen;
    let scroll = state
        .scroll
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let pending: Vec<&Line> = scroll
        .lines
        .iter()
        .filter(|line| line.number > *seen)
        .collect();
    if pending.is_empty() {
        return xml_response(
            StatusCode::OK,
            "text/xml",
            "<?xml version=\"1.0\"?><ConsoleSession/>",
            true,
            &state,
        );
    }
    let mut body = String::from("<?xml version=\"1.0\"?><ConsoleSession>");
    for line in &pending {
        let prompt = if line.prompt { "true" } else { "false" };
        body.push_str(&format!(
            "<Line Number=\"{number}\" Level=\"{level}\" Prompt=\"{prompt}\" Command=\"false\" Input=\"false\">{text}</Line>",
            number = line.number,
            level = xml_escape(&line.level),
            text = xml_escape(&line.text),
        ));
        *seen = line.number;
    }
    body.push_str("</ConsoleSession>");
    xml_response(StatusCode::OK, "application/xml", body, true, &state)
}

async fn close_session(State(state): State<Arc<ConsoleState>>, body: String) -> Response {
    state.expire_idle();
    let form = form_pairs(&body);
    let Some(id) = form_get(&form, "ID").and_then(|text| Uuid::parse_str(text).ok()) else {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    };
    let removed = state
        .sessions
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(&id)
        .is_some();
    if !removed {
        return empty(StatusCode::NOT_FOUND, "text/plain", &state);
    }
    xml_response(
        StatusCode::OK,
        "text/xml",
        "<?xml version=\"1.0\"?><ConsoleSession><Result>OK</Result></ConsoleSession>",
        false,
        &state,
    )
}

impl ConsoleState {
    fn now_ms(&self) -> u64 {
        if self.manual.load(Ordering::Relaxed) {
            return self.manual_ms.load(Ordering::Relaxed);
        }
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    #[cfg(test)]
    fn set_now_ms(&self, ms: u64) {
        self.manual_ms.store(ms, Ordering::Relaxed);
        self.manual.store(true, Ordering::Relaxed);
    }

    fn expire_idle(&self) {
        let now = self.now_ms();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        sessions.retain(|_, session| now.saturating_sub(session.last_ms) <= IDLE_LIMIT_MS);
    }

    /// Opens `id` unless the live set is already at [`MAX_SESSIONS`].
    /// Expired sessions are dropped first, matching a `DoExpire` sweep before
    /// `HandleHttpStartSession` adds a row.
    fn try_open(&self, id: Uuid) -> bool {
        let now = self.now_ms();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        sessions.retain(|_, session| now.saturating_sub(session.last_ms) <= IDLE_LIMIT_MS);
        if sessions.len() >= MAX_SESSIONS {
            let observed = (sessions.len() as u64).saturating_add(1);
            drop(sessions);
            ferroan_diag::refuse(
                CONSOLE_SESSIONS,
                Kind::Hard,
                MAX_SESSIONS as u64,
                observed,
                "console",
            );
            return false;
        }
        sessions.insert(
            id,
            ConsoleSession {
                seen: 0,
                last_ms: now,
            },
        );
        true
    }

    fn push_prompt(&self) {
        let mut scroll = self
            .scroll
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let number = scroll.lines.back().map(|line| line.number + 1).unwrap_or(1);
        scroll.lines.push_back(Line {
            number,
            level: String::new(),
            prompt: true,
            text: "0:0:+++Ferroan# ".to_owned(),
        });
    }
}

fn ensure_sweeper(state: &Arc<ConsoleState>) {
    if state
        .sweeper
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        // Tests move the clock themselves and call the handlers, which sweep.
        // Spawning here would race those tests, and `router` is built before a
        // runtime exists for `cargo xtask console-client`.
        #[cfg(not(test))]
        {
            let state = Arc::clone(state);
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    state.expire_idle();
                }
            });
        }
    }
}

fn run_command(state: &ConsoleState, command: &str) {
    let parts = split_command(command);
    let words: Vec<&str> = parts.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["show", "version"] => {
            state.push_plain("Info", &format!("0:0:{}", state.version));
        }
        ["diag", "dump"] => match ferroan_diag::write_dump() {
            Ok(path) => state.push_plain("Info", &format!("0:0:diag dump {}", path.display())),
            Err(error) => state.push_plain("Info", &format!("0:0:diag dump failed: {error}")),
        },
        ["get", "log", "level"] => {
            let name = level::name(state.level.get());
            state.push_plain("Info", &format!("0:0:Console log level is {name}"));
        }
        ["set", "log", "level", asked] => match level::parse(asked) {
            Some(rank) => {
                state.level.set(rank);
                let name = level::name(rank);
                state.push_plain("Info", &format!("0:0:Console log level is {name}"));
            }
            None => {
                state.push_plain(
                    "Info",
                    "0:0:set log level accepts ALL, DEBUG, INFO, WARN, ERROR, FATAL, OFF",
                );
                let name = level::name(state.level.get());
                state.push_plain("Info", &format!("0:0:Console log level is {name}"));
            }
        },
        _ => state.push_plain("Info", "0:0:Invalid command"),
    }
}

fn help_tree() -> &'static str {
    "<Level Name=\"show\"><Level Name=\"version\"><Command><Module>General</Module><Shared>False</Shared><HelpText>show version</HelpText><LongHelp>Show server version</LongHelp><Description></Description></Command></Level></Level><Level Name=\"diag\"><Level Name=\"dump\"><Command><Module>General</Module><Shared>False</Shared><HelpText>diag dump</HelpText><LongHelp>Write a diagnostic snapshot to LOGDIR or .scratch</LongHelp><Description></Description></Command></Level></Level><Level Name=\"set\"><Level Name=\"log\"><Level Name=\"level\"><Command><Module>General</Module><Shared>False</Shared><HelpText>set log level</HelpText><LongHelp>Set the console logging level</LongHelp><Description></Description></Command></Level></Level></Level><Level Name=\"get\"><Level Name=\"log\"><Level Name=\"level\"><Command><Module>General</Module><Shared>False</Shared><HelpText>get log level</HelpText><LongHelp>Get the current console logging level</LongHelp><Description></Description></Command></Level></Level></Level>"
}

fn form_pairs(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

fn form_get<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    form.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    out.push((high << 4) | low);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn split_command(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn empty(status: StatusCode, content_type: &'static str, state: &ConsoleState) -> Response {
    xml_response(status, content_type, String::new(), false, state)
}

fn xml_response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<String>,
    close: bool,
    state: &ConsoleState,
) -> Response {
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type);
    if close {
        response = response.header(header::CONNECTION, "close");
    }
    if let Some(origin) = &state.origin {
        response = response.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    response
        .body(axum::body::Body::from(body.into()))
        .unwrap_or_else(|_| Response::new(axum::body::Body::from(String::new())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode as HttpStatus;

    fn console() -> Arc<ConsoleState> {
        state(
            "console-test".to_owned(),
            "console-test-pass".to_owned(),
            None,
            "Ferroan 0.1.0 (ferroan-region)".to_owned(),
            RuntimeLevel::new(level::INFO),
        )
        .expect("console")
    }

    async fn text(response: Response) -> (HttpStatus, String, Option<String>) {
        let status = response.status();
        let kind = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned(), kind)
    }

    #[tokio::test]
    async fn session_commands_match_the_client_forms() {
        let state = console();
        let (status, _, kind) =
            text(start_session(State(Arc::clone(&state)), "USER=no&PASS=no".to_owned()).await)
                .await;
        assert_eq!(status, HttpStatus::UNAUTHORIZED);
        assert_eq!(kind.as_deref(), Some("text/plain"));

        let (status, body, kind) = text(
            start_session(
                State(Arc::clone(&state)),
                "USER=console-test&PASS=console-test-pass".to_owned(),
            )
            .await,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        assert_eq!(kind.as_deref(), Some("text/xml"));
        assert_eq!(body.matches("<ConsoleSession>").count(), 1);
        assert_eq!(body.matches("<HelpTree>").count(), 1);
        assert!(body.contains("<Shared>False</Shared>"));
        assert!(body.contains("Name=\"show\""));
        assert!(body.contains("Name=\"version\""));
        let id = body
            .split("<SessionID>")
            .nth(1)
            .and_then(|rest| rest.split("</SessionID>").next())
            .expect("session");
        Uuid::parse_str(id).expect("uuid");

        let (status, body, _) = text(
            session_command(
                State(Arc::clone(&state)),
                format!("ID={id}&COMMAND=show \"version\""),
            )
            .await,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        assert!(body.contains("<Result>OK</Result>"));

        let (status, body, kind) =
            text(read_responses(State(Arc::clone(&state)), Path(id.to_owned())).await).await;
        assert_eq!(status, HttpStatus::OK);
        assert_eq!(kind.as_deref(), Some("application/xml"));
        assert!(body.contains("Ferroan 0.1.0 (ferroan-region)"));
        assert!(body.contains("+++Ferroan#"));

        let (status, body, _) = text(
            session_command(
                State(Arc::clone(&state)),
                format!("ID={id}&COMMAND=set \"log\" \"level\" \"debug\""),
            )
            .await,
        )
        .await;
        assert!(body.contains("<Result>OK</Result>"));
        assert_eq!(status, HttpStatus::OK);
        assert_eq!(state.level.get(), level::DEBUG);

        let (status, _, _) = text(close_session(State(state), format!("ID={id}")).await).await;
        assert_eq!(status, HttpStatus::OK);
    }

    #[tokio::test]
    async fn a_missing_session_is_404() {
        let state = console();
        let (status, body, _) = text(
            session_command(
                State(state),
                "ID=not-a-session&COMMAND=show \"version\"".to_owned(),
            )
            .await,
        )
        .await;
        assert_eq!(status, HttpStatus::NOT_FOUND);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn idle_sessions_expire_after_500_seconds_and_the_count_is_bounded() {
        let open = console_with("operator", "");
        let (status, _, _) =
            text(start_session(State(Arc::clone(&open)), "USER=operator&PASS=".to_owned()).await)
                .await;
        assert_eq!(status, HttpStatus::OK);
        let (status, _, _) = text(
            start_session(
                State(open),
                "USER=operator&PASS=not-the-password".to_owned(),
            )
            .await,
        )
        .await;
        assert_eq!(status, HttpStatus::UNAUTHORIZED);

        let state = console();
        state.set_now_ms(0);
        let id = session_id(&state).await;
        state.set_now_ms(100_000);
        assert_eq!(command(&state, &id).await, HttpStatus::OK);
        state.set_now_ms(IDLE_LIMIT_MS);
        assert_eq!(
            command(&state, &id).await,
            HttpStatus::OK,
            "500000 ms is not yet over the DoExpire threshold"
        );
        state.set_now_ms(IDLE_LIMIT_MS + 1);
        assert_eq!(command(&state, &id).await, HttpStatus::NOT_FOUND);
        assert_eq!(
            text(read_responses(State(Arc::clone(&state)), Path(id.clone())).await)
                .await
                .0,
            HttpStatus::NOT_FOUND
        );

        state.set_now_ms(0);
        let polled = session_id(&state).await;
        state.set_now_ms(400_000);
        assert_eq!(
            text(read_responses(State(Arc::clone(&state)), Path(polled.clone())).await)
                .await
                .0,
            HttpStatus::OK
        );
        state.set_now_ms(400_000 + IDLE_LIMIT_MS);
        assert_eq!(command(&state, &polled).await, HttpStatus::OK);
        state.set_now_ms(400_000 + IDLE_LIMIT_MS + 1);
        assert_eq!(command(&state, &polled).await, HttpStatus::NOT_FOUND);

        state.set_now_ms(0);
        let mut opened = Vec::new();
        for _ in 0..MAX_SESSIONS {
            opened.push(session_id(&state).await);
        }
        let hits_before = refuse_hits();
        let (status, body, _) = text(
            start_session(
                State(Arc::clone(&state)),
                "USER=console-test&PASS=console-test-pass".to_owned(),
            )
            .await,
        )
        .await;
        assert_eq!(status, HttpStatus::SERVICE_UNAVAILABLE);
        assert!(body.is_empty());
        assert!(refuse_hits() > hits_before);
        assert!(ferroan_diag::metrics().contains("console.sessions"));
        state.set_now_ms(IDLE_LIMIT_MS + 1);
        let _again = session_id(&state).await;
        assert_eq!(opened.len(), MAX_SESSIONS);
    }

    fn console_with(user: &str, pass: &str) -> Arc<ConsoleState> {
        state(
            user.to_owned(),
            pass.to_owned(),
            None,
            "Ferroan 0.1.0 (ferroan-region)".to_owned(),
            RuntimeLevel::new(level::INFO),
        )
        .expect("console")
    }

    async fn session_id(state: &Arc<ConsoleState>) -> String {
        let (status, body, _) = text(
            start_session(
                State(Arc::clone(state)),
                "USER=console-test&PASS=console-test-pass".to_owned(),
            )
            .await,
        )
        .await;
        assert_eq!(status, HttpStatus::OK);
        body.split("<SessionID>")
            .nth(1)
            .and_then(|rest| rest.split("</SessionID>").next())
            .expect("session")
            .to_owned()
    }

    async fn command(state: &Arc<ConsoleState>, id: &str) -> HttpStatus {
        let (status, _, _) = text(
            session_command(
                State(Arc::clone(state)),
                format!("ID={id}&COMMAND=show \"version\""),
            )
            .await,
        )
        .await;
        status
    }

    fn refuse_hits() -> u64 {
        let needle = "ferroan_limit_hits_total{limit=\"console.sessions\",action=\"refuse\"} ";
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(needle))
            .and_then(|rest| rest.trim().parse().ok())
            .unwrap_or(0)
    }
}
