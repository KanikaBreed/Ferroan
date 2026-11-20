// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Logging hooks for `ferroan-region` and `ferroan-robust`.
//!
//! Stdout and stderr are always on, so a container's logging driver can take
//! them as they are. `LOGDIR` adds a UTC daily file. `LogFormat = json` writes
//! one JSON object per line. `[Monitoring] OtlpEndpoint` exports OTLP/HTTP JSON.
//! `[Network] ConsoleUser` turns on Tranquillity's remote console. Nothing is
//! exported, and the console is not served, unless those settings are present.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod console;
mod layer;
mod level;
mod otlp;
mod redact;
mod timefmt;

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ferroan_config::Config;
use serde_json::json;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

pub use redact::{register_circuit_code, register_secret};

use layer::LogLayer;
use level::RuntimeLevel;
use otlp::{Exporter, ExporterTask, LogRecord, SpanRecord};

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("LogFormat must be json, plain or text")]
    Format,
    #[error("OtlpEndpoint must be an http URL without credentials")]
    Endpoint,
    #[error("ConsoleAllowedOrigin is not a valid header value")]
    Origin,
    #[error("log directory: {0}")]
    Directory(String),
    #[error("OTLP export: {0}")]
    Export(String),
    #[error("a log subscriber is already installed")]
    AlreadyInstalled,
}

/// Resolved log settings. Not `Debug`: the console password is private.
#[derive(Clone)]
pub struct Settings {
    server: String,
    region_name: Option<String>,
    region_id: Option<String>,
    json: bool,
    otlp: Option<String>,
    categories: Vec<(String, u8)>,
    default_level: u8,
    console_user: Option<String>,
    console_pass: String,
    console_origin: Option<String>,
    log_dir: Option<PathBuf>,
}

impl Settings {
    pub fn from_config(
        server: &str,
        config: &Config,
        region_name: Option<&str>,
        region_id: Option<Uuid>,
    ) -> Result<Self, LogError> {
        let otlp = config
            .get("Monitoring", "OtlpEndpoint")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(endpoint) = &otlp {
            otlp::Endpoint::parse(endpoint)?;
        }
        let (categories, default_level) = categories(config);
        let console_user = config
            .get("Network", "ConsoleUser")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let console_origin = config
            .get("Network", "ConsoleAllowedOrigin")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(origin) = &console_origin {
            axum::http::HeaderValue::from_str(origin).map_err(|_| LogError::Origin)?;
        }
        Ok(Self {
            server: server.to_owned(),
            region_name: region_name.map(str::to_owned),
            region_id: region_id.map(|id| id.to_string()),
            json: json_format(config)?,
            otlp,
            categories,
            default_level,
            console_pass: config
                .get("Network", "ConsolePass")
                .unwrap_or("")
                .to_owned(),
            console_origin,
            console_user,
            log_dir: None,
        })
    }

    /// A console with no files and no export, for `cargo xtask console-client`.
    pub fn remote_console(server: &str, user: &str, pass: &str) -> Self {
        Self {
            server: server.to_owned(),
            region_name: None,
            region_id: None,
            json: false,
            otlp: None,
            categories: Vec::new(),
            default_level: level::INFO,
            console_user: Some(user.to_owned()),
            console_pass: pass.to_owned(),
            console_origin: None,
            log_dir: None,
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }
}

/// Keeps the exporter alive until the server stops. Drop it on shutdown.
pub struct Guard {
    hub: Arc<Hub>,
    /// Held so drop flushes the exporter. Not read otherwise.
    exporter: Option<ExporterTask>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.exporter.take();
    }
}

impl Guard {
    pub fn console_router(&self) -> Option<axum::Router> {
        self.hub.console.clone().map(console::router)
    }
}

pub struct ConsoleListen {
    pub addr: std::net::SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for ConsoleListen {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn listen(router: axum::Router) -> Result<ConsoleListen, LogError> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| LogError::Export(error.to_string()))?;
    let addr = listener
        .local_addr()
        .map_err(|error| LogError::Export(error.to_string()))?;
    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
    });
    Ok(ConsoleListen {
        addr,
        shutdown: Some(shutdown),
        task: Some(task),
    })
}

pub fn install(settings: &Settings) -> Result<Guard, LogError> {
    ferroan_diag::set_redactor(redact::redact);
    ferroan_diag::set_context(
        &settings.server,
        settings.region_name.as_deref().unwrap_or(""),
    );
    ferroan_diag::install_panic_hook();
    let mut owned = settings.clone();
    if owned.log_dir.is_none()
        && let Some(dir) = std::env::var_os("LOGDIR").filter(|dir| !dir.is_empty())
    {
        owned.log_dir = Some(PathBuf::from(dir));
    }
    let (hub, exporter) = Hub::open(&owned)?;
    tracing_subscriber::registry()
        .with(LogLayer {
            hub: Arc::clone(&hub),
        })
        .try_init()
        .map_err(|_| LogError::AlreadyInstalled)?;
    Ok(Guard { hub, exporter })
}

