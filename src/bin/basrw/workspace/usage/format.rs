//! Compact and exact labels for context usage details.

#[cfg(test)]
mod tests;

pub(super) fn short_tokens(count: u64) -> String {
    let (unit, suffix) = if count >= 1_000_000 {
        (1_000_000, "M")
    } else if count >= 1_000 {
        (1_000, "k")
    } else {
        return count.to_string();
    };
    let fractional = count / unit < 10;
    let divisor = if fractional { unit / 10 } else { unit };
    // Round the quotient instead of adding to count, which may be u64::MAX.
    let rounded = count / divisor + u64::from(count % divisor >= divisor / 2);
    if !fractional {
        return format!("{rounded}{suffix}");
    }
    let whole = rounded / 10;
    let tenth = rounded % 10;
    if tenth == 0 {
        format!("{whole}{suffix}")
    } else {
        format!("{whole}.{tenth}{suffix}")
    }
}

pub(super) fn exact_tokens(count: u64) -> String {
    let digits = count.to_string();
    let mut label = String::with_capacity(digits.len() + (digits.len() - 1) / 3);
    for (index, digit) in digits.char_indices() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            label.push(',');
        }
        label.push(digit);
    }
    label
}

pub(super) fn cost_label(cost: &basr::acp_v2::Cost) -> String {
    let precision = if cost.amount > 0.0 && cost.amount < 0.01 {
        4
    } else {
        2
    };
    format!("{:.*} {}", precision, cost.amount, cost.currency)
}
