// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/login.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// The password form follows Tranquillity
//   Source/OpenSim.Services.LLLoginService/LLLoginService.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
// which strips a leading `$1$` and does not truncate the password.

//! XML-RPC `login_to_simulator`.

use std::fmt;
use std::net::Ipv4Addr;
use std::time::Duration;

use ferroan_llsd::{Llsd, Map};
use md5::{Digest, Md5};
use uuid::Uuid;

use crate::body::read_limited;
use crate::xmlrpc::{self, XmlRpcError};

/// Sent in the `channel` member. Tranquillity stores it on the session.
pub const CHANNEL: &str = "Ferroan Bot";
/// Sent in the `version` member.
pub const LOGIN_VERSION: &str = "0.1.0";

/// Login replies larger than this are refused. A new test user's inventory
/// skeleton is far smaller. Raised later if a scenario needs a bigger reply.
pub const MAX_LOGIN_BODY: usize = 8 * 1024 * 1024;

/// Members the bot asks the login reply to include. Tranquillity's handler
/// at the pinned revision fills inventory and textures without reading this
/// list; it is still sent so the request has the same shape as a viewer.
pub const LOGIN_OPTIONS: &[&str] = &[
    "inventory-root",
    "inventory-skeleton",
    "inventory-lib-root",
    "inventory-lib-owner",
    "inventory-skel-lib",
    "gestures",
    "display_names",
    "login-flags",
    "global-textures",
];

const MAX_NAME_CHARS: usize = 64;
const MAX_PASSWORD_CHARS: usize = 1024;
const LOGIN_ATTEMPTS: usize = 5;

#[derive(Clone, PartialEq, Eq)]
pub struct LoginRequest {
    pub login_uri: String,
    /// `First Last`, or one name (last name `Resident`).
    pub username: String,
    pub password: String,
    pub start: String,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("login_uri", &self.login_uri)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("start", &self.start)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct LoginResponse {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub secure_session_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub sim_ip: Ipv4Addr,
    pub sim_port: u16,
    pub circuit_code: u32,
    pub region_x: u32,
    pub region_y: u32,
    pub region_size_x: u32,
    pub region_size_y: u32,
    pub seed_capability: String,
    pub look_at: [f32; 3],
    pub message: String,
    pub inventory_root: Uuid,
    /// The reply, kept for scenarios that need a field this struct does not.
    pub raw: Llsd,
}

impl fmt::Debug for LoginResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginResponse")
            .field("agent_id", &self.agent_id)
            .field("sim_ip", &self.sim_ip)
            .field("sim_port", &self.sim_port)
            .field("seed_capability", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("login name or password is empty or too long")]
    Credentials,
    #[error("http: {0}")]
    Http(String),
    #[error("login server status {0}")]
    Status(u16),
    #[error("login reply is {got} bytes, over the {limit} byte limit")]
    TooLarge { got: usize, limit: usize },
    #[error("fault {code}: {message}")]
    Fault { code: i32, message: String },
    #[error("login refused ({reason}): {message}")]
    Refused { reason: String, message: String },
    #[error("malformed login reply ({0})")]
    Malformed(&'static str),
    #[error("the login reply was not XML-RPC")]
    NotXmlRpc,
    #[error("too many login redirects")]
    TooManyRedirects,
}

impl From<XmlRpcError> for LoginError {
    fn from(error: XmlRpcError) -> Self {
        match error {
            XmlRpcError::Fault { code, message } => LoginError::Fault { code, message },
            XmlRpcError::Xml(_) | XmlRpcError::Malformed => LoginError::NotXmlRpc,
        }
    }
}

pub fn http_client() -> Result<reqwest::Client, LoginError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("Ferroan Bot ", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| LoginError::Http(error.to_string()))
}

/// `$1$` plus the md5 hex of the password. Linden Lab grids truncate to
/// 16 characters, as the viewer does. A value that is already `$1$` and 35
/// characters is sent unchanged.
pub fn hash_password(password: &str, truncate16: bool) -> String {
    if password.len() == 35 && password.starts_with("$1$") {
        return password.to_owned();
    }
    let taken: String = if truncate16 {
        password.chars().take(16).collect()
    } else {
        password.to_owned()
    };
    format!("$1${:x}", Md5::digest(taken.as_bytes()))
}

pub fn is_linden_grid(uri: &str) -> bool {
    uri.contains("lindenlab.com")
}

