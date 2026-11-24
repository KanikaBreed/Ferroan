// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! en-US general spelling of one position coordinate.
//!
//! The text is the shortest round trip. Scientific notation is used when the
//! exponent is below -4 or at least 9. The marker is `E`, with a sign and at
//! least two digits. Infinities are U+221E. Two equal-length spellings at the
//! same distance keep the even final digit.

pub(crate) fn format_f32(value: f32) -> String {
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
    let negative = value.is_sign_negative();
    let magnitude = value.abs();
    let Some((digits, exponent)) = shortest_parts(magnitude) else {
        return "0".to_owned();
    };
    let (digits, exponent) = closest_even(magnitude, digits, exponent);
    let body = render(&digits, exponent);
    if negative { format!("-{body}") } else { body }
}

/// Prefer a neighbour that is closer, or equally close with an even final digit.
fn closest_even(value: f32, digits: String, exponent: i32) -> (String, i32) {
    let primary = render(&digits, exponent);
    let mut best_digits = digits.clone();
    let mut best_exponent = exponent;
    let mut best_distance = distance(value, &digits, exponent);
    for delta in [-2, -1, 1, 2] {
        let Some((alt_digits, alt_exponent)) = bump(&digits, exponent, delta) else {
            continue;
        };
        let alt = render(&alt_digits, alt_exponent);
        if alt.len() > primary.len() {
            continue;
        }
        let Some(parsed) = alt.parse::<f32>().ok() else {
            continue;
        };
        if parsed.to_bits() != value.to_bits() {
            continue;
        }
        let alt_distance = distance(value, &alt_digits, alt_exponent);
        let alt_even = even_final_digit(&alt_digits);
        let best_even = even_final_digit(&best_digits);
        if alt_distance < best_distance || (alt_distance == best_distance && alt_even && !best_even)
        {
            best_distance = alt_distance;
            best_digits = alt_digits;
            best_exponent = alt_exponent;
        }
    }
    (best_digits, best_exponent)
}

fn even_final_digit(digits: &str) -> bool {
    digits.as_bytes().last().copied().unwrap_or(b'0') % 2 == 0
}

fn shortest_parts(value: f32) -> Option<(String, i32)> {
    let text = format!("{value:?}");
    if let Some((mantissa, exponent)) = text.split_once(['e', 'E']) {
        let exponent = exponent.parse().ok()?;
        return Some((digits_from_mantissa(mantissa), exponent));
    }
    if let Some((whole, fraction)) = text.split_once('.') {
        if fraction.bytes().all(|byte| byte == b'0') {
            return Some((trim_end_zeros(whole), i32::try_from(whole.len()).ok()? - 1));
        }
        if whole == "0" {
            let zeros = fraction.bytes().take_while(|byte| *byte == b'0').count();
            let digits = trim_end_zeros(&fraction[zeros..]);
            // Leading zeros sit after the decimal, so the exponent is -(zeros + 1).
            let exponent = i32::try_from(zeros).ok()?.checked_add(1)?.checked_neg()?;
            return Some((digits, exponent));
        }
        let digits = format!("{whole}{}", trim_end_zeros(fraction));
        return Some((digits, i32::try_from(whole.len()).ok()? - 1));
    }
    Some((trim_end_zeros(&text), i32::try_from(text.len()).ok()? - 1))
}

fn digits_from_mantissa(mantissa: &str) -> String {
    let Some((whole, fraction)) = mantissa.split_once('.') else {
        return mantissa.to_owned();
    };
    let fraction = trim_end_zeros(fraction);
    if fraction.is_empty() {
        whole.to_owned()
    } else {
        format!("{whole}{fraction}")
    }
}

