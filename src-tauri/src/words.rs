use std::collections::HashMap;

const MAX_ITEMS: usize = 40;

#[derive(Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub keyword: bool,
}

fn starts_with_ignoring_case(word: &str, lower_prefix: &str) -> bool {
    let mut word = word.chars().flat_map(char::to_lowercase);
    lower_prefix.chars().all(|c| word.next() == Some(c))
}

/// Completion for when clangd isn't running: language words plus names from the open files.
pub fn suggest(prefix: &str, keywords: &[&'static str], sources: &[String]) -> Vec<Word> {
    let lower = prefix.to_lowercase();
    let wanted = |w: &str| w != prefix && starts_with_ignoring_case(w, &lower);
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for text in sources {
        for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            if !word.starts_with(|c: char| c.is_ascii_digit()) && !word.is_empty() && wanted(word) {
                *seen.entry(word).or_default() += 1;
            }
        }
    }
    for word in keywords.iter().filter(|w| wanted(w)) {
        seen.entry(word).or_default();
    }
    let mut ranked: Vec<(&str, usize)> = seen.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.len().cmp(&b.0.len()))
            .then(a.0.cmp(b.0))
    });
    ranked
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(text, _)| Word {
            text: text.to_string(),
            keyword: keywords.contains(&text),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(words: Vec<Word>) -> Vec<String> {
        words.into_iter().map(|w| w.text).collect()
    }

    #[test]
    fn names_are_ranked_by_how_often_they_appear() {
        let sources = ["counter = count + counter; cou".to_string()];
        assert_eq!(
            texts(suggest("cou", &[], &sources)),
            vec!["counter".to_string(), "count".to_string()]
        );
    }

    #[test]
    fn keywords_are_offered_and_marked() {
        let found = suggest("whi", &["while", "for"], &[]);
        assert_eq!(
            found,
            vec![Word {
                text: "while".into(),
                keyword: true
            }]
        );
    }

    #[test]
    fn matching_ignores_case_but_skips_the_word_being_typed() {
        let sources = ["MyValue myv 1myvalue".to_string()];
        assert_eq!(texts(suggest("myv", &[], &sources)), vec!["MyValue"]);
    }
}
