//! Keep telemetry free of credentials. The same [`Redactor`] backs the
//! console layer (before anything is printed) and the OTLP exporters
//! (before anything reaches the wire): URLs, session ids, cookies and
//! tokens are replaced by [`REDACTED`].
//!
//! Default rules cover the platform's secrets (docs/06 §6.5, docs/09 §9.1):
//! passwords, `sid`/`dosid` session ids, tokens and `Authorization`
//! headers. Services extend the deny-list through the `with_*` builders
//! and [`crate::logging::Config::with_redactor`].

use std::borrow::Cow;

const MAX_WORDS: usize = 28;
const MAX_WORD_LEN: usize = 24;

/// The value written in place of anything sensitive.
pub const REDACTED: &str = "<redacted>";

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

/// Key fragments marking a whole attribute/log field as secret no matter
/// the value (`http.request.header.authorization`, `log.dosid`,
/// `panel_token`). Matched case-insensitively as a substring of the key.
const SENSITIVE_KEY_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "dosid",
    "session",
    "token",
    "secret",
    "cookie",
    "authorization",
    "api_key",
    "apikey",
    "credential",
    "bearer",
];

/// Short segments that only count as sensitive when they appear as a whole
/// `.`/`_`/`-`/`:`-separated key part, so `grid` or `inside` survive.
const SENSITIVE_KEY_SEGMENTS: &[&str] = &["sid"];

/// Credential scrubbing shared by the console layer and the OTLP export
/// path.
///
/// * [`Redactor::text`] — free-text scrub for log bodies/messages (URLs,
///   opaque tokens and marker-bearing words become `<redacted>`).
/// * [`Redactor::is_sensitive_key`] — attribute keys that are secret by
///   name; their values are masked in full.
/// * [`Redactor::value`] — value scrub with the attribute key for context.
#[derive(Clone, Debug)]
pub struct Redactor {
    key_markers: Vec<Cow<'static, str>>,
    key_segments: Vec<Cow<'static, str>>,
    text_markers: Vec<Cow<'static, str>>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self {
            key_markers: SENSITIVE_KEY_MARKERS
                .iter()
                .map(|marker| Cow::Borrowed(*marker))
                .collect(),
            key_segments: SENSITIVE_KEY_SEGMENTS
                .iter()
                .map(|marker| Cow::Borrowed(*marker))
                .collect(),
            text_markers: SENSITIVE_MARKERS
                .iter()
                .map(|marker| Cow::Borrowed(*marker))
                .collect(),
        }
    }
}

impl Redactor {
    /// Extend the always-secret key list with `marker` — a
    /// case-insensitive substring match on the key, e.g. `"vault_token"`.
    /// Separators are normalized like they are on keys.
    pub fn with_key_marker(mut self, marker: impl Into<Cow<'static, str>>) -> Self {
        let marker: String = lower(marker.into())
            .chars()
            .map(|c| if matches!(c, '.' | '-' | ':' | ' ') { '_' } else { c })
            .collect();
        self.key_markers.push(Cow::Owned(marker));
        self
    }

    /// Treat `segment` as secret when it is a whole key part — for short
    /// ids such as `sid` that would over-match as a substring.
    pub fn with_key_segment(mut self, segment: impl Into<Cow<'static, str>>) -> Self {
        self.key_segments.push(lower(segment.into()));
        self
    }

    /// Extra word marker for free-text redaction (log bodies, messages).
    pub fn with_marker(mut self, marker: impl Into<Cow<'static, str>>) -> Self {
        self.text_markers.push(lower(marker.into()));
        self
    }

    /// `true` when `key` names a credential whose value must never be
    /// exported (`password`, `dosid`, `panel_token`, `http…authorization`,
    /// a bare `sid` segment, …). `.`/`_`/`-`/`:` separators are normalized
    /// before matching so `api-key` and `api_key` are the same.
    pub fn is_sensitive_key(&self, key: &str) -> bool {
        let lower = key.to_ascii_lowercase();
        let normalized: String = lower
            .chars()
            .map(|c| if matches!(c, '.' | '_' | '-' | ':' | ' ') { '_' } else { c })
            .collect();
        if self
            .key_markers
            .iter()
            .any(|marker| normalized.contains(marker.as_ref()))
        {
            return true;
        }
        normalized
            .split('_')
            .any(|part| self.key_segments.iter().any(|seg| seg.as_ref() == part))
    }

    /// Keep the failure class of a free-text value while removing URLs and
    /// long opaque values.
    pub fn text(&self, input: &str) -> String {
        let mut safe = String::with_capacity(input.len());
        for word in input.split_whitespace().take(MAX_WORDS) {
            if !safe.is_empty() {
                safe.push(' ');
            }
            safe.push_str(if self.sensitive(word) { REDACTED } else { word });
        }
        safe
    }

    /// Redact the value of the attribute `key`; sensitive keys are masked
    /// in full, everything else goes through [`Redactor::text`].
    pub fn value(&self, key: &str, value: &str) -> String {
        if self.is_sensitive_key(key) {
            REDACTED.into()
        } else {
            self.text(value)
        }
    }

    fn sensitive(&self, word: &str) -> bool {
        // Field values rendered with `{:?}` arrive wrapped in quotes; look
        // past them so a long quoted value is still recognised.
        let bare = word.trim_matches('"');
        if bare.contains("://") || bare.len() > MAX_WORD_LEN {
            return true;
        }
        let lower = bare.to_ascii_lowercase();
        self.text_markers
            .iter()
            .any(|marker| lower.contains(marker.as_ref()))
    }
}

fn lower(value: Cow<'static, str>) -> Cow<'static, str> {
    if value.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(value.to_ascii_lowercase())
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(input: &str) -> String {
        Redactor::default().text(input)
    }

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

    #[test]
    fn sensitive_keys_match_on_case_and_segments() {
        let redactor = Redactor::default();
        for key in [
            "password",
            "Password",
            "dosid",
            "panel_token",
            "http.request.header.authorization",
            "instance_token",
            "session",
            "user_sid",
            "xenomit.proxy.password",
            "api-key",
        ] {
            assert!(redactor.is_sensitive_key(key), "{key} must be secret");
        }
        for key in [
            "bot.id",
            "inside",
            "grid",
            "outcome",
            "phase",
            "service.instance.id",
        ] {
            assert!(!redactor.is_sensitive_key(key), "{key} must survive");
        }
    }

    #[test]
    fn custom_rules_extend_the_deny_list() {
        let redactor = Redactor::default()
            .with_key_marker("vault_key")
            .with_key_segment("otp")
            .with_marker("hunters2");
        assert!(redactor.is_sensitive_key("my_vault_key"));
        assert!(redactor.is_sensitive_key("login.otp"));
        assert!(!redactor.is_sensitive_key("otpcode"));
        assert_eq!(redactor.text("found hunters2 here"), "found <redacted> here");
    }
}