/// First token, then the next token, or `Resident` when there is one token.
/// Separators are space and `.`.
pub fn split_username(username: &str) -> (String, String) {
    let mut parts = username
        .trim()
        .split([' ', '.'])
        .filter(|part| !part.is_empty());
    let first = parts.next().unwrap_or("").to_owned();
    let last = parts.next().unwrap_or("Resident").to_owned();
    (first, last)
}

fn md5_hex(bytes: &[u8]) -> String {
    format!("{:x}", Md5::digest(bytes))
}

/// The login struct. `mac` and `id0` are fixed labels for this program,
/// not a hash of the machine it runs on.
pub fn build_params(request: &LoginRequest) -> Result<Llsd, LoginError> {
    let (first, last) = split_username(&request.username);
    if first.is_empty()
        || first.chars().count() > MAX_NAME_CHARS
        || last.chars().count() > MAX_NAME_CHARS
        || request.password.is_empty()
        || request.password.chars().count() > MAX_PASSWORD_CHARS
    {
        return Err(LoginError::Credentials);
    }
    let mut params = Map::new();
    params.insert("first".into(), first.into());
    params.insert("last".into(), last.into());
    params.insert(
        "passwd".into(),
        hash_password(&request.password, is_linden_grid(&request.login_uri)).into(),
    );
    params.insert("start".into(), request.start.clone().into());
    params.insert("channel".into(), CHANNEL.into());
    params.insert("version".into(), LOGIN_VERSION.into());
    params.insert(
        "platform".into(),
        if cfg!(windows) {
            "win"
        } else if cfg!(target_os = "macos") {
            "mac"
        } else {
            "lnx"
        }
        .into(),
    );
    params.insert("platform_string".into(), std::env::consts::OS.into());
    params.insert("address_size".into(), 64.into());
    params.insert("mac".into(), md5_hex(b"ferroan-bot-mac").into());
    params.insert("id0".into(), md5_hex(b"ferroan-bot-id0").into());
    params.insert("agree_to_tos".into(), true.into());
    params.insert("read_critical".into(), true.into());
    params.insert("extended_errors".into(), true.into());
    let options: Vec<Llsd> = LOGIN_OPTIONS.iter().map(|name| Llsd::from(*name)).collect();
    params.insert("options".into(), Llsd::Array(options));
    Ok(Llsd::Map(params))
}

