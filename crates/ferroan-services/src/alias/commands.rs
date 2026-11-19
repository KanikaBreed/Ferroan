// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Alias console commands, without a prompt.

use uuid::Uuid;

use super::service::AliasService;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub lines: Vec<String>,
    pub error: Option<String>,
}

impl AliasService {
    pub async fn command(&self, argv: &[&str]) -> CommandOutcome {
        match argv {
            ["create", "alias", ..] => self.create_command(argv).await,
            ["show", "alias", ..] => self.show_command(argv).await,
            ["delete", "alias", ..] => self.delete_command(argv).await,
            _ => CommandOutcome {
                lines: Vec::new(),
                error: Some("unknown alias command".to_string()),
            },
        }
    }

    async fn create_command(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 5 {
            return error(
                "create alias needs <user id> <alias id> <description>; this process has no console prompt",
            );
        }
        let Ok(user_id) = Uuid::parse_str(argv[2]) else {
            return error(format!("ID {} is not a valid UUID", argv[2]));
        };
        let Ok(alias_id) = Uuid::parse_str(argv[3]) else {
            return error(format!("ID {} is not a valid UUID", argv[3]));
        };
        match self
            .create_alias(alias_id, user_id, Some(argv[4].to_string()))
            .await
        {
            Ok(Some(alias)) => lines(vec![format!(
                "Alias Created - UserID: {}, AliasID: {}, Description: {}",
                alias.user_id,
                alias.alias_id,
                alias.description.unwrap_or_default()
            )]),
            Ok(None) => error("alias was not stored"),
            Err(store_error) => error(store_error.to_string()),
        }
    }

    async fn show_command(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 3 {
            return error("show alias needs <user id>; this process has no console prompt");
        }
        let Ok(user_id) = Uuid::parse_str(argv[2]) else {
            return error(format!("ID {} is not a valid UUID", argv[2]));
        };
        match self.get_user_aliases(user_id).await {
            Ok(None) => lines(vec![format!("No aliases for user {}", argv[2])]),
            Ok(Some(aliases)) => lines(
                aliases
                    .into_iter()
                    .map(|alias| {
                        format!(
                            "UserID: {}, AliasID: {}, Description: {}",
                            alias.user_id,
                            alias.alias_id,
                            alias.description.unwrap_or_default()
                        )
                    })
                    .collect(),
            ),
            Err(store_error) => error(store_error.to_string()),
        }
    }

    async fn delete_command(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 3 {
            return error("delete alias needs <alias id>; this process has no console prompt");
        }
        let Ok(alias_id) = Uuid::parse_str(argv[2]) else {
            return error(format!("ID {} is not a valid UUID", argv[2]));
        };
        match self.delete_alias(alias_id).await {
            Ok(true) => lines(vec![format!("Alias for ID {} deleted", argv[2])]),
            Ok(false) => lines(vec![format!("No alias with ID {} found", argv[2])]),
            Err(store_error) => error(store_error.to_string()),
        }
    }
}

fn lines(lines: Vec<String>) -> CommandOutcome {
    CommandOutcome { lines, error: None }
}

fn error(message: impl Into<String>) -> CommandOutcome {
    CommandOutcome {
        lines: Vec::new(),
        error: Some(message.into()),
    }
}
