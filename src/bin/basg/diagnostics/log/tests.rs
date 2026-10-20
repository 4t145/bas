use super::*;
use tracing_subscriber::prelude::*;

#[test]
fn oversized_formatting_stops_at_the_byte_budget() {
    struct Huge(std::cell::Cell<usize>);
    impl fmt::Debug for Huge {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for _ in 0..MAX_RECORD_BYTES * 100 {
                self.0.set(self.0.get() + 1);
                f.write_str("界")?;
            }
            panic!("formatting should have stopped at the byte budget");
        }
    }
    impl fmt::Display for Huge {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(self, f)
        }
    }
    let store = Store::new();
    let huge = Huge(std::cell::Cell::new(0));
    tracing::subscriber::with_default(tracing_subscriber::registry().with(store.layer()), || {
        tracing::info!(message = ?huge);
        assert_eq!(huge.0.replace(0), MAX_RECORD_BYTES / "界".len() + 1);
        tracing::info!(message = %huge);
        assert_eq!(huge.0.get(), MAX_RECORD_BYTES / "界".len() + 1);
    });
    let expected = bounded("界".repeat(MAX_RECORD_BYTES), MAX_RECORD_BYTES);
    for entry in store.snapshot().entries {
        assert_eq!(entry.message, expected);
        assert!(entry.message.ends_with(TRUNCATED));
        assert!(entry.message.len() <= MAX_RECORD_BYTES);
    }
}

#[test]
fn limited_formatting_preserves_short_and_exact_budget_values() {
    let mut fields = Fields::default();
    fields.record_value("message", format_args!("hello {} {:?}", "界", "quoted"));
    fields.record_value("count", format_args!("{}", 42));
    assert_eq!(fields.message, "hello 界 \"quoted\"");
    assert_eq!(fields.other, "count=42");

    let exact = "x".repeat(MAX_RECORD_BYTES);
    fields.record_value("message", format_args!("{exact}"));
    assert_eq!(fields.message, exact);
    let huge = "界".repeat(MAX_RECORD_BYTES);
    fields.record_value("message", format_args!("{huge}"));
    assert_eq!(fields.message, bounded(huge, MAX_RECORD_BYTES));

    let mut output = LimitedText::new(MAX_RECORD_BYTES);
    assert!(output.write_str(&exact).is_ok());
    assert!(output.write_str("界").is_err());
    let truncated = output.text.clone();
    assert!(output.write_str("ignored").is_err());
    assert_eq!(output.text, truncated);
    assert_eq!(output.text, bounded(format!("{exact}界"), MAX_RECORD_BYTES));
}

#[test]
fn plain_text_bounds_output_after_stripping_escapes() {
    let escaped = "\x1b[31m界\x1b[0m\x1b]title\x07\n\t\x00";
    let huge = escaped.repeat(MAX_RECORD_BYTES);
    let expected = bounded("界\n\t".repeat(MAX_RECORD_BYTES), MAX_RECORD_BYTES);
    assert_eq!(plain_text_bounded(&huge, MAX_RECORD_BYTES), expected);
    let exact = "x".repeat(MAX_TARGET_BYTES);
    assert_eq!(
        plain_text_bounded(&format!("{exact}\x1b[0m\x00"), MAX_TARGET_BYTES),
        exact
    );
    let store = Store::new();
    store.record(Level::Info, huge.clone(), huge);
    let entry = store.snapshot().entries.pop().expect("record exists");
    assert_eq!(entry.message, expected);
    assert!(entry.target.len() <= MAX_TARGET_BYTES);
    assert!(entry.target.ends_with(TRUNCATED));
}

