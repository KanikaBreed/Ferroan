// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `UserSearchWords.From` at Pin5 (`c70fee205f`, TB18, #466).
//!
//! `string.Split()` drops empty pieces. Every word whose UTF-16 length is
//! under 3 is dropped. More than two words left, or none, means the search
//! returns no rows.

pub(crate) fn search_words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|word| utf16_len(word) >= 3)
        .map(str::to_string)
        .collect()
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_words_are_all_dropped() {
        assert!(search_words("  ab  ").is_empty());
        assert_eq!(search_words("ab GRID"), vec!["GRID".to_string()]);
        assert_eq!(search_words("ab cd GRID"), vec!["GRID".to_string()]);
        assert!(search_words("ab cd").is_empty());
        assert!(search_words("a bb c").is_empty());
        assert_eq!(
            search_words("aaa bbb ccc"),
            vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()]
        );
    }
}
