// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Console commands that create and change accounts.
//!
//! `ferroan-robust` has no `MainConsole`. These functions take the same argv
//! the C# handlers see (`create user` is two words, so the first name is
//! argv[2]). A missing value that C# would prompt for is an error. `import
//! users` and `update users` always read `Imports/users.csv` under `root` and
//! ignore a file argument. A line shorter than five fields is skipped. A line
//! of five fields stops the import: C# then indexes the missing rezday. The
//! password is passed to `CreateUser` and is not stored while authentication
//! is unset. It is not written into the outcome.

use std::path::Path;

use uuid::Uuid;

use super::service::{AccountService, NewAccount};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub lines: Vec<String>,
    pub error: Option<String>,
}

impl CommandOutcome {
    fn lines(lines: Vec<String>) -> Self {
        Self { lines, error: None }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            lines: Vec::new(),
            error: Some(message.into()),
        }
    }

    fn stop(lines: Vec<String>, message: impl Into<String>) -> Self {
        Self {
            lines,
            error: Some(message.into()),
        }
    }
}

impl AccountService {
    pub async fn command(&self, argv: &[&str], root: &Path) -> CommandOutcome {
        match argv {
            ["show", "account", ..] => self.show_account(argv).await,
            ["create", "user", ..] => self.create_user_command(argv).await,
            ["reset", "user", "password", ..] => self.reset_password(argv).await,
            ["reset", "user", "email", ..] => self.reset_email(argv).await,
            ["set", "user", "level", ..] => self.set_level(argv).await,
            ["set", "display", "name", ..] => self.set_display_command(argv).await,
            ["import", "users", ..] => self.import_users(root).await,
            ["update", "users", ..] => self.update_users(root).await,
            _ => CommandOutcome::error("unknown account command"),
        }
    }

    async fn show_account(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() != 4 {
            return CommandOutcome::lines(vec![
                "Usage: show account <first-name> <last-name>".to_string(),
            ]);
        }
        let Some(account) = self
            .get_by_name(Uuid::nil(), argv[2], argv[3])
            .await
            .ok()
            .flatten()
        else {
            return CommandOutcome::lines(vec![format!("No user named {} {}", argv[2], argv[3])]);
        };
        let mut lines = vec![
            format!("Name:    {}", account.formatted_name()),
            format!("ID:      {}", account.principal_id),
            format!("Title:   {}", account.user_title.unwrap_or_default()),
            format!("E-mail:  {}", account.email),
            format!("Created: {}", unix_general(account.created)),
            format!("Level:   {}", account.user_level),
            format!("Flags:   {}", account.user_flags),
        ];
        for (key, value) in &account.service_urls {
            lines.push(format!("{key}: {value}"));
        }
        CommandOutcome::lines(lines)
    }

    async fn create_user_command(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 8 {
            return CommandOutcome::error(
                "create user needs <first> <last> <password> <email> <user id> <model>; this process has no console prompt",
            );
        }
        let principal = if argv[6].trim().is_empty() {
            Uuid::new_v4()
        } else {
            match Uuid::parse_str(argv[6]) {
                Ok(id) => id,
                Err(_) => {
                    return CommandOutcome::error(format!("ID {} is not a valid UUID", argv[6]));
                }
            }
        };
        match self
            .create_user(
                Uuid::nil(),
                NewAccount {
                    principal_id: principal,
                    first_name: argv[2].to_string(),
                    last_name: argv[3].to_string(),
                    password: argv[4].to_string(),
                    email: argv[5].to_string(),
                    model: argv[7].to_string(),
                },
            )
            .await
        {
            Ok(Some(_)) => CommandOutcome::lines(Vec::new()),
            Ok(None) => CommandOutcome::error(format!(
                "[USER ACCOUNT SERVICE]: A user with the name {} {} already exists!",
                argv[2].trim(),
                argv[3].trim()
            )),
            Err(error) => CommandOutcome::error(error.to_string()),
        }
    }

    async fn reset_password(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 6 {
            return CommandOutcome::error(
                "reset user password needs <first> <last> <password>; this process has no console prompt",
            );
        }
        let Some(account) = self
            .get_by_name(Uuid::nil(), argv[3], argv[4])
            .await
            .ok()
            .flatten()
        else {
            return CommandOutcome::lines(vec![format!("No such user as {} {}", argv[3], argv[4])]);
        };
        let _ = account;
        // AuthenticationService is unset, so SetPassword is not called.
        CommandOutcome::lines(vec![format!(
            "Unable to reset password for account {} {}.",
            argv[3], argv[4]
        )])
    }

    async fn reset_email(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 6 {
            return CommandOutcome::error(
                "reset user email needs <first> <last> <email>; this process has no console prompt",
            );
        }
        let Some(mut account) = self
            .get_by_name(Uuid::nil(), argv[3], argv[4])
            .await
            .ok()
            .flatten()
        else {
            return CommandOutcome::lines(vec![format!("No such user as {} {}", argv[3], argv[4])]);
        };
        account.email = argv[5].to_string();
        match self.store_user(&account).await {
            Ok(true) => CommandOutcome::lines(vec![format!(
                "User Email set for user {} {} to {}",
                argv[3], argv[4], account.email
            )]),
            Ok(false) => CommandOutcome::lines(vec![format!(
                "Unable to set Email for account {} {}.",
                argv[3], argv[4]
            )]),
            Err(error) => CommandOutcome::error(error.to_string()),
        }
    }

