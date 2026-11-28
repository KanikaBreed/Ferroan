// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// `double.ToString()` on .NET 10.0.14, en-US. The runtime source is not copied.
// YEngine calls that method for a float token.

//! Text of a `double` the way `ToString()` prints it on .NET 10, en-US.

pub fn format_f64(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let neg = value.is_sign_negative();
    let abs = value.abs();
    let sci = format!("{abs:e}");
    let Some((coeff, exp)) = split_sci(&sci) else {
        return if neg { format!("-{sci}") } else { sci };
    };
    let body = if exp >= 17 || exp <= -5 {
        scientific(coeff, exp)
    } else {
        fixed(coeff, exp)
    };
    if neg { format!("-{body}") } else { body }
}

fn split_sci(text: &str) -> Option<(&str, i32)> {
    let (coeff, exp) = text.split_once('e')?;
    let exp: i32 = exp.parse().ok()?;
    Some((coeff, exp))
}

fn scientific(coeff: &str, exp: i32) -> String {
    let sign = if exp < 0 { '-' } else { '+' };
    format!("{coeff}E{sign}{:02}", exp.unsigned_abs())
}

fn fixed(coeff: &str, exp: i32) -> String {
    let digits: String = coeff.chars().filter(|ch| *ch != '.').collect();
    if exp >= 0 {
        let left = exp as usize + 1;
        if digits.len() <= left {
            let mut out = digits;
            out.extend(std::iter::repeat_n('0', left - out.len()));
            out
        } else {
            let mut out = String::new();
            out.push_str(&digits[..left]);
            out.push('.');
            out.push_str(&digits[left..]);
            out
        }
    } else {
        let zeros = (-exp - 1) as usize;
        let mut out = String::from("0.");
        out.extend(std::iter::repeat_n('0', zeros));
        out.push_str(&digits);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::format_f64;

    #[test]
    fn matches_runtime_samples() {
        let samples = [
            (0x3FB999999999999Au64, "0.1"),
            (0x3EE4F8B588E368F1, "1E-05"),
            (0x4415AF1D78B58C40, "1E+20"),
            (0x400921FB54442D17, "3.1415926535897927"),
            (0x3FF0000000000005, "1.000000000000001"),
            (0x3FF0000000000000, "1"),
            (0x4059000000000000, "100"),
            (0x3F1A36E2EB1C432D, "0.0001"),
            (0x4029000000000000, "12.5"),
            (0x0000000000000000, "0"),
            (0x3F847AE147AE147B, "0.01"),
            (0x416312CFE0000000, "9999999"),
            (0x3FF8000000000000, "1.5"),
            (0x4024800000000000, "10.25"),
            (0x42D6BCC41E900000, "100000000000000"),
            (0x430C6BF526340000, "1000000000000000"),
            (0x4341C37937E08000, "10000000000000000"),
            (0x3F18E757928E0C9E, "9.5E-05"),
            (0x3F20624DD2F1A9FC, "0.000125"),
            (0x44DFE154F457EA13, "6.022E+23"),
            (0x7FF0000000000000, "Infinity"),
            (0xFFF0000000000000, "-Infinity"),
            (0x8000000000000000, "-0"),
        ];
        for (bits, text) in samples {
            assert_eq!(format_f64(f64::from_bits(bits)), text, "{bits:016X}");
        }
    }
}
