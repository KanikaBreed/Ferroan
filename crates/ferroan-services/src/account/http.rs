// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `POST /accounts`.
//!
//! `UserAccountServerPostHandler` answers 200 `application/xml`. Failure is
//! the fixed ASCII document with a space before `=`. Success is
//! `ServerUtils.BuildXmlResponse`. A body over [`ferroan_http::MAX_QUERY_BYTES`]
//! is 413. Tranquillity does not cap the body. `X-SecondLife-Shard` is 403
//! `text/plain` with an empty body unless `AllowllHTTPRequestIn` is true.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroan_http::{Field, FieldMap, MAX_QUERY_BYTES, build_xml_response, parse_query_string};
use uuid::Uuid;

use super::model::Account;
use super::service::{AccountService, NewAccount};

const FAILURE: &[u8] =
    b"<?xml version =\"1.0\"?><ServerResponse><result>Failure</result></ServerResponse>";

#[derive(Clone)]
pub struct AccountHttp {
    inner: Arc<Inner>,
}

struct Inner {
    service: AccountService,
    allow_create_user: bool,
    allow_set_account: bool,
    reject_shard_header: bool,
}

impl AccountHttp {
    pub fn new(
        service: AccountService,
        allow_create_user: bool,
        allow_set_account: bool,
        reject_shard_header: bool,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                service,
                allow_create_user,
                allow_set_account,
                reject_shard_header,
            }),
        }
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .route("/accounts", post(post_accounts))
            .with_state(self.inner)
    }
}

async fn post_accounts(
    State(inner): State<Arc<Inner>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if inner.reject_shard_header && headers.get("X-SecondLife-Shard").is_some() {
        return plain(StatusCode::FORBIDDEN, Vec::new());
    }
    let bytes = match axum::body::to_bytes(body, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE, Vec::new()),
    };
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim();
    let fields = match parse_query_string(trimmed) {
        Ok(fields) => fields,
        Err(_) => return failure(),
    };
    match dispatch(&inner, &fields).await {
        Ok(bytes) => xml(bytes),
        Err(Outcome::Failure) => failure(),
    }
}

enum Outcome {
    Failure,
}

async fn dispatch(inner: &Inner, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let method = text_field(fields, "METHOD").ok_or(Outcome::Failure)?;
    match method.as_str() {
        "createuser" => {
            if inner.allow_create_user {
                create_user(&inner.service, fields).await
            } else {
                Err(Outcome::Failure)
            }
        }
        "getaccount" => get_account(&inner.service, fields).await,
        "getaccounts" => get_accounts(&inner.service, fields).await,
        "getmultiaccounts" => get_multi(&inner.service, fields).await,
        "setdisplayname" => set_display_name(&inner.service, fields).await,
        "setaccount" => {
            if inner.allow_set_account {
                set_account(&inner.service, fields).await
            } else {
                Err(Outcome::Failure)
            }
        }
        _ => Err(Outcome::Failure),
    }
}

async fn get_account(service: &AccountService, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let scope = match optional_uuid(fields, "ScopeID") {
        Ok(scope) => scope,
        Err(OptionalUuid::Bad) => return Ok(result_null()),
        Err(OptionalUuid::Absent) => Uuid::nil(),
    };
    let account = if fields.contains_key("UserID") {
        lookup_id(service, scope, fields, "UserID").await?
    } else if fields.contains_key("PrincipalID") {
        lookup_id(service, scope, fields, "PrincipalID").await?
    } else if let Some(email) = text_field(fields, "Email") {
        service
            .get_by_email(scope, &email)
            .await
            .map_err(|_| Outcome::Failure)?
    } else if let (Some(first), Some(last)) = (
        text_field(fields, "FirstName"),
        text_field(fields, "LastName"),
    ) {
        service
            .get_by_name(scope, &first, &last)
            .await
            .map_err(|_| Outcome::Failure)?
    } else {
        None
    };
    Ok(match account {
        Some(account) => result_account(&account)?,
        None => result_null(),
    })
}

async fn lookup_id(
    service: &AccountService,
    scope: Uuid,
    fields: &FieldMap,
    key: &str,
) -> Result<Option<Account>, Outcome> {
    let Some(text) = text_field(fields, key) else {
        return Ok(None);
    };
    let Ok(id) = Uuid::parse_str(&text) else {
        return Ok(None);
    };
    service
        .get_by_principal(scope, id)
        .await
        .map_err(|_| Outcome::Failure)
}

async fn get_accounts(service: &AccountService, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let Some(query) = text_field(fields, "query") else {
        return Err(Outcome::Failure);
    };
    let scope = required_scope(fields)?;
    let accounts = if query.trim().is_empty() {
        Vec::new()
    } else {
        service
            .search(scope, &query)
            .await
            .map_err(|_| Outcome::Failure)?
    };
    account_list(&accounts)
}

async fn get_multi(service: &AccountService, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let scope = required_scope(fields)?;
    let Some(Field::List(ids)) = fields.get("IDS") else {
        return Err(Outcome::Failure);
    };
    let mut parsed = Vec::new();
    for id in ids {
        if let Ok(id) = Uuid::parse_str(id) {
            parsed.push(id);
        }
    }
    let accounts = if parsed.is_empty() {
        Vec::new()
    } else {
        service
            .get_many(scope, &parsed)
            .await
            .map_err(|_| Outcome::Failure)?
    };
    account_list(&accounts)
}

