use super::*;

#[test]
fn short_token_counts_round_at_unit_and_precision_boundaries() {
    for (count, expected) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1k"),
        (1_049, "1k"),
        (1_050, "1.1k"),
        (1_950, "2k"),
        (9_949, "9.9k"),
        (9_950, "10k"),
        (10_000, "10k"),
        (10_499, "10k"),
        (10_500, "11k"),
        (999_499, "999k"),
        (999_500, "1000k"),
        (1_000_000, "1M"),
        (1_049_999, "1M"),
        (1_050_000, "1.1M"),
        (9_949_999, "9.9M"),
        (9_950_000, "10M"),
        (10_000_000, "10M"),
        (10_499_999, "10M"),
        (10_500_000, "11M"),
        (u64::MAX, "18446744073710M"),
    ] {
        assert_eq!(short_tokens(count), expected, "token count {count}");
    }
}

#[test]
fn exact_token_counts_preserve_every_digit_with_grouping() {
    for (count, expected) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1,000"),
        (12_345, "12,345"),
        (123_456, "123,456"),
        (1_234_567, "1,234,567"),
        (u64::MAX, "18,446,744,073,709,551,615"),
    ] {
        assert_eq!(exact_tokens(count), expected);
    }
}

#[test]
fn small_positive_costs_keep_four_decimal_places_and_the_currency() {
    for (amount, currency, expected) in [
        (0.0, "USD", "0.00 USD"),
        (0.0001, "USD", "0.0001 USD"),
        (0.0099, "EUR", "0.0099 EUR"),
        (0.01, "EUR", "0.01 EUR"),
        (12.34, "CNY", "12.34 CNY"),
    ] {
        assert_eq!(
            cost_label(&basr::acp_v2::Cost::new(amount, currency)),
            expected
        );
    }
}