pub(crate) struct Hub {
    server: String,
    region_name: Option<String>,
    region_id: Option<String>,
    json: bool,
    categories: Vec<(String, u8)>,
    level: RuntimeLevel,
    lines: Mutex<LineOut>,
    file: Mutex<Option<DailyFile>>,
    console: Option<Arc<console::ConsoleState>>,
    otlp: Option<Exporter>,
}

enum LineOut {
    Std,
    #[cfg(test)]
    Buffer(Arc<Mutex<Vec<String>>>),
}

struct DailyFile {
    directory: PathBuf,
    server: String,
    day: String,
    file: BufWriter<File>,
}

impl Hub {
    fn open(settings: &Settings) -> Result<(Arc<Self>, Option<ExporterTask>), LogError> {
        let file = match &settings.log_dir {
            Some(dir) => Some(DailyFile::open(dir, &settings.server)?),
            None => None,
        };
        let level = RuntimeLevel::new(settings.default_level);
        let console = match &settings.console_user {
            Some(user) => Some(console::state(
                user.clone(),
                settings.console_pass.clone(),
                settings.console_origin.clone(),
                format!(
                    "Ferroan {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    settings.server
                ),
                level.clone(),
            )?),
            None => None,
        };
        let (otlp, task) = match &settings.otlp {
            Some(endpoint) => {
                let (exporter, task) = Exporter::start(endpoint, &settings.server)?;
                (Some(exporter), Some(task))
            }
            None => (None, None),
        };
        let hub = Arc::new(Self {
            server: settings.server.clone(),
            region_name: settings.region_name.clone(),
            region_id: settings.region_id.clone(),
            json: settings.json,
            categories: settings.categories.clone(),
            level,
            lines: Mutex::new(LineOut::Std),
            file: Mutex::new(file),
            console,
            otlp,
        });
        Ok((hub, task))
    }

    fn allows(&self, target: &str, event: &tracing::Level) -> bool {
        let rank = self
            .category_rank(target)
            .unwrap_or_else(|| self.level.get());
        match level::ceiling(rank) {
            None => false,
            Some(max) => *event <= max,
        }
    }

    fn category_rank(&self, target: &str) -> Option<u8> {
        let target = target.to_ascii_lowercase();
        self.categories
            .iter()
            .find(|(prefix, _)| target.starts_with(prefix))
            .map(|(_, rank)| *rank)
    }

    fn emit(
        &self,
        event: &tracing::Level,
        target: &str,
        message: &str,
        agent_id: Option<&str>,
        trace_id: Option<&str>,
        span_id: Option<&str>,
    ) {
        let message = one_line(message);
        if matches!(*event, tracing::Level::ERROR | tracing::Level::WARN)
            && target != "ferroan::limit"
            && target != "ferroan::error"
        {
            ferroan_diag::push_log(level::label(event), target, &redact::redact(&message));
        }
        let now = timefmt::stamp(std::time::SystemTime::now());
        let plain = format!(
            "{stamp} {level:<5} {message}",
            stamp = timefmt::stamp_str(&now.plain),
            level = level::label(event),
        );
        let plain = redact::redact(&plain);
        let sink = if self.json {
            redact::redact(&json_line(self, &now, event, target, &message, agent_id))
        } else {
            plain.clone()
        };
        self.write_sink(event, &sink);
        self.write_file(timefmt::stamp_str(&now.day), &sink);
        if let Some(console) = &self.console {
            console.push_plain(level::label(event), &plain);
        }
        self.export_log(LogRecord {
            unix_nano: now.unix_nano,
            severity_number: level::severity(event),
            severity_text: level::label(event).to_owned(),
            body: redact::redact(&message),
            target: target.to_owned(),
            agent_id: agent_id.map(redact::redact),
            trace_id: trace_id.map(str::to_owned),
            span_id: span_id.map(str::to_owned),
        });
    }

    fn export_log(&self, record: LogRecord) {
        if let Some(otlp) = &self.otlp {
            otlp.log(record);
        }
    }

    fn export_span(&self, record: SpanRecord) {
        if let Some(otlp) = &self.otlp {
            otlp.span(record);
        }
    }

