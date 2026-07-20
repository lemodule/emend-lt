//! Ports of the `StringTools` case predicates LanguageTool's tagger branches on.

/// No lowercase letter appears (true for words with no letters at all).
pub fn is_all_uppercase(s: &str) -> bool {
    !s.chars().any(|c| c.is_alphabetic() && c.is_lowercase())
}

/// First char is uppercase and no later letter is uppercase.
pub fn is_capitalized_word(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) if first.is_uppercase() => {
            !chars.any(|c| c.is_alphabetic() && !c.is_lowercase())
        }
        _ => false,
    }
}

/// Some letter is not lowercase (i.e. at least one uppercase letter).
pub fn is_not_all_lowercase(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic() && !c.is_lowercase())
}

/// `!isAllUppercase && !isCapitalizedWord && isNotAllLowercase`.
pub fn is_mixed_case(s: &str) -> bool {
    !is_all_uppercase(s) && !is_capitalized_word(s) && is_not_all_lowercase(s)
}

/// Uppercase only the first character (Java `changeFirstCharCase(s, true)`).
pub fn uppercase_first_char(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicates() {
        assert!(is_all_uppercase("FRANCE"));
        assert!(is_all_uppercase("123"));
        assert!(!is_all_uppercase("France"));
        assert!(is_capitalized_word("France"));
        assert!(!is_capitalized_word("france"));
        assert!(!is_capitalized_word("FRANCE")); // has later uppercase letters
        assert!(is_mixed_case("iPhone"));
        assert!(is_mixed_case("McGraw"));
        assert!(!is_mixed_case("Hello"));
        assert!(!is_mixed_case("hello"));
        assert_eq!(uppercase_first_char("france"), "France");
    }
}