#[test]
fn scoped_subscriber_captures_levels_fields_and_plain_text() {
    let store = Store::new();
    let subscriber = tracing_subscriber::registry().with(store.layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "connection", host = "localhost", port = 42, "connecting");
        tracing::warn!(target: "connection", retry = true, "retry");
        tracing::error!(target: "connection", error = %"bad certificate", "failed");
    });
    let snapshot = store.snapshot();
    assert_eq!(snapshot.revision, 3);
    assert_eq!(snapshot.entries.len(), 3);
    for (entry, level) in snapshot
        .entries
        .iter()
        .zip([Level::Info, Level::Warn, Level::Error])
    {
        assert_eq!(entry.level, level);
        assert_eq!(entry.target, "connection");
        assert!(entry.text().contains(level.as_str()));
        assert!(entry.text().contains(&entry.message));
    }
    assert!(snapshot.entries[0].message.contains("host=localhost"));
    assert!(snapshot.entries[0].message.contains("port=42"));
    assert!(snapshot.entries[1].message.contains("retry=true"));
    assert!(snapshot.entries[2].message.contains("bad certificate"));
    store.record(
        Level::Debug,
        "\x1b[31mtest\x1b[0m",
        "\x1b[32mhello\x1b[0m\nworld",
    );
    let entry = store.snapshot().entries.pop().expect("record exists");
    assert_eq!(entry.message, "hello\nworld");
    assert!(!entry.text().contains('\x1b'));
}

#[tokio::test]
async fn concurrent_records_have_ordered_ids_revisions_and_wake_receivers() {
    let store = Store::new();
    let mut updates = store.subscribe();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    store.record(Level::Info, "parallel", "record");
                }
            })
        })
        .collect();
    tokio::time::timeout(std::time::Duration::from_secs(5), updates.changed())
        .await
        .expect("producer wakes receiver")
        .expect("store is alive");
    for thread in threads {
        thread.join().expect("producer succeeds");
    }
    let snapshot = store.snapshot();
    assert_eq!(snapshot.revision, 800);
    assert_eq!(*updates.borrow(), snapshot.revision);
    for (index, entry) in snapshot.entries.iter().enumerate() {
        assert_eq!(entry.id, index as u64 + 1);
    }
    assert_eq!(*store.subscribe().borrow(), 800);
}

#[test]
fn retention_truncation_and_clear_are_bounded() {
    let store = Store::new();
    for _ in 0..MAX_ENTRIES + 7 {
        store.record(Level::Trace, "test", "small");
    }
    let snapshot = store.snapshot();
    assert_eq!(snapshot.entries.len(), MAX_ENTRIES);
    assert_eq!(snapshot.evicted, 7);
    assert_eq!(snapshot.entries[0].id, 8);
    store.clear();
    let oversized = "界".repeat(MAX_RECORD_BYTES);
    for _ in 0..40 {
        store.record(Level::Error, "test", &oversized);
    }
    let snapshot = store.snapshot();
    assert!(snapshot.evicted > 0);
    assert!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.message.len())
            .sum::<usize>()
            <= MAX_MESSAGE_BYTES
    );
    for entry in &snapshot.entries {
        assert!(entry.message.len() <= MAX_RECORD_BYTES);
        assert!(entry.message.ends_with(TRUNCATED));
        assert!(entry.message.starts_with('界'));
    }
    let last_id = snapshot.entries.last().expect("retained entry").id;
    let updates = store.subscribe();
    store.clear();
    let cleared = store.snapshot();
    assert!(cleared.entries.is_empty());
    assert_eq!(cleared.evicted, 0);
    assert_eq!(cleared.revision, snapshot.revision + 1);
    assert!(updates.has_changed().expect("store alive"));
    assert_eq!(store.record(Level::Info, "test", "new"), last_id + 1);
}

#[test]
fn credential_fields_are_redacted_without_formatting_values() {
    struct MustNotFormat;
    impl fmt::Debug for MustNotFormat {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("sensitive field must not be formatted");
        }
    }
    let store = Store::new();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(store.layer()), || {
        tracing::info!(
            authorization = ?MustNotFormat,
            access_token = "hidden-token",
            api_key = "hidden-key",
            password = "hidden-password",
            private_key = "hidden-private",
            certificate_pem = "hidden-cert",
            reason = "useful diagnosis",
            "failure"
        );
    });
    let snapshot = store.snapshot();
    let message = &snapshot.entries[0].message;
    assert!(!message.contains("hidden"));
    assert!(message.contains("failure"));
    assert!(message.contains("reason=useful diagnosis"));
    assert_eq!(message.matches("[redacted]").count(), 6);
}