    fn write_sink(&self, event: &tracing::Level, line: &str) {
        let mut out = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match &mut *out {
            LineOut::Std => {
                let stream: &mut dyn Write =
                    if matches!(*event, tracing::Level::ERROR | tracing::Level::WARN) {
                        &mut std::io::stderr()
                    } else {
                        &mut std::io::stdout()
                    };
                let _ = writeln!(stream, "{line}");
                let _ = stream.flush();
            }
            #[cfg(test)]
            LineOut::Buffer(buffer) => {
                if let Ok(mut lines) = buffer.lock() {
                    lines.push(line.to_owned());
                }
            }
        }
    }

    fn write_file(&self, day: &str, line: &str) {
        let mut slot = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(daily) = slot.as_mut() else {
            return;
        };
        if daily.day != day {
            match DailyFile::open(&daily.directory, &daily.server) {
                Ok(reopened) => *daily = reopened,
                Err(error) => {
                    eprintln!("ferroan-log: {error}");
                    return;
                }
            }
        }
        if writeln!(daily.file, "{line}").is_err() || daily.file.flush().is_err() {
            eprintln!("ferroan-log: could not write the log file");
        }
    }
}

impl DailyFile {
    fn open(directory: &Path, server: &str) -> Result<Self, LogError> {
        std::fs::create_dir_all(directory)
            .map_err(|error| LogError::Directory(error.to_string()))?;
        let now = timefmt::stamp(std::time::SystemTime::now());
        let day = timefmt::stamp_str(&now.day).to_owned();
        let path = directory.join(format!("{server}{day}.log"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| LogError::Directory(error.to_string()))?;
        Ok(Self {
            directory: directory.to_path_buf(),
            server: server.to_owned(),
            day,
            file: BufWriter::new(file),
        })
    }
}

fn json_line(
    hub: &Hub,
    now: &timefmt::Stamp,
    event: &tracing::Level,
    target: &str,
    message: &str,
    agent_id: Option<&str>,
) -> String {
    let mut map = serde_json::Map::new();
    map.insert("time".to_owned(), json!(timefmt::stamp_str(&now.rfc3339)));
    map.insert("level".to_owned(), json!(level::label(event)));
    map.insert("target".to_owned(), json!(target));
    map.insert("server".to_owned(), json!(hub.server));
    if let Some(name) = &hub.region_name {
        map.insert("region_name".to_owned(), json!(name));
    }
    if let Some(id) = &hub.region_id {
        map.insert("region_id".to_owned(), json!(id));
    }
    map.insert("message".to_owned(), json!(message));
    if let Some(agent) = agent_id {
        map.insert("agent_id".to_owned(), json!(agent));
    }
    serde_json::Value::Object(map).to_string()
}

fn one_line(message: &str) -> String {
    if message.contains(['\n', '\r']) {
        message.replace(['\n', '\r'], " ")
    } else {
        message.to_owned()
    }
}

fn json_format(config: &Config) -> Result<bool, LogError> {
    let value = config
        .get("Monitoring", "LogFormat")
        .or_else(|| config.get("Startup", "LogFormat"))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match value.map(|value| value.to_ascii_lowercase()) {
        None => Ok(false),
        Some(value) if value == "json" => Ok(true),
        Some(value) if value == "plain" || value == "text" => Ok(false),
        Some(_) => Err(LogError::Format),
    }
}

fn categories(config: &Config) -> (Vec<(String, u8)>, u8) {
    let mut pairs = Vec::new();
    let mut default_level = level::INFO;
    for (key, value) in config.keys("Logging") {
        let Some(rank) = level::parse(value) else {
            continue;
        };
        let name = key.trim();
        if name.eq_ignore_ascii_case("Default") {
            default_level = rank;
            continue;
        }
        let name = name.trim_end_matches('*');
        if name.is_empty() {
            continue;
        }
        pairs.retain(|(existing, _): &(String, u8)| !existing.eq_ignore_ascii_case(name));
        pairs.push((name.to_ascii_lowercase(), rank));
    }
    pairs.sort_by(|left, right| right.0.len().cmp(&left.0.len()).then(left.0.cmp(&right.0)));
    (pairs, default_level)
}

#[cfg(test)]
fn capture(settings: &Settings, body: impl FnOnce()) -> Vec<String> {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let (mut hub, _task) = Hub::open(settings).expect("hub");
    Arc::get_mut(&mut hub).expect("hub").lines = Mutex::new(LineOut::Buffer(Arc::clone(&buffer)));
    let subscriber = tracing_subscriber::registry().with(LogLayer { hub });
    tracing::subscriber::with_default(subscriber, body);
    buffer.lock().expect("buffer").clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(json: bool) -> Settings {
        let mut config = Config::default();
        if json {
            config.set("Monitoring", "LogFormat", "json");
        }
        config.set("Logging", "Default", "debug");
        config.set("Logging", "ferroan_region", "error");
        config.set("Logging", "noisy*", "off");
        let mut settings = Settings::from_config(
            "ferroan-region",
            &config,
            Some("Test Region"),
            Some(Uuid::from_u128(7)),
        )
        .expect("settings");
        settings.log_dir = None;
        settings
    }

    #[test]
    fn a_json_line_has_the_required_fields_and_drops_a_newline() {
        let lines = capture(&sample(true), || {
            tracing::info!(
                agent_id = "00000000-0000-0000-0000-000000000007",
                "login accepted first_name=Test last_name=User\nmore"
            );
        });
        assert_eq!(lines.len(), 1);
        let value: serde_json::Value = serde_json::from_str(&lines[0]).expect("json");
        assert!(value["time"].as_str().unwrap().ends_with('Z'));
        assert_eq!(value["level"], "INFO");
        assert!(value["target"].as_str().unwrap().contains("ferroan_log"));
        assert_eq!(value["server"], "ferroan-region");
        assert_eq!(value["region_name"], "Test Region");
        assert_eq!(value["region_id"], Uuid::from_u128(7).to_string());
        assert_eq!(value["agent_id"], "00000000-0000-0000-0000-000000000007");
        let message = value["message"].as_str().unwrap();
        assert!(message.contains("login accepted"));
        assert!(message.contains("first_name=Test"));
        assert!(message.contains("last_name=User"));
        assert!(!message.contains('\n'));
    }

    #[test]
    fn plain_lines_pad_the_level_and_a_category_can_drop_them() {
        let lines = capture(&sample(false), || {
            tracing::info!(target: "ferroan_region::simulation", "hidden");
            tracing::error!(target: "ferroan_region::simulation", "visible");
            tracing::debug!(target: "other", "debug-line");
            tracing::info!(target: "noisy::child", "dropped");
        });
        assert!(lines.iter().all(|line| !line.contains("hidden")));
        assert!(lines.iter().all(|line| !line.contains("dropped")));
        assert!(lines.iter().any(|line| line.contains("ERROR visible")));
        assert!(lines.iter().any(|line| line.contains("DEBUG debug-line")));
    }

    #[test]
    fn logdir_writes_a_dated_file_and_redacts_a_secret() {
        register_secret("session-secret-value");
        let dir = std::env::temp_dir().join(format!("ferroan-log-{}", Uuid::new_v4()));
        let mut settings = sample(false);
        settings.log_dir = Some(dir.clone());
        let _ = capture(&settings, || {
            tracing::info!("saw session-secret-value");
        });
        let stamped = timefmt::stamp(std::time::SystemTime::now());
        let day = timefmt::stamp_str(&stamped.day);
        let path = dir.join(format!("ferroan-region{day}.log"));
        let text = std::fs::read_to_string(&path).expect("file");
        assert!(text.contains("[redacted]"));
        assert!(!text.contains("session-secret-value"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_log_format_loses_to_monitoring_and_other_values_fail() {
        let mut config = Config::default();
        config.set("Startup", "LogFormat", "json");
        config.set("Monitoring", "LogFormat", "plain");
        let settings = Settings::from_config("ferroan-robust", &config, None, None).expect("plain");
        assert!(!settings.json);
        config.set("Monitoring", "LogFormat", "yaml");
        assert!(Settings::from_config("ferroan-robust", &config, None, None).is_err());
        config.set(
            "Monitoring",
            "OtlpEndpoint",
            "http://user:secret@127.0.0.1:4318",
        );
        config.set("Monitoring", "LogFormat", "json");
        assert!(matches!(
            Settings::from_config("ferroan-robust", &config, None, None),
            Err(LogError::Endpoint)
        ));
    }

    #[test]
    fn a_login_span_exports_when_an_endpoint_is_set() {
        // The collector is not required: the queue accepts the span and the
        // copy of it that Loki can store. Export failure is reported on stderr.
        let mut config = Config::default();
        config.set("Monitoring", "OtlpEndpoint", "http://127.0.0.1:1");
        let settings = Settings::from_config("ferroan-region", &config, None, None).expect("otlp");
        let lines = capture(&settings, || {
            let span =
                tracing::info_span!("login", agent_id = "00000000-0000-0000-0000-000000000009");
            let _entered = span.enter();
            tracing::info!("login accepted");
        });
        assert!(lines.iter().any(|line| line.contains("login accepted")));
    }
}