    async fn set_level(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 6 {
            return CommandOutcome::error(
                "set user level needs <first> <last> <level>; this process has no console prompt",
            );
        }
        let Some(mut account) = self
            .get_by_name(Uuid::nil(), argv[3], argv[4])
            .await
            .ok()
            .flatten()
        else {
            return CommandOutcome::lines(vec!["No such user".to_string()]);
        };
        let Ok(level) = argv[5].parse::<i32>() else {
            return CommandOutcome::lines(vec!["Invalid user level".to_string()]);
        };
        account.user_level = level;
        match self.store_user(&account).await {
            Ok(true) => CommandOutcome::lines(vec![format!(
                "User level set for user {} {} to {level}",
                argv[3], argv[4]
            )]),
            Ok(false) => CommandOutcome::lines(vec![format!(
                "Unable to set user level for account {} {}.",
                argv[3], argv[4]
            )]),
            Err(error) => CommandOutcome::error(error.to_string()),
        }
    }

    async fn set_display_command(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() < 6 {
            return CommandOutcome::error(
                "set display name needs <first> <last> <display name>; this process has no console prompt",
            );
        }
        let Some(account) = self
            .get_by_name(Uuid::nil(), argv[3], argv[4])
            .await
            .ok()
            .flatten()
        else {
            return CommandOutcome::lines(vec![format!("No such user as {} {}", argv[3], argv[4])]);
        };
        match self.set_display_name(account.principal_id, argv[5]).await {
            Ok(true) => CommandOutcome::lines(vec!["Display name updated!".to_string()]),
            Ok(false) => CommandOutcome::lines(vec![format!(
                "Unable to set DisplayName for account {} {}.",
                argv[3], argv[4]
            )]),
            Err(error) => CommandOutcome::error(error.to_string()),
        }
    }

    async fn import_users(&self, root: &Path) -> CommandOutcome {
        let path = root.join("Imports").join("users.csv");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                return CommandOutcome::error(format!(
                    "could not read {}: {error}",
                    path.display()
                ));
            }
        };
        let mut lines = Vec::new();
        let mut added = 0_u32;
        for line in text.lines() {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() < 5 {
                lines.push(
                    "[User Load]: Insufficient details; Skipping System.String[]".to_string(),
                );
                continue;
            }
            if fields.len() == 5 {
                return CommandOutcome::stop(lines, "import stopped: rezday is missing");
            }
            let Ok(user_id) = Uuid::parse_str(fields[0]) else {
                return CommandOutcome::stop(lines, "import stopped: user id is not a UUID");
            };
            let Ok(rezday) = fields[5].trim().parse::<i32>() else {
                return CommandOutcome::stop(lines, "import stopped: rezday is not an integer");
            };
            let created = self
                .create_user(
                    Uuid::nil(),
                    NewAccount {
                        principal_id: user_id,
                        first_name: fields[1].to_string(),
                        last_name: fields[2].to_string(),
                        password: fields[3].to_string(),
                        email: fields[4].to_string(),
                        model: String::new(),
                    },
                )
                .await;
            let Ok(_) = created else {
                return CommandOutcome::stop(lines, "import stopped: account store failed");
            };
            let Some(mut account) = self
                .get_by_name(Uuid::nil(), fields[1], fields[2])
                .await
                .ok()
                .flatten()
            else {
                return CommandOutcome::stop(lines, "import stopped: account was not stored");
            };
            account.created = rezday;
            if self.store_user(&account).await.ok() != Some(true) {
                return CommandOutcome::stop(lines, "import stopped: could not store rezday");
            }
            added += 1;
        }
        lines.push(format!("File: users.csv loaded,  {added} users added"));
        CommandOutcome::lines(lines)
    }

    async fn update_users(&self, root: &Path) -> CommandOutcome {
        let path = root.join("Imports").join("users.csv");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                return CommandOutcome::error(format!(
                    "could not read {}: {error}",
                    path.display()
                ));
            }
        };
        let mut lines = Vec::new();
        let mut updated = 0_u32;
        for line in text.lines() {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() < 5 {
                lines.push(
                    "[USER UPDATE]: Insufficient details; Skipping System.String[]".to_string(),
                );
                continue;
            }
            if fields.len() == 5 {
                return CommandOutcome::stop(lines, "update stopped: rezday is missing");
            }
            let Ok(rezday) = fields[5].trim().parse::<i32>() else {
                return CommandOutcome::stop(lines, "update stopped: rezday is not an integer");
            };
            let Some(mut account) = self
                .get_by_name(Uuid::nil(), fields[1], fields[2])
                .await
                .ok()
                .flatten()
            else {
                return CommandOutcome::stop(
                    lines,
                    format!("update stopped: no user {} {}", fields[1], fields[2]),
                );
            };
            account.created = rezday;
            if self.store_user(&account).await.ok() != Some(true) {
                return CommandOutcome::stop(lines, "update stopped: could not store the account");
            }
            lines.push(format!(
                "[USER UPDATE]: User {} {} has been updated",
                fields[1], fields[2]
            ));
            updated += 1;
        }
        lines.push(format!("File: users.csv loaded,  {updated} users updated"));
        CommandOutcome::lines(lines)
    }
}

/// Invariant-culture general format of `Utils.UnixTimeToDateTime`.
pub fn unix_general(created: i32) -> String {
    let seconds = u64::from(created.cast_unsigned());
    let days = (seconds / 86_400) as i64;
    let time = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = time / 3600;
    let minute = (time % 3600) / 60;
    let second = time % 60;
    format!("{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_uses_the_invariant_general_pattern() {
        assert_eq!(unix_general(0), "01/01/1970 00:00:00");
        assert_eq!(unix_general(1_700_000_000), "11/14/2023 22:13:20");
    }
}
