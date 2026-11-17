// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Versions Tranquillity's migration resources reach from an empty database.
//!
//! Each number is the highest `:VERSION` in that backend's resource, or the
//! `NNN` in an old-style `NNN_Store.sql` file. `Source/OpenSim.Data/Migration.cs`
//! applies every higher version and records the last one. The three backends
//! do not share version numbers.
//!
//! `store_versions_in_dir` re-reads the pinned checkout. The arrays here are
//! what a service checks at runtime, including on CI, where that checkout is
//! not present.

use crate::backend::Backend;
use crate::error::DbError;

/// One store and the version an up-to-date Tranquillity database holds for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreVersion {
    /// Value stored in `migrations.name`.
    pub name: &'static str,
    /// Value stored in `migrations.version` after `Migration.Update` from empty.
    pub version: i32,
}

/// Stores `backend` knows how to check, sorted by name.
pub fn expected_stores(backend: Backend) -> &'static [StoreVersion] {
    match backend {
        Backend::Sqlite => SQLITE,
        Backend::Mysql => MYSQL,
        Backend::Postgres => POSTGRES,
    }
}

/// The version `backend` expects for `store`.
pub fn expected_version(backend: Backend, store: &str) -> Result<i32, DbError> {
    expected_stores(backend)
        .iter()
        .find(|entry| entry.name == store)
        .map(|entry| entry.version)
        .ok_or_else(|| DbError::UnknownStore {
            backend,
            store: store.to_string(),
        })
}

/// `OpenSim.Data.SQLite/Resources` at `713857f10b8e91a639a967d4354a4682b080793c`.
const SQLITE: &[StoreVersion] = &[
    StoreVersion {
        name: "AgentPrefs",
        version: 2,
    },
    StoreVersion {
        name: "AssetStore",
        version: 6,
    },
    StoreVersion {
        name: "AuthStore",
        version: 2,
    },
    StoreVersion {
        name: "Avatar",
        version: 1,
    },
    StoreVersion {
        name: "EstateStore",
        version: 13,
    },
    StoreVersion {
        name: "FriendsStore",
        version: 2,
    },
    StoreVersion {
        name: "GridUserStore",
        version: 1,
    },
    StoreVersion {
        name: "HGTravelStore",
        version: 2,
    },
    StoreVersion {
        name: "MuteListStore",
        version: 1,
    },
    StoreVersion {
        name: "RegionStore",
        version: 46,
    },
    StoreVersion {
        name: "TrustedGrid",
        version: 1,
    },
    StoreVersion {
        name: "UserAccount",
        version: 3,
    },
    StoreVersion {
        name: "UserProfiles",
        version: 4,
    },
    StoreVersion {
        name: "XInventoryStore",
        version: 3,
    },
];

/// `OpenSim.Data.MySQL/Resources` at the same revision.
const MYSQL: &[StoreVersion] = &[
    StoreVersion {
        name: "AgentPrefs",
        version: 1,
    },
    StoreVersion {
        name: "AssetStore",
        version: 10,
    },
    StoreVersion {
        name: "AuthStore",
        version: 4,
    },
    StoreVersion {
        name: "Avatar",
        version: 3,
    },
    StoreVersion {
        name: "EstateStore",
        version: 38,
    },
    StoreVersion {
        name: "Experience",
        version: 4,
    },
    StoreVersion {
        name: "FSAssetStore",
        version: 1,
    },
    StoreVersion {
        name: "FriendsStore",
        version: 4,
    },
    StoreVersion {
        name: "GridStore",
        version: 11,
    },
    StoreVersion {
        name: "GridUserStore",
        version: 2,
    },
    StoreVersion {
        name: "HGTravelStore",
        version: 2,
    },
    StoreVersion {
        name: "IM_Store",
        version: 5,
    },
    StoreVersion {
        name: "InventoryStore",
        version: 8,
    },
    StoreVersion {
        name: "LogStore",
        version: 1,
    },
    StoreVersion {
        name: "MuteListStore",
        version: 1,
    },
    StoreVersion {
        name: "Presence",
        version: 4,
    },
    StoreVersion {
        name: "RegionStore",
        version: 70,
    },
    StoreVersion {
        name: "TrustedGrid",
        version: 1,
    },
    StoreVersion {
        name: "UserAccount",
        version: 7,
    },
    StoreVersion {
        name: "UserAlias",
        version: 1,
    },
    StoreVersion {
        name: "UserProfiles",
        version: 5,
    },
    StoreVersion {
        name: "XAssetStore",
        version: 2,
    },
    StoreVersion {
        name: "os_groups_Store",
        version: 3,
    },
];