fn trim_end_zeros(text: &str) -> String {
    let trimmed = text.trim_end_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn render(digits: &str, exponent: i32) -> String {
    if digits.is_empty() {
        return "0".to_owned();
    }
    if !(-4..9).contains(&exponent) {
        scientific(digits, exponent)
    } else {
        fixed(digits, exponent)
    }
}

fn scientific(digits: &str, exponent: i32) -> String {
    let mut text = String::new();
    text.push(digits.as_bytes()[0] as char);
    if digits.len() > 1 {
        text.push('.');
        text.push_str(&digits[1..]);
    }
    text.push('E');
    text.push(if exponent >= 0 { '+' } else { '-' });
    text.push_str(&format!("{:02}", exponent.abs()));
    text
}

fn fixed(digits: &str, exponent: i32) -> String {
    if exponent < 0 {
        let mut text = String::from("0.");
        let zeros = usize::try_from(-exponent - 1).unwrap_or(0);
        text.extend(std::iter::repeat_n('0', zeros));
        text.push_str(digits);
        return text;
    }
    let Some(place) = usize::try_from(exponent).ok() else {
        return digits.to_owned();
    };
    if place + 1 >= digits.len() {
        let mut text = digits.to_owned();
        text.extend(std::iter::repeat_n('0', place + 1 - digits.len()));
        return text;
    }
    let mut text = String::new();
    text.push_str(&digits[..=place]);
    text.push('.');
    text.push_str(&digits[place + 1..]);
    text
}

fn bump(digits: &str, exponent: i32, delta: i32) -> Option<(String, i32)> {
    if delta == 0 || digits.is_empty() {
        return None;
    }
    let sign = delta.signum();
    let mut current = (digits.to_owned(), exponent);
    for _ in 0..delta.abs() {
        current = bump_one(&current.0, current.1, sign)?;
    }
    Some(current)
}

fn bump_one(digits: &str, exponent: i32, sign: i32) -> Option<(String, i32)> {
    let mut bytes: Vec<u8> = digits.bytes().collect();
    let mut index = bytes.len() - 1;
    let mut carry = sign;
    loop {
        let next = i32::from(bytes[index] - b'0') + carry;
        if (0..10).contains(&next) {
            bytes[index] = b'0' + u8::try_from(next).ok()?;
            return Some((bytes_as_string(bytes), exponent));
        }
        if next >= 10 {
            bytes[index] = b'0';
            carry = 1;
        } else {
            bytes[index] = b'9';
            carry = -1;
        }
        if index == 0 {
            break;
        }
        index -= 1;
    }
    if carry > 0 {
        bytes.insert(0, b'1');
        return Some((bytes_as_string(bytes), exponent.checked_add(1)?));
    }
    if bytes.first() == Some(&b'0') {
        bytes.remove(0);
    }
    if bytes.is_empty() {
        return None;
    }
    Some((bytes_as_string(bytes), exponent.checked_sub(1)?))
}

fn bytes_as_string(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_default()
}

fn distance(value: f32, digits: &str, exponent: i32) -> Distance {
    let (significand, power_two) = binary_parts(value.to_bits());
    let digits_value = decimal_digits(digits);
    let power_ten = exponent - (i32::try_from(digits.len()).unwrap_or(1) - 1);
    let (left_num, left_den) = power_of_two(significand, power_two);
    let (right_num, right_den) = power_of_ten(digits_value, power_ten);
    let numerator = big_abs_diff(
        &big_mul(&left_num, &right_den),
        &big_mul(&right_num, &left_den),
    );
    let denominator = big_mul(&left_den, &right_den);
    Distance {
        numerator,
        denominator,
    }
}

fn binary_parts(bits: u32) -> (u64, i32) {
    let exponent = i32::try_from((bits >> 23) & 0xff).unwrap_or(0);
    let mantissa = u64::from(bits & 0x7f_ffff);
    if exponent == 0 {
        (mantissa, -126 - 23)
    } else {
        (mantissa + (1 << 23), exponent - 127 - 23)
    }
}

fn decimal_digits(digits: &str) -> u64 {
    let mut value = 0u64;
    for byte in digits.bytes() {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(byte.saturating_sub(b'0')));
    }
    value
}

fn power_of_two(significand: u64, power: i32) -> (Big, Big) {
    if power >= 0 {
        (
            big_shift(
                &Big::from_u64(significand),
                u32::try_from(power).unwrap_or(0),
            ),
            Big::one(),
        )
    } else {
        (
            Big::from_u64(significand),
            big_shift(&Big::one(), u32::try_from(-power).unwrap_or(0)),
        )
    }
}

fn power_of_ten(significand: u64, power: i32) -> (Big, Big) {
    if power >= 0 {
        (
            big_mul_pow10(
                &Big::from_u64(significand),
                u32::try_from(power).unwrap_or(0),
            ),
            Big::one(),
        )
    } else {
        (
            Big::from_u64(significand),
            big_mul_pow10(&Big::one(), u32::try_from(-power).unwrap_or(0)),
        )
    }
}

#[derive(Clone)]
struct Distance {
    numerator: Big,
    denominator: Big,
}

impl PartialEq for Distance {
    fn eq(&self, other: &Self) -> bool {
        big_mul(&self.numerator, &other.denominator) == big_mul(&other.numerator, &self.denominator)
    }
}

impl Eq for Distance {}

impl PartialOrd for Distance {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Distance {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        big_cmp(
            &big_mul(&self.numerator, &other.denominator),
            &big_mul(&other.numerator, &self.denominator),
        )
    }
}

#[derive(Clone, Eq, PartialEq)]
struct Big {
    limbs: Vec<u32>,
}

