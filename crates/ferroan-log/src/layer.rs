// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Write as _;

use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::Hub;
use crate::otlp::{LogRecord, SpanRecord};

pub(crate) struct LogLayer {
    pub(crate) hub: std::sync::Arc<Hub>,
}

#[derive(Clone)]
pub(crate) struct SpanMark {
    pub(crate) trace_id: String,
    pub(crate) span_id: String,
    pub(crate) start_nano: u64,
    pub(crate) agent_id: Option<String>,
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: Context<'_, S>,
    ) {
        if attrs.metadata().name() != "login" {
            return;
        }
        if !self
            .hub
            .allows(attrs.metadata().target(), attrs.metadata().level())
        {
            return;
        }
        let mut fields = EventFields::default();
        attrs.record(&mut fields);
        let now = crate::timefmt::stamp(std::time::SystemTime::now());
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanMark {
                trace_id: new_id(16),
                span_id: new_id(8),
                start_nano: now.unix_nano,
                agent_id: fields.agent_id,
            });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if !self
            .hub
            .allows(event.metadata().target(), event.metadata().level())
        {
            return;
        }
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let mut trace_id = None;
        let mut span_id = None;
        if fields.agent_id.is_none() {
            if let Some(span) = ctx.lookup_current()
                && let Some(mark) = span.extensions().get::<SpanMark>()
            {
                fields.agent_id.clone_from(&mark.agent_id);
                trace_id = Some(mark.trace_id.clone());
                span_id = Some(mark.span_id.clone());
            }
        } else if let Some(span) = ctx.lookup_current()
            && let Some(mark) = span.extensions().get::<SpanMark>()
        {
            trace_id = Some(mark.trace_id.clone());
            span_id = Some(mark.span_id.clone());
        }
        self.hub.emit(
            event.metadata().level(),
            event.metadata().target(),
            &fields.message,
            fields.agent_id.as_deref(),
            trace_id.as_deref(),
            span_id.as_deref(),
        );
    }

    fn on_close(&self, id: tracing::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else {
            return;
        };
        let Some(mark) = span.extensions().get::<SpanMark>().cloned() else {
            return;
        };
        let end = crate::timefmt::stamp(std::time::SystemTime::now()).unix_nano;
        self.hub.export_span(SpanRecord {
            name: "login".to_owned(),
            trace_id: mark.trace_id.clone(),
            span_id: mark.span_id.clone(),
            start_nano: mark.start_nano,
            end_nano: end,
            agent_id: mark.agent_id.clone(),
        });
        let body = format!("trace span=login trace_id={}", mark.trace_id);
        self.hub.export_log(LogRecord {
            unix_nano: end,
            severity_number: 9,
            severity_text: "INFO".to_owned(),
            body: crate::redact::redact(&body),
            target: "ferroan_log".to_owned(),
            agent_id: mark.agent_id,
            trace_id: Some(mark.trace_id),
            span_id: Some(mark.span_id),
        });
    }
}

#[derive(Default)]
struct EventFields {
    message: String,
    agent_id: Option<String>,
}

impl Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut text = String::new();
        if write!(&mut text, "{value:?}").is_ok() {
            self.store(field.name(), &text);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.store(field.name(), value);
    }
}

impl EventFields {
    fn store(&mut self, name: &str, value: &str) {
        if name == "message" {
            self.message = value.to_owned();
        } else if name == "agent_id" {
            self.agent_id = Some(value.to_owned());
        }
    }
}

fn new_id(bytes: usize) -> String {
    let raw = uuid::Uuid::new_v4();
    let hex = raw.simple().to_string();
    hex.chars().take(bytes * 2).collect()
}
