use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::sync::Mutex;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::{Context, Layer};

use super::redact::Redactor;

/// Console output in the style of `xenomit-client`'s diagnostics: one line per
/// event on stderr, message and fields redacted, and a cap per
/// `(target, level)` pair so a noisy dependency cannot flood the terminal.
pub(crate) struct Console<W> {
    service: Box<str>,
    writer: W,
    event_cap: Option<usize>,
    redactor: Redactor,
    /// Count of emitted events per `(target, level)` for rate limiting.
    emitted: Mutex<HashMap<(Box<str>, Level), usize>>,
}

pub(crate) fn layer<W>(
    service: &str,
    writer: W,
    event_cap: Option<usize>,
    redactor: Redactor,
) -> Console<W> {
    Console {
        service: service.into(),
        writer,
        event_cap,
        redactor,
        emitted: Mutex::new(HashMap::new()),
    }
}

impl<S, W> Layer<S> for Console<W>
where
    S: Subscriber,
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let level = *metadata.level();

        if !self.allow(metadata.target(), level) {
            return;
        }

        let mut fields = Fields::default();
        event.record(&mut fields);

        let mut line = String::with_capacity(128);
        let _ = fmt::Write::write_fmt(
            &mut line,
            format_args!("[{}] {} {}:", self.service, level, metadata.target()),
        );
        if let Some(message) = &fields.message {
            line.push(' ');
            line.push_str(&self.redactor.text(message));
        }
        for (key, value) in &fields.fields {
            let _ = fmt::Write::write_fmt(
                &mut line,
                format_args!(" {key}={}", self.redactor.value(key, value)),
            );
        }
        line.push('\n');

        let _ = self.writer.make_writer().write_all(line.as_bytes());
    }
}

impl<W> Console<W>
where
    W: for<'writer> MakeWriter<'writer>,
{
    /// Returns `false` once the cap for this `(target, level)` is reached.
    /// Prints a single notice when suppression begins.
    fn allow(&self, target: &str, level: Level) -> bool {
        let Some(cap) = self.event_cap else {
            return true;
        };
        let mut emitted = self.emitted.lock().unwrap_or_else(|p| p.into_inner());
        let count = emitted.entry((target.into(), level)).or_insert(0);
        *count += 1;
        if *count <= cap {
            return true;
        }
        if *count == cap + 1 {
            let notice = format!(
                "[{}] {} {}: suppressing further events (cap of {cap} reached)\n",
                self.service, level, target
            );
            let _ = self.writer.make_writer().write_all(notice.as_bytes());
        }
        false
    }
}

/// Extracts the `message` field as the log body and keeps the remaining
/// fields as `key=value` pairs, mirroring how the appender bridge maps them
/// onto OpenTelemetry log attributes.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl Fields {
    fn push(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().into(), value));
        }
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field, value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    use super::*;

    /// Shared buffer that satisfies `MakeWriter` for tests.
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buf {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Buf {
        fn output(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn with_console(cap: Option<usize>) -> (impl SubscriberInitExt, Buf) {
        let buf = Buf::default();
        let writer = buf.clone();
        let registry = tracing_subscriber::registry().with(layer(
            "test",
            move || writer.clone(),
            cap,
            Redactor::default(),
        ));
        (registry, buf)
    }

    #[test]
    fn prints_level_target_and_fields() {
        let (subscriber, buf) = with_console(None);
        let _guard = subscriber.set_default();

        tracing::info!(port = 8080, "listening");

        let out = buf.output();
        assert!(out.contains("[test] INFO"), "{out}");
        assert!(out.contains("listening"), "{out}");
        assert!(out.contains("port=8080"), "{out}");
    }

    #[test]
    fn secrets_are_redacted() {
        let (subscriber, buf) = with_console(None);
        let _guard = subscriber.set_default();

        tracing::warn!(dosid = "supersecretvalue", "login failed");

        let out = buf.output();
        assert!(out.contains("<redacted>"), "{out}");
        assert!(!out.contains("supersecretvalue"), "{out}");
    }

    #[test]
    fn caps_suppress_repeated_events() {
        let (subscriber, buf) = with_console(Some(2));
        let _guard = subscriber.set_default();

        for _ in 0..5 {
            tracing::warn!("stub called");
        }

        let out = buf.output();
        assert_eq!(out.matches("stub called").count(), 2, "{out}");
        assert!(out.contains("suppressing further events"), "{out}");
    }
}
