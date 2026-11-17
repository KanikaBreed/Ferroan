// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
//! OpenMetaverse text for reals and dates.
//!
//! `OSDReal.AsString` is `double.ToString("g")` under the en-US culture with
//! user overrides off. On the ICU build that produced the fixtures, the
//! infinity symbols are U+221E, not the strings "Infinity". `OSDDate.AsString`
//! prints `yyyy-MM-ddTHH:mm:ssZ`, or `.ff` (hundredths, cut off rather than
//! rounded) when `Millisecond > 0`.

use chrono::{DateTime, Utc};

/// 100-nanosecond ticks in one second. `DateTime` and `TimeSpan` use this.
pub(crate) const TICKS_PER_SECOND: i64 = 10_000_000;

/// Ticks since the Unix epoch, the way `DateTime.AddSeconds` counts them.
///
/// OpenMetaverse stores a `DateTime`. Binary LLSD stores
/// `TimeSpan.TotalSeconds` as a little-endian `f64`, and `AddSeconds` does
/// not return the same millisecond: 120 ms of `2020-01-02T03:04:05` comes
/// back as 119 ms and prints `.11`, while the XML text of the original value
/// is `.12`. The ticks are what those two results differ by. An `f64` alone
/// cannot hold both.
pub(crate) fn seconds_to_ticks(seconds: f64) -> i64 {
    if !seconds.is_finite() {
        return 0;
    }
    // DateTime.AddUnits: whole seconds, then the fraction truncated toward zero.
    let integral = seconds.trunc();
    let fractional = seconds - integral;
    let whole = (integral as i64).saturating_mul(TICKS_PER_SECOND);
    let frac = (fractional * TICKS_PER_SECOND as f64) as i64;
    whole.saturating_add(frac)
}

/// `TimeSpan.TotalSeconds` for a tick count.
pub(crate) fn ticks_to_seconds(ticks: i64) -> f64 {
    ticks as f64 / TICKS_PER_SECOND as f64
}

fn ticks_from_parts(secs: i64, nanos: u32) -> i64 {
    secs.saturating_mul(TICKS_PER_SECOND)
        .saturating_add((nanos as i64) / 100)
}

/// Read a date string the way the XML and notation parsers do.
///
/// The fraction in the text is kept (`.12` is 120 ms). It is not passed
/// through `AddSeconds`, which would turn that same instant into 119 ms.
pub(crate) fn parse_date(s: &str) -> i64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return ticks_from_parts(dt.timestamp(), dt.timestamp_subsec_nanos());
    }
    // People API dates: "2010-04-16T21:34:02+00:00Z" (offset and Z).
    if let Some(t) = s
        .strip_suffix('Z')
        .filter(|t| t.len() > 6 && matches!(t.as_bytes()[t.len() - 6], b'+' | b'-'))
        && let Ok(dt) = DateTime::parse_from_rfc3339(t)
    {
        return ticks_from_parts(dt.timestamp(), dt.timestamp_subsec_nanos());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        let u = dt.and_utc();
        return ticks_from_parts(u.timestamp(), u.timestamp_subsec_nanos());
    }
    0
}

/// `OSDDate.AsString`.
pub(crate) fn format_date(ticks: i64) -> String {
    let secs = ticks.div_euclid(TICKS_PER_SECOND);
    let frac = ticks.rem_euclid(TICKS_PER_SECOND);
    let millis = frac / (TICKS_PER_SECOND / 1000);
    let Some(dt) = DateTime::<Utc>::from_timestamp(secs, 0) else {
        return "1970-01-01T00:00:00Z".to_owned();
    };
    if millis == 0 {
        return dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    }
    // `.ff` is the first two digits of the millisecond count, not a rounded hundredth.
    let hundredths = millis / 10;
    format!("{}{hundredths:02}Z", dt.format("%Y-%m-%dT%H:%M:%S."))
}

/// `double.ToString("g")` for en-US ICU: shortest round trip, scientific when
/// the exponent is below -4 or at least 17, and U+221E for infinities.
pub(crate) fn format_real(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-∞".to_owned()
        } else {
            "∞".to_owned()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    let neg = value.is_sign_negative();
    let (digits, exp) = shortest_parts(value.abs());
    let body = if !(-4..17).contains(&exp) {
        scientific(&digits, exp)
    } else {
        fixed(&digits, exp)
    };
    if neg { format!("-{body}") } else { body }
}

/// Significant digits and the scientific exponent of Rust's shortest debug form.
fn shortest_parts(value: f64) -> (String, i32) {
    let text = format!("{value:?}");
    if let Some((mant, exp_s)) = text.split_once(['e', 'E']) {
        let exp = exp_s.parse::<i32>().unwrap_or(0);
        let digits: String = mant.chars().filter(|c| *c != '.' && *c != '+').collect();
        return (digits, exp);
    }
    if let Some((int_part, frac_part)) = text.split_once('.') {
        if frac_part.chars().all(|c| c == '0') {
            let exp = int_part.len() as i32 - 1;
            return (trim_end_zeros(int_part), exp);
        }
        if int_part == "0" {
            let zeros = frac_part.chars().take_while(|c| *c == '0').count();
            let digits = frac_part[zeros..].trim_end_matches('0').to_owned();
            return (digits, -((zeros as i32) + 1));
        }
        let digits = format!("{int_part}{}", frac_part.trim_end_matches('0'));
        return (digits, int_part.len() as i32 - 1);
    }
    (trim_end_zeros(&text), text.len() as i32 - 1)
}

