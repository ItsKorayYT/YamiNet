//! Local, bounded text folding for case- and accent-insensitive search.
//!
//! No query or indexed text is ever sent anywhere. Folding only makes local
//! substring matching friendlier: `CAFÉ`, `café` and `cafe` all match the same
//! entry, while non-Latin scripts (for example Japanese) are matched exactly.

/// Folds one string for local substring comparison.
///
/// The result is lowercase, common Latin diacritics are reduced to an ASCII
/// base letter and combining marks are removed. This is a search convenience,
/// not a locale-aware collation or a substitute for Unicode normalization.
pub(crate) fn fold(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        for lower in character.to_lowercase() {
            if is_combining_mark(lower) {
                continue;
            }
            if let Some(base) = latin_base(lower) {
                output.push_str(base);
            } else {
                output.push(lower);
            }
        }
    }
    output
}

fn is_combining_mark(character: char) -> bool {
    matches!(
        character,
        '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{fe20}'..='\u{fe2f}'
    )
}

/// Common precomposed Latin letters, mapped to a searchable ASCII base.
fn latin_base(character: char) -> Option<&'static str> {
    Some(match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'þ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn folds_case_accents_and_sharp_s() {
        assert_eq!(fold("CAFÉ"), "cafe");
        assert_eq!(fold("Café"), "cafe");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("Ærø"), "aero");
        assert_eq!(fold("İstanbul"), "istanbul");
    }

    #[test]
    fn leaves_non_latin_text_available_for_exact_substrings() {
        assert_eq!(fold("日本語の手帖"), "日本語の手帖");
        assert!(fold("YamiNet で公開").contains("公開"));
    }
}
