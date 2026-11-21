// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The clocks the two asset plugins write into `create_time` and `access_time`.
//!
//! MySQL uses `Utils.DateTimeToUnixTime(DateTime.UtcNow)`, which is UTC unix
//! seconds. PostgreSQL subtracts `new DateTime(1970, 1, 1).Ticks` from
//! `DateTime.Now.Ticks`, so the stored number is the local clock read as if
//! it were UTC. Neither plugin updates `access_time` on get.
//! `UpdateAccessTime` in `MySQLAssetData` is never called, and the PostgreSQL
//! copy is commented out.

/// `start + count - 1`, the limit `PGSQLAssetData.FetchAssetMetadataSet` binds
/// as `:stop`. C# `int` addition is unchecked.
pub(super) fn postgres_metadata_limit(start: i32, count: i32) -> i32 {
    start.wrapping_add(count).wrapping_sub(1)
}

pub(super) fn mysql_unix_now() -> i32 {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    i32::try_from(seconds).unwrap_or(i32::MAX)
}

pub(super) fn postgres_unix_now() -> i32 {
    let Some(epoch) =
        chrono::NaiveDate::from_ymd_opt(1970, 1, 1).and_then(|date| date.and_hms_opt(0, 0, 0))
    else {
        return 0;
    };
    let local = chrono::Local::now().naive_local();
    i32::try_from((local - epoch).num_seconds()).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{mysql_unix_now, postgres_metadata_limit, postgres_unix_now};

    #[test]
    fn postgres_metadata_limit_matches_the_csharp_stop() {
        assert_eq!(postgres_metadata_limit(0, 10), 9);
        assert_eq!(postgres_metadata_limit(0, 1), 0);
        assert_eq!(postgres_metadata_limit(2, 5), 6);
        assert_eq!(postgres_metadata_limit(10, 0), 9);
    }

    #[test]
    fn the_two_clocks_stay_near_unix_time() {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs() as i64;
        let mysql = i64::from(mysql_unix_now());
        let postgres = i64::from(postgres_unix_now());
        assert!((mysql - unix).abs() <= 2, "mysql {mysql} unix {unix}");
        assert!(
            (postgres - unix).abs() <= 18 * 3600,
            "postgres {postgres} unix {unix}"
        );
    }
}