fn trim_end_zeros(s: &str) -> String {
    let t = s.trim_end_matches('0');
    if t.is_empty() {
        "0".to_owned()
    } else {
        t.to_owned()
    }
}

fn scientific(digits: &str, exp: i32) -> String {
    let mut s = String::new();
    s.push(digits.as_bytes()[0] as char);
    if digits.len() > 1 {
        s.push('.');
        s.push_str(&digits[1..]);
    }
    s.push('e');
    s.push(if exp >= 0 { '+' } else { '-' });
    s.push_str(&format!("{:02}", exp.abs()));
    s
}

fn fixed(digits: &str, exp: i32) -> String {
    if exp < 0 {
        let mut s = String::from("0.");
        s.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
        s.push_str(digits);
        return s;
    }
    let place = exp as usize;
    if place + 1 >= digits.len() {
        let mut s = digits.to_owned();
        s.extend(std::iter::repeat_n('0', place + 1 - digits.len()));
        return s;
    }
    let mut s = String::new();
    s.push_str(&digits[..=place]);
    s.push('.');
    s.push_str(&digits[place + 1..]);
    s
}

/// OpenMetaverse's XML writer appends real text with `AppendASCII`, which
/// keeps the low 8 bits of each UTF-16 unit. U+221E (∞) becomes 0x1E.
pub(crate) fn push_ascii_low(text: &str, out: &mut String) {
    for c in text.chars() {
        out.push(char::from((c as u32 & 0xff) as u8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_format_matches_en_us_icu() {
        let cases: &[(u64, &str)] = &[
            (0x0000_0000_0000_0000, "0"),
            (0x8000_0000_0000_0000, "-0"),
            (0x3ff0_0000_0000_0000, "1"),
            (0x3fb9_9999_9999_999a, "0.1"),
            (0x3ff8_0000_0000_0000, "1.5"),
            (0x412c_ecf6_77ce_d917, "947835.234"),
            (0x0000_0000_0000_0001, "5e-324"),
            (0x7fef_ffff_ffff_ffff, "1.7976931348623157e+308"),
            (0xffef_ffff_ffff_ffff, "-1.7976931348623157e+308"),
            (0xfff8_0000_0000_0000, "NaN"),
            (0x7ff0_0000_0000_0000, "∞"),
            (0xfff0_0000_0000_0000, "-∞"),
            (0x3f1a_36e2_eb1c_432d, "0.0001"),
            (0x3ee4_f8b5_88e3_68f1, "1e-05"),
            (0x3f19_f3c7_0c99_6b76, "9.9e-05"),
            (0x430c_6bf5_2634_0000, "1000000000000000"),
            (0x4341_c379_37e0_8000, "10000000000000000"),
            (0x4376_3457_85d8_a000, "1e+17"),
            (0x4415_af1d_78b5_8c40, "1e+20"),
            (0x444b_1ae4_d6e2_ef50, "1e+21"),
            (0x4009_21fb_5444_2d18, "3.141592653589793"),
            (0xbf20_2c9d_edbc_309d, "-0.0001234"),
            (0x3ff0_0000_0000_0001, "1.0000000000000002"),
            (0x0010_0000_0000_0000, "2.2250738585072014e-308"),
            (0x42dc_1221_8377_de6b, "123456789012345.67"),
            (0x3f1a_36e2_eb1c_432c, "9.999999999999999e-05"),
            (0x4163_12cf_e000_0000, "9999999"),
            (0x412e_8480_0000_0000, "1000000"),
            (0x3fd3_3333_3333_3333, "0.3"),
            (0x3fd5_5555_5555_5555, "0.3333333333333333"),
            (0x3e80_823f_7115_5233, "1.23e-07"),
            (0x444a_c4da_03bc_47e4, "9.876e+20"),
            (0x430c_6bf5_2633_ffff, "999999999999999.9"),
        ];
        for (bits, text) in cases {
            let got = format_real(f64::from_bits(*bits));
            assert_eq!(got, *text, "{bits:016x}");
        }
    }

    #[test]
    fn dates_follow_add_seconds_and_the_written_text() {
        let wires: &[([u8; 8], &str)] = &[
            (
                [0x00, 0x00, 0x40, 0x69, 0x57, 0x83, 0xd7, 0x41],
                "2020-01-02T03:04:05Z",
            ),
            (
                [0x14, 0xae, 0x47, 0x69, 0x57, 0x83, 0xd7, 0x41],
                "2020-01-02T03:04:05.11Z",
            ),
            (
                [0x3b, 0xdf, 0x47, 0x69, 0x57, 0x83, 0xd7, 0x41],
                "2020-01-02T03:04:05.12Z",
            ),
        ];
        for (raw, text) in wires {
            let seconds = f64::from_le_bytes(*raw);
            let ticks = seconds_to_ticks(seconds);
            assert_eq!(
                ticks_to_seconds(ticks).to_bits(),
                seconds.to_bits(),
                "{text}"
            );
            assert_eq!(format_date(ticks), *text);
        }
        let written = parse_date("2020-01-02T03:04:05.12Z");
        assert_eq!(format_date(written), "2020-01-02T03:04:05.12Z");
        let collapsed = seconds_to_ticks(ticks_to_seconds(written));
        assert_ne!(written, collapsed);
        assert_eq!(format_date(collapsed), "2020-01-02T03:04:05.11Z");
    }
}
