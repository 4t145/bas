use super::*;

/// # Errors
/// Returns an error if the first event and its envelope exceed the page budget.
fn history_page(
    events: impl ExactSizeIterator<Item = pb::Event>,
    has_more: bool,
    budget: usize,
) -> Result<pb::LoadHistoryResponse, Status> {
    let count = events.len();
    let mut encoder = PageEncoder::new(budget);
    for (index, event) in events.enumerate() {
        if !encoder.push(event, index + 1 < count || has_more)? {
            break;
        }
    }
    Ok(encoder.finish())
}

fn event(cursor: u64, size: usize) -> pb::Event {
    pb::Event {
        cursor,
        kind: Some(pb::event::Kind::Failed(pb::Failed {
            message: "x".repeat(size),
        })),
        ..Default::default()
    }
}

#[test]
fn legal_large_events_split_without_gaps_and_resume_in_order() {
    let events = vec![event(127, 128), event(128, 128)];
    let budget = pb::LoadHistoryResponse {
        events: vec![events[0].clone()],
        next: Some(127),
    }
    .encoded_len();
    let first = history_page(events.clone().into_iter(), false, budget)
        .expect("budget fits the first event with its continuation cursor");
    assert_eq!(first.events, events[..1]);
    assert_eq!(first.next, Some(127));
    assert_eq!(first.encode_to_vec().len(), budget);
    let second = history_page(events.into_iter().skip(1), false, budget)
        .expect("budget fits the remaining event without a continuation cursor");
    assert_eq!(second.events[0].cursor, 128);
    assert_eq!(second.next, None);
}

#[test]
fn oversized_later_event_does_not_hide_preceding_events_or_get_skipped() {
    let events = vec![event(1, 5), event(2, 200)];
    let first = history_page(events.clone().into_iter(), false, 100)
        .expect("the small first event fits even when the second event is oversized");
    assert_eq!(first.events, events[..1]);
    assert_eq!(first.next, Some(1));
    assert_eq!(
        history_page(events.into_iter().skip(1), false, 100)
            .expect_err("the oversized event must fail rather than be skipped")
            .code(),
        tonic::Code::ResourceExhausted,
    );
    assert_eq!(
        history_page(Vec::new().into_iter(), false, 0)
            .expect("an empty response fits a zero-byte budget")
            .next,
        None
    );
}
