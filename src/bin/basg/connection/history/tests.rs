use super::*;

#[test]
fn history_wire_budget_accepts_boundary_and_rejects_overflow() {
    let mut total = MAX_HISTORY_BYTES - 10;
    account_bytes(&mut total, 10).expect("exact boundary");
    assert!(matches!(
        account_bytes(&mut total, 1),
        Err(Failure::Condition(Condition::HistorySizeLimit))
    ));
    assert!(account_bytes(&mut total, usize::MAX).is_err());
}