impl Big {
    fn one() -> Self {
        Self { limbs: vec![1] }
    }

    fn from_u64(value: u64) -> Self {
        Self {
            limbs: vec![value as u32, (value >> 32) as u32],
        }
        .normalized()
    }

    fn normalized(mut self) -> Self {
        while self.limbs.len() > 1 && self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
        if self.limbs.is_empty() {
            self.limbs.push(0);
        }
        self
    }
}

fn big_cmp(left: &Big, right: &Big) -> std::cmp::Ordering {
    match left.limbs.len().cmp(&right.limbs.len()) {
        std::cmp::Ordering::Equal => left.limbs.iter().rev().cmp(right.limbs.iter().rev()),
        order => order,
    }
}

fn big_shift(value: &Big, bits: u32) -> Big {
    if value.limbs == [0] {
        return Big::from_u64(0);
    }
    let words = (bits / 32) as usize;
    let rest = bits % 32;
    let mut limbs = vec![0u32; words];
    let mut carry = 0u64;
    for limb in &value.limbs {
        let current = (u64::from(*limb) << rest) | carry;
        limbs.push(current as u32);
        carry = current >> 32;
    }
    if carry != 0 {
        limbs.push(carry as u32);
    }
    Big { limbs }.normalized()
}

fn big_mul(left: &Big, right: &Big) -> Big {
    if left.limbs == [0] || right.limbs == [0] {
        return Big::from_u64(0);
    }
    let mut limbs = vec![0u32; left.limbs.len() + right.limbs.len() + 1];
    for (left_index, &left_limb) in left.limbs.iter().enumerate() {
        let mut carry = 0u128;
        for (right_index, &right_limb) in right.limbs.iter().enumerate() {
            let current = u128::from(limbs[left_index + right_index])
                + u128::from(left_limb) * u128::from(right_limb)
                + carry;
            limbs[left_index + right_index] = current as u32;
            carry = current >> 32;
        }
        let mut index = left_index + right.limbs.len();
        while carry != 0 {
            if index == limbs.len() {
                limbs.push(0);
            }
            let current = u128::from(limbs[index]) + carry;
            limbs[index] = current as u32;
            carry = current >> 32;
            index += 1;
        }
    }
    Big { limbs }.normalized()
}

fn big_mul_pow10(value: &Big, mut exponent: u32) -> Big {
    let mut current = value.clone();
    while exponent > 0 {
        let step = exponent.min(9);
        current = big_mul_u32(&current, 10u32.pow(step));
        exponent -= step;
    }
    current
}

fn big_mul_u32(value: &Big, factor: u32) -> Big {
    let mut limbs = Vec::with_capacity(value.limbs.len() + 1);
    let mut carry = 0u64;
    for &limb in &value.limbs {
        let current = u64::from(limb) * u64::from(factor) + carry;
        limbs.push(current as u32);
        carry = current >> 32;
    }
    if carry != 0 {
        limbs.push(carry as u32);
    }
    Big { limbs }.normalized()
}

fn big_abs_diff(left: &Big, right: &Big) -> Big {
    if big_cmp(left, right) != std::cmp::Ordering::Less {
        big_sub(left, right)
    } else {
        big_sub(right, left)
    }
}

fn big_sub(left: &Big, right: &Big) -> Big {
    let mut limbs = Vec::with_capacity(left.limbs.len());
    let mut borrow = 0i64;
    for (index, left_limb) in left.limbs.iter().copied().enumerate() {
        let right_limb = i64::from(right.limbs.get(index).copied().unwrap_or(0));
        let left_limb = i64::from(left_limb);
        let mut current = left_limb - right_limb - borrow;
        if current < 0 {
            current += 1i64 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        limbs.push(current as u32);
    }
    Big { limbs }.normalized()
}

#[cfg(test)]
mod tests {
    use super::format_f32;

    #[test]
    fn float_text_matches_the_en_us_capture() {
        let captured = include_str!("../../../../fixtures/griduser/float-text.txt");
        let mut rows = 0usize;
        for (number, line) in captured.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((bits, expected)) = line.split_once('\t') else {
                panic!("line {} has no tab", number + 1);
            };
            let bits = u32::from_str_radix(bits, 16).expect("hex bits");
            let value = f32::from_bits(bits);
            assert_eq!(format_f32(value), expected, "{bits:08X}");
            match expected {
                "NaN" => assert!(value.is_nan()),
                "∞" | "-∞" | "-0" => {}
                other => {
                    let parsed: f32 = other.parse().expect("finite text");
                    assert_eq!(parsed.to_bits(), bits, "{other}");
                }
            }
            rows += 1;
        }
        assert!(rows >= 200, "{rows}");
    }
}
