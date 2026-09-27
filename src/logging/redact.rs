//! Keep console output free of credentials. Messages in these services can
//! contain URLs, session cookies and tokens, so anything that looks like a
//! secret is replaced before it is written.

const MAX_WORDS: usize = 28;
const MAX_WORD_LEN: usize = 24;

const SENSITIVE_MARKERS: &[&str] = &[
    "sessionid",
    "session_id",
    "dosid",
    "sid=",
    "token",
    "password",
    "passwd",
    "secret",
    "cookie",
    "authorization",
    "api_key",
    "apikey",
    "bearer",
];

/// Keep the failure class of a message while removing URLs and long opaque
/// values before printing anything.
pub(crate) fn redact(input: &str) -> String {
    let mut safe = String::with_capacity(input.len());
    for word in input.split_whitespace().take(MAX_WORDS) {
        if !safe.is_empty() {
            safe.push(' ');
        }
        safe.push_str(if sensitive(word) { "<redacted>" } else { word });
    }
    safe
}

fn sensitive(word: &str) -> bool {
    // Field values rendered with `{:?}` arrive wrapped in quotes; look past
    // them so a long quoted value is still recognised.
    let bare = word.trim_matches('"');
    if bare.contains("://") || bare.len() > MAX_WORD_LEN {
        return true;
    }
    let lower = bare.to_ascii_lowercase();
    SENSITIVE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_pass_through() {
        assert_eq!(redact("socket connect failed"), "socket connect failed");
    }

    #[test]
    fn urls_are_redacted() {
        assert_eq!(
            redact("GET https://int1.darkorbit.com/api failed"),
            "GET <redacted> failed"
        );
    }

    #[test]
    fn credentials_are_redacted() {
        assert_eq!(redact("dosid=abc123"), "<redacted>");
        assert_eq!(redact("sessionId: a1b2"), "<redacted> a1b2");
        assert_eq!(
            redact("Authorization: Bearer xyz"),
            "<redacted> <redacted> xyz"
        );
        assert_eq!(redact("cookie: dosid=xyz"), "<redacted> <redacted>");
    }

    #[test]
    fn long_opaque_values_are_redacted() {
        assert_eq!(redact("id abcdefghijklmnopqrstuvwxy"), "id <redacted>");
    }

    #[test]
    fn output_is_bounded() {
        let long = "word ".repeat(100);
        assert_eq!(redact(&long).split_whitespace().count(), MAX_WORDS);
    }
}
