use super::*;
use bas::model::{Cursor, Event, EventKind, Owner};

fn synthetic(first: u64, count: usize, text_bytes: usize) -> PreparedHistory {
    let meta = SessionMeta::new(SessionId::new(), "embedded".into(), Owner::current());
    let events = (first..first + count as u64)
        .map(|cursor| {
            Event::new(
                meta.id,
                Cursor::from_raw(cursor),
                EventKind::Acp(bas::acp::SessionUpdate::AgentMessageChunk(
                    bas::acp::ContentChunk::new("x".repeat(text_bytes).into()),
                )),
            )
        })
        .collect();
    bas::service::HistoryWindow {
        session: meta.into(),
        events,
        context: Vec::new(),
        before: (first > 1).then_some(first),
        cursor: first + count as u64 - 1,
    }
    .into()
}

fn timeline(prepared: PreparedHistory) -> state::TimelineState {
    let mut timeline =
        state::TimelineState::new(ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)));
    timeline.conversation = Some(Conversation::new(prepared.session.meta.clone()));
    timeline.presentation_bytes = prepared.weight.bytes().saturating_mul(2);
    timeline.history_weight = prepared.weight;
    timeline.history = Some(prepared.value);
    timeline
}

#[test]
fn appended_and_prepended_weights_enforce_eviction() {
    let mut cache = Cache::default();
    let mut first = timeline(synthetic(2, 1, 2 * 1024 * 1024));
    let id = first.history.as_ref().expect("history").session.meta.id;
    let initial = first.retained_bytes();
    cache.insert(id, first);
    assert_eq!(cache.bytes, initial);
    first = cache.take(id).expect("cached timeline");
    assert_eq!(cache.bytes, 0);

    let mut incoming = synthetic(3, 1, 3 * 1024 * 1024);
    incoming.value.events[0].session = id;
    let event: crate::connection::Prepared<Event> = incoming.value.events.remove(0).into();
    first
        .history
        .as_mut()
        .expect("history")
        .events
        .push(event.value);
    first.history_weight.append(event.bytes);
    first.presentation_bytes += event.bytes * 2;
    assert!(first.retained_bytes() > initial + 9 * 1024 * 1024);

    let second = timeline(synthetic(1, 1, 5 * 1024 * 1024));
    let second_id = second.history.as_ref().expect("history").session.meta.id;
    cache.insert(second_id, second);
    let mut older = synthetic(1, 1, 1024 * 1024);
    older.value.events[0].session = id;
    first
        .history
        .as_mut()
        .expect("history")
        .events
        .splice(0..0, older.value.events);
    first.history_weight.prepend(older.weight);
    first.presentation_bytes = first.history_weight.bytes() * 2;
    let grown = first.retained_bytes();
    cache.insert(id, first);
    assert!(cache.take(second_id).is_none());
    assert_eq!(cache.bytes, grown);
    assert!(cache.bytes <= HOT_BYTES);

    let oversized = timeline(synthetic(1, 1, 11 * 1024 * 1024));
    let oversized_id = oversized.history.as_ref().expect("history").session.meta.id;
    cache.insert(oversized_id, oversized);
    assert!(cache.take(oversized_id).is_none());
    assert_eq!(cache.bytes, grown);
    cache.remove(id);
    assert_eq!(cache.bytes, 0);
}

#[test]
fn metadata_replacements_are_bounded_without_poll_accumulation() {
    let mut timeline = timeline(synthetic(1, 1, 1024));
    let mut meta = timeline
        .conversation
        .as_ref()
        .expect("conversation")
        .session
        .clone();
    meta.revision = meta.revision.next();
    meta.title = Some("x".repeat(1024 * 1024));
    timeline.merge_meta(&meta);
    let measured = timeline.retained_bytes();
    for _ in 0..100 {
        timeline.merge_meta(&meta);
    }
    assert_eq!(timeline.retained_bytes(), measured);
    assert!(measured >= 1024 * 1024);

    timeline.history_weight.merge_meta(2 * 1024 * 1024);
    let snapshot_budget = timeline.retained_bytes();
    for _ in 0..100 {
        timeline.history_weight.merge_meta(2 * 1024 * 1024);
    }
    assert_eq!(timeline.retained_bytes(), snapshot_budget);

    meta.revision = meta.revision.next();
    meta.title = Some("x".repeat(HOT_BYTES));
    timeline.merge_meta(&meta);
    let mut cache = Cache::default();
    cache.insert(meta.id, timeline);
    assert!(
        cache.take(meta.id).is_none(),
        "out-of-band metadata cannot bypass cache budget"
    );
}

#[test]
#[ignore = "bounded synthetic hot-switch timing; run with --ignored --nocapture"]
fn synthetic_hot_cache_benchmark() {
    use std::{hint::black_box, time::Instant};
    // Setup/ingress is outside both timings. This isolates the switch cache path.
    let mut current = timeline(synthetic(1, 128, 32 * 1024));
    let id = current.history.as_ref().expect("history").session.meta.id;
    let old_iterations = 20;
    let started = Instant::now();
    for _ in 0..old_iterations {
        black_box(
            serde_json::to_vec(black_box(current.history.as_ref().expect("history")))
                .expect("JSON")
                .len()
                .saturating_mul(3),
        );
    }
    let old = started.elapsed();
    let mut cache = Cache::default();
    let new_iterations = 10_000;
    let started = Instant::now();
    for _ in 0..new_iterations {
        cache.insert(black_box(id), current);
        current = black_box(cache.take(id).expect("cached timeline"));
    }
    let new = started.elapsed();
    eprintln!(
        "synthetic hot cache: text_bytes={} retained_bytes={} old_serialization_us={:.3} new_insert_take_us={:.3} old_iterations={old_iterations} new_iterations={new_iterations}",
        4 * 1024 * 1024,
        current.retained_bytes(),
        old.as_secs_f64() * 1_000_000. / f64::from(old_iterations),
        new.as_secs_f64() * 1_000_000. / f64::from(new_iterations),
    );
}