async fn set_display_name(service: &AccountService, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let id = match optional_uuid(fields, "PrincipalID") {
        Ok(id) => id,
        Err(_) => return Err(Outcome::Failure),
    };
    if service
        .get_by_principal(Uuid::nil(), id)
        .await
        .map_err(|_| Outcome::Failure)?
        .is_none()
    {
        return Err(Outcome::Failure);
    }
    let Some(name) = text_field(fields, "DisplayName") else {
        return Err(Outcome::Failure);
    };
    if !service
        .set_display_name(id, &name)
        .await
        .map_err(|_| Outcome::Failure)?
    {
        return Err(Outcome::Failure);
    }
    let mut result = FieldMap::new();
    result.insert("result".to_string(), Field::Str("success".to_string()));
    xml_of(&result)
}

async fn set_account(service: &AccountService, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let id = match optional_uuid(fields, "PrincipalID") {
        Ok(id) if !id.is_nil() => id,
        _ => return Err(Outcome::Failure),
    };
    let scope = required_scope(fields)?;
    let Some(mut account) = service
        .get_by_principal(scope, id)
        .await
        .map_err(|_| Outcome::Failure)?
    else {
        return Err(Outcome::Failure);
    };
    if let Some(value) = text_field(fields, "FirstName") {
        account.first_name = value;
    }
    if let Some(value) = text_field(fields, "LastName") {
        account.last_name = value;
    }
    if let Some(value) = text_field(fields, "Email") {
        account.email = value;
    }
    if let Some(value) = text_field(fields, "Created")
        && let Ok(created) = value.parse::<i32>()
    {
        account.created = created;
    }
    if let Some(value) = text_field(fields, "UserLevel")
        && let Ok(level) = value.parse::<i32>()
    {
        account.user_level = level;
    }
    if let Some(value) = text_field(fields, "UserFlags")
        && let Ok(flags) = value.parse::<i32>()
    {
        account.user_flags = flags;
    }
    if let Some(value) = text_field(fields, "UserTitle") {
        account.user_title = Some(value);
    }
    if !service
        .store_user(&account)
        .await
        .map_err(|_| Outcome::Failure)?
    {
        return Err(Outcome::Failure);
    }
    result_account(&account)
}

async fn create_user(service: &AccountService, fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    let Some(first_name) = text_field(fields, "FirstName") else {
        return Err(Outcome::Failure);
    };
    let Some(last_name) = text_field(fields, "LastName") else {
        return Err(Outcome::Failure);
    };
    let Some(password) = text_field(fields, "Password") else {
        return Err(Outcome::Failure);
    };
    let scope = required_scope(fields)?;
    let principal_id = match optional_uuid(fields, "PrincipalID") {
        Ok(id) => id,
        Err(OptionalUuid::Absent) => Uuid::new_v4(),
        Err(OptionalUuid::Bad) => return Err(Outcome::Failure),
    };
    let email = text_field(fields, "Email").unwrap_or_default();
    let model = text_field(fields, "Model").unwrap_or_default();
    let created = service
        .create_user(
            scope,
            NewAccount {
                principal_id,
                first_name,
                last_name,
                password,
                email,
                model,
            },
        )
        .await
        .map_err(|_| Outcome::Failure)?;
    let Some(created) = created else {
        return Err(Outcome::Failure);
    };
    result_account(&created)
}

fn account_list(accounts: &[Account]) -> Result<Vec<u8>, Outcome> {
    let mut result = FieldMap::new();
    if accounts.is_empty() {
        result.insert("result".to_string(), Field::Str("null".to_string()));
    } else {
        for (index, account) in accounts.iter().enumerate() {
            result.insert(format!("account{index}"), Field::Map(account.to_fields()));
        }
    }
    xml_of(&result)
}

fn result_account(account: &Account) -> Result<Vec<u8>, Outcome> {
    let mut result = FieldMap::new();
    result.insert("result".to_string(), Field::Map(account.to_fields()));
    xml_of(&result)
}

fn result_null() -> Vec<u8> {
    let mut result = FieldMap::new();
    result.insert("result".to_string(), Field::Str("null".to_string()));
    xml_of(&result).unwrap_or_else(|_| FAILURE.to_vec())
}

fn xml_of(fields: &FieldMap) -> Result<Vec<u8>, Outcome> {
    build_xml_response(fields)
        .map(|text| text.into_bytes())
        .map_err(|_| Outcome::Failure)
}

enum OptionalUuid {
    Absent,
    Bad,
}

fn required_scope(fields: &FieldMap) -> Result<Uuid, Outcome> {
    match optional_uuid(fields, "ScopeID") {
        Ok(scope) => Ok(scope),
        Err(OptionalUuid::Absent) => Ok(Uuid::nil()),
        Err(OptionalUuid::Bad) => Err(Outcome::Failure),
    }
}

fn optional_uuid(fields: &FieldMap, key: &str) -> Result<Uuid, OptionalUuid> {
    let Some(text) = text_field(fields, key) else {
        return if fields.contains_key(key) {
            Err(OptionalUuid::Bad)
        } else {
            Err(OptionalUuid::Absent)
        };
    };
    Uuid::parse_str(&text).map_err(|_| OptionalUuid::Bad)
}

fn text_field(fields: &FieldMap, key: &str) -> Option<String> {
    fields.get(key).and_then(field_text)
}

fn field_text(field: &Field) -> Option<String> {
    match field {
        Field::Str(text) | Field::Uuid(text) => Some(text.clone()),
        Field::Bool(true) => Some("True".to_string()),
        Field::Bool(false) => Some("False".to_string()),
        Field::Int(number) => Some(number.to_string()),
        Field::List(_) | Field::Map(_) | Field::Null => None,
    }
}

fn failure() -> Response {
    xml(FAILURE.to_vec())
}

fn xml(body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        body,
    )
        .into_response()
}

fn plain(status: StatusCode, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body).into_response()
}
