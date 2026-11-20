// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! UTC timestamps without an extra date crate. The day boundary is UTC.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub(crate) struct Stamp {
    pub plain: [u8; 23],
    pub rfc3339: [u8; 24],
    pub day: [u8; 8],
    pub unix_nano: u64,
}

pub(crate) fn stamp(now: SystemTime) -> Stamp {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let millis = since.subsec_millis();
    let nanos = since.subsec_nanos();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let tod = secs % 86_400;
    let hour = tod / 3_600;
    let minute = (tod % 3_600) / 60;
    let second = tod % 60;
    let (year, month, day) = civil_from_days(days);
    let mut plain = [0u8; 23];
    let mut rfc3339 = [0u8; 24];
    let mut ymd = [0u8; 8];
    write_width(&mut ymd, 0, year, 4);
    write_width(&mut ymd, 4, month, 2);
    write_width(&mut ymd, 6, day, 2);
    plain[..4].copy_from_slice(&ymd[..4]);
    plain[4] = b'-';
    plain[5..7].copy_from_slice(&ymd[4..6]);
    plain[7] = b'-';
    plain[8..10].copy_from_slice(&ymd[6..8]);
    plain[10] = b' ';
    write_width(&mut plain, 11, hour, 2);
    plain[13] = b':';
    write_width(&mut plain, 14, minute, 2);
    plain[16] = b':';
    write_width(&mut plain, 17, second, 2);
    plain[19] = b',';
    write_width(&mut plain, 20, u64::from(millis), 3);
    rfc3339[..10].copy_from_slice(&plain[..10]);
    rfc3339[10] = b'T';
    rfc3339[11..19].copy_from_slice(&plain[11..19]);
    rfc3339[19] = b'.';
    write_width(&mut rfc3339, 20, u64::from(millis), 3);
    rfc3339[23] = b'Z';
    let unix_nano = secs
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::from(nanos));
    Stamp {
        plain,
        rfc3339,
        day: ymd,
        unix_nano,
    }
}

pub(crate) fn stamp_str(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap_or("")
}

fn write_width(buf: &mut [u8], at: usize, mut value: u64, width: usize) {
    for offset in (0..width).rev() {
        buf[at + offset] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
    }
}

/// Howard Hinnant's civil-from-days, public domain. `days` is days since 1970-01-01.
fn civil_from_days(days: i64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (u64::try_from(year).unwrap_or(1970), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_a_known_day() {
        let epoch = stamp(UNIX_EPOCH);
        assert_eq!(stamp_str(&epoch.plain), "1970-01-01 00:00:00,000");
        assert_eq!(stamp_str(&epoch.rfc3339), "1970-01-01T00:00:00.000Z");
        assert_eq!(stamp_str(&epoch.day), "19700101");
        // 2026-11-20 09:30:25.123 UTC
        let later = UNIX_EPOCH
            + std::time::Duration::from_secs(1_795_167_025)
            + std::time::Duration::from_millis(123);
        let stamp = stamp(later);
        assert_eq!(stamp_str(&stamp.plain), "2026-11-20 09:30:25,123");
        assert_eq!(stamp_str(&stamp.rfc3339), "2026-11-20T09:30:25.123Z");
        assert_eq!(stamp_str(&stamp.day), "20261120");
    }
}
