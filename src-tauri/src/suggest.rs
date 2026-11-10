use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MIN_TYPED: usize = 3;
const REFRESH: Duration = Duration::from_secs(30);

type Files = Arc<Vec<(PathBuf, String)>>;
static CACHE: Mutex<Option<(String, Instant, Files)>> = Mutex::new(None);

/// Source files of the project, read again at most every 30 seconds.
pub fn project_files(root: &str) -> Files {
    let mut cache = CACHE.lock().unwrap();
    if let Some((r, at, files)) = cache.as_ref() {
        if r == root && at.elapsed() < REFRESH {
            return files.clone();
        }
    }
    let files = Arc::new(if root.is_empty() {
        vec![]
    } else {
        crate::project::source_texts(root)
    });
    *cache = Some((root.to_string(), Instant::now(), files.clone()));
    files
}

/// Text after the caret is fine when it is only the closers the editor added by itself.
pub fn suffix_is_closers(suffix: &str) -> bool {
    suffix
        .chars()
        .all(|c| c.is_whitespace() || ")]}\"';,>".contains(c))
}

fn fit_suffix<'a>(rest: &'a str, suffix: &str) -> Option<&'a str> {
    let closers = suffix.trim_end();
    let kept = if closers.is_empty() {
        rest.trim_end()
    } else {
        &rest[..rest.find(closers)?]
    };
    (kept.trim().chars().count() >= 2 || kept.contains(['{', ';', ')'])).then_some(kept)
}

fn templates(cpp: bool) -> &'static [&'static str] {
    if cpp {
        &[
            "int main() {",
            "for (int i = 0; i < n; i++) {",
            "for (const auto& x : ",
            "#include <iostream>",
            "#include <vector>",
            "#include <string>",
            "std::cout << ",
        ]
    } else {
        &[
            "int main(void) {",
            "for (int i = 0; i < n; i++) {",
            "#include <stdio.h>",
            "#include <stdlib.h>",
            "#include <string.h>",
        ]
    }
}

/// Finishes the line from lines already written, weighted by where they come from.
pub fn complete<'a>(
    prefix: &str,
    suffix: &str,
    lines: impl Iterator<Item = (usize, Cow<'a, str>)>,
    cpp: bool,
) -> Option<String> {
    let typed = prefix.trim_start();
    if typed.chars().count() < MIN_TYPED || typed.starts_with("//") || typed.starts_with("/*") {
        return None;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let owned: Vec<(usize, Cow<'a, str>)> = lines
        .filter(|(_, l)| l.len() > typed.len() && l.trim_start().starts_with(typed))
        .collect();
    for (weight, line) in &owned {
        let rest = &line.trim_start()[typed.len()..];
        if let Some(kept) = fit_suffix(rest, suffix) {
            *seen.entry(kept).or_default() += weight;
        }
    }
    let best = seen.into_iter().max_by(|a, b| {
        a.1.cmp(&b.1)
            .then(b.0.len().cmp(&a.0.len()))
            .then(b.0.cmp(a.0))
    });
    if let Some((text, _)) = best {
        return Some(text.to_string());
    }
    templates(cpp)
        .iter()
        .filter(|t| t.len() > typed.len() && t.starts_with(typed))
        .find_map(|t| fit_suffix(&t[typed.len()..], suffix))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(list: &[(usize, &'static str)]) -> std::vec::IntoIter<(usize, Cow<'static, str>)> {
        let owned: Vec<_> = list.iter().map(|&(w, l)| (w, Cow::Borrowed(l))).collect();
        owned.into_iter()
    }

    #[test]
    fn a_repeated_line_is_finished_and_the_auto_closed_part_is_kept() {
        let src = [(3, "    printf(\"inserisci i numeri: \");")];
        let got = complete("    printf(\"inserisci i nu", "\")", lines(&src), false);
        assert_eq!(got.as_deref(), Some("meri: "));
        let got = complete("printf(\"inserisci i nu", "", lines(&src), false);
        assert_eq!(got.as_deref(), Some("meri: \");"));
    }

    #[test]
    fn the_most_common_continuation_wins() {
        let src = [
            (1, "int total = a + b;"),
            (1, "int total = a + c;"),
            (2, "int total = a + c;"),
        ];
        let got = complete("int total = a + ", "", lines(&src), false);
        assert_eq!(got.as_deref(), Some("c;"));
    }

    #[test]
    fn short_prefixes_comments_and_unrelated_lines_give_nothing() {
        let src = [(1, "return 0;")];
        assert_eq!(complete("re", "", lines(&src), false), None);
        assert_eq!(complete("// return", "", lines(&src), false), None);
        assert_eq!(complete("while (x", "", lines(&src), false), None);
    }

    #[test]
    fn text_after_the_caret_must_be_closers() {
        assert!(suffix_is_closers("\")"));
        assert!(suffix_is_closers(""));
        assert!(!suffix_is_closers("foo(x)"));
    }

    #[test]
    fn templates_fill_in_when_nothing_was_written_before() {
        let none = std::iter::empty();
        assert_eq!(
            complete("int main(", "", none, false).as_deref(),
            Some("void) {")
        );
        let none = std::iter::empty();
        assert_eq!(
            complete("#include <io", "", none, true).as_deref(),
            Some("stream>")
        );
        let none = std::iter::empty();
        assert_eq!(
            complete("for (int i", "", none, false).as_deref(),
            Some(" = 0; i < n; i++) {")
        );
    }
}
