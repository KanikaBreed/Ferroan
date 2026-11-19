// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `SQLiteUserAccountData.GetUsers` word filter.
//!
//! `string.Split()` on this pin drops empty pieces. A word whose UTF-16
//! length is under 3 is removed. The loop does not step back, so the word
//! that slides into the current index is not examined. More than two words
//! left, or none, means the search returns no rows.

pub(crate) fn search_words(query: &str) -> Vec<String> {
    let mut words: Vec<String> = query.split_whitespace().map(str::to_string).collect();
    let mut index = 0;
    while index < words.len() {
        if utf16_len(&words[index]) < 3 {
            if index + 1 != words.len() {
                words.remove(index);
                index += 1;
                continue;
            }
            words.pop();
            break;
        }
        index += 1;
    }
    words
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_words_follow_the_csharp_loop() {
        assert!(search_words("  ab  ").is_empty());
        assert_eq!(search_words("ab GRID"), vec!["GRID".to_string()]);
        assert_eq!(
            search_words("ab cd GRID"),
            vec!["cd".to_string(), "GRID".to_string()]
        );
        assert_eq!(search_words("ab cd"), vec!["cd".to_string()]);
        assert_eq!(search_words("a bb c"), vec!["bb".to_string()]);
        assert_eq!(
            search_words("aaa bbb ccc"),
            vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()]
        );
    }
}