/// `OpenSim.Data.PGSQL/Resources` at the same revision.
const POSTGRES: &[StoreVersion] = &[
    StoreVersion {
        name: "AgentPrefs",
        version: 1,
    },
    StoreVersion {
        name: "AssetStore",
        version: 9,
    },
    StoreVersion {
        name: "AuthStore",
        version: 3,
    },
    StoreVersion {
        name: "Avatar",
        version: 3,
    },
    StoreVersion {
        name: "EstateStore",
        version: 16,
    },
    StoreVersion {
        name: "FSAssetStore",
        version: 1,
    },
    StoreVersion {
        name: "FriendsStore",
        version: 3,
    },
    StoreVersion {
        name: "GridStore",
        version: 10,
    },
    StoreVersion {
        name: "GridUserStore",
        version: 2,
    },
    StoreVersion {
        name: "HGTravelStore",
        version: 1,
    },
    StoreVersion {
        name: "IM_Store",
        version: 3,
    },
    StoreVersion {
        name: "InventoryStore",
        version: 11,
    },
    StoreVersion {
        name: "LogStore",
        version: 1,
    },
    StoreVersion {
        name: "MuteListStore",
        version: 1,
    },
    StoreVersion {
        name: "Presence",
        version: 4,
    },
    StoreVersion {
        name: "RegionStore",
        version: 61,
    },
    StoreVersion {
        name: "UserAccount",
        version: 5,
    },
    StoreVersion {
        name: "UserProfiles",
        version: 6,
    },
    StoreVersion {
        name: "UserStore",
        version: 11,
    },
    StoreVersion {
        name: "XAssetStore",
        version: 5,
    },
    StoreVersion {
        name: "os_groups_Store",
        version: 4,
    },
];

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use super::*;
    use crate::migrations::store_versions_in_dir;

    #[test]
    fn catalogs_are_sorted_and_positive() {
        for backend in [Backend::Sqlite, Backend::Mysql, Backend::Postgres] {
            let stores = expected_stores(backend);
            assert!(!stores.is_empty());
            let mut previous = "";
            for store in stores {
                assert!(store.version > 0, "{}", store.name);
                assert_ne!(store.name, "migrations");
                assert!(store.name.len() <= 100);
                assert!(previous < store.name, "{previous} then {}", store.name);
                previous = store.name;
            }
        }
    }

    #[test]
    fn sqlite_asset_store_is_the_pinned_version() {
        assert_eq!(expected_version(Backend::Sqlite, "AssetStore").unwrap(), 6);
        assert!(expected_version(Backend::Sqlite, "InventoryStore").is_err());
        assert_eq!(expected_version(Backend::Mysql, "AssetStore").unwrap(), 10);
    }

    #[test]
    fn catalog_matches_the_pinned_checkout_when_it_is_present() {
        let Some(root) = reference_root() else {
            eprintln!("reference checkout not found; migration catalog was not compared");
            return;
        };
        let pairs = [
            (Backend::Sqlite, "OpenSim.Data.SQLite"),
            (Backend::Mysql, "OpenSim.Data.MySQL"),
            (Backend::Postgres, "OpenSim.Data.PGSQL"),
        ];
        for (backend, project) in pairs {
            let dir = root.join(format!("OpenSim-Tranquillity/Source/{project}/Resources"));
            if !dir.is_dir() {
                eprintln!(
                    "missing {}; migration catalog was not compared",
                    dir.display()
                );
                return;
            }
            let found = store_versions_in_dir(&dir).unwrap_or_else(|error| panic!("{error}"));
            let expected: Vec<(String, i32)> = expected_stores(backend)
                .iter()
                .map(|store| (store.name.to_string(), store.version))
                .collect();
            assert_eq!(found, expected, "{backend} at {}", dir.display());
        }
    }

    fn reference_root() -> Option<PathBuf> {
        let set = env::var("FERROAN_REF")
            .ok()
            .filter(|value| !value.is_empty());
        let path = if let Some(value) = set {
            if let Some(rest) = value.strip_prefix("~/") {
                env::var_os("HOME")
                    .or_else(|| env::var_os("USERPROFILE"))
                    .map(PathBuf::from)?
                    .join(rest)
            } else {
                PathBuf::from(value)
            }
        } else {
            let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))?;
            PathBuf::from(home).join("Git/reference")
        };
        path.is_dir().then_some(path)
    }
}