/// `[r0.1, r0.9, r0]` as three floats.
pub fn parse_ll_vector(text: &str) -> Option<[f32; 3]> {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | 'r' | ' '))
        .collect();
    let mut parts = cleaned.split(',').map(|part| part.parse::<f32>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

fn positive_u32(value: &Llsd, fallback: u32) -> u32 {
    let number = value.as_u32();
    if number == 0 { fallback } else { number }
}

pub fn parse_success(value: Llsd) -> Result<LoginResponse, LoginError> {
    let agent_id = value["agent_id"].as_uuid();
    if agent_id.is_nil() {
        return Err(LoginError::Malformed("agent_id"));
    }
    let sim_ip: Ipv4Addr = value["sim_ip"]
        .as_str()
        .parse()
        .map_err(|_| LoginError::Malformed("sim_ip"))?;
    let sim_port =
        u16::try_from(value["sim_port"].as_i32()).map_err(|_| LoginError::Malformed("sim_port"))?;
    Ok(LoginResponse {
        agent_id,
        session_id: value["session_id"].as_uuid(),
        secure_session_id: value["secure_session_id"].as_uuid(),
        first_name: value["first_name"].as_str().trim_matches('"').to_owned(),
        last_name: value["last_name"].as_str().trim_matches('"').to_owned(),
        sim_ip,
        sim_port,
        circuit_code: value["circuit_code"].as_u32(),
        region_x: value["region_x"].as_u32(),
        region_y: value["region_y"].as_u32(),
        region_size_x: positive_u32(&value["region_size_x"], 256),
        region_size_y: positive_u32(&value["region_size_y"], 256),
        seed_capability: value["seed_capability"].as_str().to_owned(),
        look_at: parse_ll_vector(value["look_at"].as_str()).unwrap_or([1.0, 0.0, 0.0]),
        message: value["message"].as_str().to_owned(),
        inventory_root: value["inventory-root"][0]["folder_id"].as_uuid(),
        raw: value,
    })
}

fn refusal(value: &Llsd) -> LoginError {
    LoginError::Refused {
        reason: {
            let reason = value["reason"].as_str();
            if reason.is_empty() {
                "unknown".to_owned()
            } else {
                reason.to_owned()
            }
        },
        message: value["message"].to_string_value(),
    }
}

fn same_authority(from: &str, to: &str) -> bool {
    let (Ok(from_url), Ok(to_url)) = (reqwest::Url::parse(from), reqwest::Url::parse(to)) else {
        return false;
    };
    from_url.host_str() == to_url.host_str()
        && from_url.port_or_known_default() == to_url.port_or_known_default()
}

/// POST `login_to_simulator`. An `indeterminate` reply with `next_url` is
/// followed at most five times, and only when the next URL is the same host
/// and port. Any other redirect stops the login.
pub async fn login(
    http: &reqwest::Client,
    request: &LoginRequest,
) -> Result<LoginResponse, LoginError> {
    let params = build_params(request)?;
    let mut uri = request.login_uri.clone();
    for _attempt in 0..LOGIN_ATTEMPTS {
        let body = xmlrpc::build_call("login_to_simulator", &params);
        let response = http
            .post(&uri)
            .header(reqwest::header::CONTENT_TYPE, "text/xml")
            .body(body)
            .send()
            .await
            .map_err(|error| LoginError::Http(error.without_url().to_string()))?;
        let status = response.status();
        let at = response.url().clone();
        if !status.is_success() {
            return Err(LoginError::Status(status.as_u16()));
        }
        let bytes = match read_limited(response, MAX_LOGIN_BODY).await {
            Ok(bytes) => bytes,
            Err(crate::body::BodyError::Transport) => {
                return Err(LoginError::Http("reading the login reply failed".into()));
            }
            Err(crate::body::BodyError::TooLarge(got)) => {
                return Err(LoginError::TooLarge {
                    got,
                    limit: MAX_LOGIN_BODY,
                });
            }
        };
        let value = xmlrpc::parse_response(&bytes)?;
        match value["login"].to_string_value().as_str() {
            "true" => return parse_success(value),
            "indeterminate" => {
                let next = value["next_url"].as_str();
                if next.is_empty() {
                    return Err(LoginError::Malformed("indeterminate without next_url"));
                }
                let next = at
                    .join(next)
                    .map_err(|_| LoginError::Malformed("next_url"))?;
                if !same_authority(at.as_str(), next.as_str()) {
                    return Err(LoginError::Refused {
                        reason: "redirect".to_owned(),
                        message: "login next_url is a different host".to_owned(),
                    });
                }
                uri = next.into();
            }
            _ => return Err(refusal(&value)),
        }
    }
    Err(LoginError::TooManyRedirects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_matches_the_known_md5() {
        // md5("test") = 098f6bcd4621d373cade4e832627b4f6
        assert_eq!(
            hash_password("test", false),
            "$1$098f6bcd4621d373cade4e832627b4f6"
        );
        let already = "$1$098f6bcd4621d373cade4e832627b4f6";
        assert_eq!(hash_password(already, true), already);
        assert_eq!(
            hash_password("0123456789abcdefEXTRA", true),
            hash_password("0123456789abcdef", false)
        );
        assert_ne!(
            hash_password("0123456789abcdefEXTRA", false),
            hash_password("0123456789abcdef", false)
        );
    }

    #[test]
    fn username_splits_on_space_and_dot() {
        assert_eq!(split_username("Test User"), ("Test".into(), "User".into()));
        assert_eq!(split_username("Test"), ("Test".into(), "Resident".into()));
        assert_eq!(split_username("Test.User"), ("Test".into(), "User".into()));
    }

    #[test]
    fn login_body_hashes_the_password_and_names_the_method() {
        let request = LoginRequest {
            login_uri: "http://127.0.0.1:9100/".into(),
            username: "Test User".into(),
            password: "hunter2-not-in-the-xml".into(),
            start: "last".into(),
        };
        let params = build_params(&request).unwrap();
        let body = xmlrpc::build_call("login_to_simulator", &params);
        assert!(body.contains("<methodName>login_to_simulator</methodName>"));
        assert!(body.contains("$1$"));
        assert!(!body.contains("hunter2-not-in-the-xml"));
        assert!(body.contains(">Test</string>"));
        assert!(body.contains(">User</string>"));
        assert!(body.contains("inventory-root"));
        assert_eq!(
            params["passwd"].as_str(),
            hash_password("hunter2-not-in-the-xml", false)
        );
    }

    #[test]
    fn linden_uri_truncates_the_password() {
        let request = LoginRequest {
            login_uri: "https://login.agni.lindenlab.com/cgi-bin/login.cgi".into(),
            username: "Test User".into(),
            password: "0123456789abcdefEXTRA".into(),
            start: "last".into(),
        };
        let params = build_params(&request).unwrap();
        assert_eq!(
            params["passwd"].as_str(),
            hash_password("0123456789abcdef", false)
        );
    }

    #[test]
    fn success_reply_parses_the_circuit_fields() {
        let response = br#"<?xml version="1.0"?><methodResponse><params><param><value><struct>
            <member><name>login</name><value>true</value></member>
            <member><name>agent_id</name><value><string>11111111-1111-1111-1111-111111111111</string></value></member>
            <member><name>session_id</name><value><string>22222222-2222-2222-2222-222222222222</string></value></member>
            <member><name>secure_session_id</name><value><string>33333333-3333-3333-3333-333333333333</string></value></member>
            <member><name>first_name</name><value><string>Test</string></value></member>
            <member><name>last_name</name><value><string>User</string></value></member>
            <member><name>sim_ip</name><value><string>127.0.0.1</string></value></member>
            <member><name>sim_port</name><value><int>9100</int></value></member>
            <member><name>circuit_code</name><value><int>42</int></value></member>
            <member><name>region_x</name><value><int>1000</int></value></member>
            <member><name>region_y</name><value><int>1000</int></value></member>
            <member><name>seed_capability</name><value><string>http://127.0.0.1:9100/CAPS/seed</string></value></member>
            <member><name>look_at</name><value><string>[r0,r1,r0]</string></value></member>
            <member><name>message</name><value><string>Welcome</string></value></member>
            <member><name>inventory-root</name><value><array><data><value><struct><member><name>folder_id</name><value><string>44444444-4444-4444-4444-444444444444</string></value></member></struct></value></data></array></value></member>
            </struct></value></param></params></methodResponse>"#;
        let parsed = parse_success(xmlrpc::parse_response(response).unwrap()).unwrap();
        assert_eq!(
            parsed.agent_id,
            Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
        );
        assert_eq!(parsed.sim_ip, Ipv4Addr::LOCALHOST);
        assert_eq!(parsed.sim_port, 9100);
        assert_eq!(parsed.circuit_code, 42);
        assert_eq!(parsed.region_x, 1000);
        assert_eq!(parsed.region_size_x, 256);
        assert_eq!(parsed.look_at, [0.0, 1.0, 0.0]);
        assert_eq!(
            parsed.inventory_root,
            Uuid::parse_str("44444444-4444-4444-4444-444444444444").unwrap()
        );
        assert_eq!(parsed.seed_capability, "http://127.0.0.1:9100/CAPS/seed");
    }

    #[test]
    fn false_login_is_a_refusal() {
        let response = b"<methodResponse><params><param><value><struct><member><name>login</name><value>false</value></member><member><name>reason</name><value><string>key</string></value></member><member><name>message</name><value><string>Could not authenticate</string></value></member></struct></value></param></params></methodResponse>";
        let value = xmlrpc::parse_response(response).unwrap();
        assert!(matches!(
            refusal(&value),
            LoginError::Refused { reason, .. } if reason == "key"
        ));
    }

    #[tokio::test]
    async fn login_posts_text_xml_and_reads_the_reply() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let reply = b"<?xml version=\"1.0\"?><methodResponse><params><param><value><struct><member><name>login</name><value>true</value></member><member><name>agent_id</name><value><string>11111111-1111-1111-1111-111111111111</string></value></member><member><name>session_id</name><value><string>22222222-2222-2222-2222-222222222222</string></value></member><member><name>sim_ip</name><value><string>127.0.0.1</string></value></member><member><name>sim_port</name><value><int>9100</int></value></member><member><name>circuit_code</name><value><int>7</int></value></member><member><name>seed_capability</name><value><string>http://127.0.0.1/seed</string></value></member></struct></value></param></params></methodResponse>";
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let request = crate::body::read_http_request(&mut sock).await;
            let header = String::from_utf8_lossy(&request);
            assert!(
                header
                    .to_ascii_lowercase()
                    .contains("content-type: text/xml")
            );
            assert!(header.contains("$1$098f6bcd4621d373cade4e832627b4f6"));
            assert!(!header.contains("<string>test</string>"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            tokio::io::AsyncWriteExt::write_all(&mut sock, response.as_bytes())
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut sock, reply)
                .await
                .unwrap();
        });
        let http = http_client().unwrap();
        let parsed = login(
            &http,
            &LoginRequest {
                login_uri: format!("http://127.0.0.1:{port}/"),
                username: "Test User".into(),
                password: "test".into(),
                start: "last".into(),
            },
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(parsed.circuit_code, 7);
        assert_eq!(parsed.sim_port, 9100);
        let printed = format!("{parsed:?}");
        assert!(printed.contains("agent_id"));
        assert!(printed.contains("<redacted>"));
        assert!(!printed.contains("http://"));
    }
}
