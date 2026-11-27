// Copyright (c) .NET Foundation and Contributors
//
// Licensed to the .NET Foundation under one or more agreements.
// The .NET Foundation licenses this file to you under the MIT license.
//
// Ported to Rust for Ferroan from
//   src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/RegexCharClass.cs
//   dotnet/runtime v10.0.14
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Property names and word characters from `RegexCharClass`.
//!
//! The range tables are not needed to accept or reject a pattern. A name is
//! known when it is a general category in `s_definedCategories` other than
//! `__InternalRegexIgnoreCase__`, or a block name in `s_propTable`.

use unicode_general_category::{GeneralCategory, get_general_category};

/// Category names, ordinal order, without the internal ignore-case entry.
pub(crate) const CATEGORIES: &[&str] = &[
    "C", "Cc", "Cf", "Cn", "Co", "Cs", "L", "Ll", "Lm", "Lo", "Lt", "Lu", "M", "Mc", "Me", "Mn",
    "N", "Nd", "Nl", "No", "P", "Pc", "Pd", "Pe", "Pf", "Pi", "Po", "Ps", "S", "Sc", "Sk", "Sm",
    "So", "Z", "Zl", "Zp", "Zs",
];

/// Block names from `s_propTable`, already in ordinal order, plus the four
/// XML names that share that table.
pub(crate) const PROPERTIES: &[&str] = &[
    "IsAlphabeticPresentationForms",
    "IsArabic",
    "IsArabicPresentationForms-A",
    "IsArabicPresentationForms-B",
    "IsArmenian",
    "IsArrows",
    "IsBasicLatin",
    "IsBengali",
    "IsBlockElements",
    "IsBopomofo",
    "IsBopomofoExtended",
    "IsBoxDrawing",
    "IsBraillePatterns",
    "IsBuhid",
    "IsCJKCompatibility",
    "IsCJKCompatibilityForms",
    "IsCJKCompatibilityIdeographs",
    "IsCJKRadicalsSupplement",
    "IsCJKSymbolsandPunctuation",
    "IsCJKUnifiedIdeographs",
    "IsCJKUnifiedIdeographsExtensionA",
    "IsCherokee",
    "IsCombiningDiacriticalMarks",
    "IsCombiningDiacriticalMarksforSymbols",
    "IsCombiningHalfMarks",
    "IsCombiningMarksforSymbols",
    "IsControlPictures",
    "IsCurrencySymbols",
    "IsCyrillic",
    "IsCyrillicSupplement",
    "IsDevanagari",
    "IsDingbats",
    "IsEnclosedAlphanumerics",
    "IsEnclosedCJKLettersandMonths",
    "IsEthiopic",
    "IsGeneralPunctuation",
    "IsGeometricShapes",
    "IsGeorgian",
    "IsGreek",
    "IsGreekExtended",
    "IsGreekandCoptic",
    "IsGujarati",
    "IsGurmukhi",
    "IsHalfwidthandFullwidthForms",
    "IsHangulCompatibilityJamo",
    "IsHangulJamo",
    "IsHangulSyllables",
    "IsHanunoo",
    "IsHebrew",
    "IsHighPrivateUseSurrogates",
    "IsHighSurrogates",
    "IsHiragana",
    "IsIPAExtensions",
    "IsIdeographicDescriptionCharacters",
    "IsKanbun",
    "IsKangxiRadicals",
    "IsKannada",
    "IsKatakana",
    "IsKatakanaPhoneticExtensions",
    "IsKhmer",
    "IsKhmerSymbols",
    "IsLao",
    "IsLatin-1Supplement",
    "IsLatinExtended-A",
    "IsLatinExtended-B",
    "IsLatinExtendedAdditional",
    "IsLetterlikeSymbols",
    "IsLimbu",
    "IsLowSurrogates",
    "IsMalayalam",
    "IsMathematicalOperators",
    "IsMiscellaneousMathematicalSymbols-A",
    "IsMiscellaneousMathematicalSymbols-B",
    "IsMiscellaneousSymbols",
    "IsMiscellaneousSymbolsandArrows",
    "IsMiscellaneousTechnical",
    "IsMongolian",
    "IsMyanmar",
    "IsNumberForms",
    "IsOgham",
    "IsOpticalCharacterRecognition",
    "IsOriya",
    "IsPhoneticExtensions",
    "IsPrivateUse",
    "IsPrivateUseArea",
    "IsRunic",
    "IsSinhala",
    "IsSmallFormVariants",
    "IsSpacingModifierLetters",
    "IsSpecials",
    "IsSuperscriptsandSubscripts",
    "IsSupplementalArrows-A",
    "IsSupplementalArrows-B",
    "IsSupplementalMathematicalOperators",
    "IsSyriac",
    "IsTagalog",
    "IsTagbanwa",
    "IsTaiLe",
    "IsTamil",
    "IsTelugu",
    "IsThaana",
    "IsThai",
    "IsTibetan",
    "IsUnifiedCanadianAboriginalSyllabics",
    "IsVariationSelectors",
    "IsYiRadicals",
    "IsYiSyllables",
    "IsYijingHexagramSymbols",
    "_xmlC",
    "_xmlD",
    "_xmlI",
    "_xmlW",
];

pub(crate) fn known_property(name: &str) -> bool {
    CATEGORIES.binary_search(&name).is_ok() || PROPERTIES.binary_search(&name).is_ok()
}

/// `RegexCharClass.WordCharAsciiLookup`. Bit `ch` is set when that ASCII unit
/// is a word character.
const WORD_ASCII: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x03, 0xFE, 0xFF, 0xFF, 0x87, 0xFE, 0xFF, 0xFF, 0x07,
];

fn word_category(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::NonspacingMark
            | GeneralCategory::DecimalNumber
            | GeneralCategory::ConnectorPunctuation
    )
}

/// `RegexCharClass.IsBoundaryWordChar` for one UTF-16 code unit.
///
/// A surrogate is not a word character. U+200C and U+200D are.
pub(crate) fn is_boundary_word(unit: u16) -> bool {
    if unit <= 127 {
        let byte = WORD_ASCII[usize::from(unit >> 3)];
        return byte & (1 << (unit & 7)) != 0;
    }
    let Some(ch) = char::from_u32(u32::from(unit)) else {
        return false;
    };
    word_category(get_general_category(ch)) || ch == '\u{200C}' || ch == '\u{200D}'
}
