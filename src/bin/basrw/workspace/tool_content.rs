//! Tool output previews derived from the data supplied by the agent.

use basr::acp;

#[cfg(test)]
#[path = "tool_content/tests.rs"]
mod tests;

pub(super) fn legacy_diff(diff: &acp::Diff) -> String {
    let path = diff.path.to_string_lossy();
    match &diff.old_text {
        Some(old_text) => {
            let patch = similar::TextDiff::configure()
                .timeout(std::time::Duration::from_millis(100))
                .diff_lines(old_text, &diff.new_text)
                .unified_diff()
                .context_radius(3)
                .header(&path, &path)
                .to_string();
            if patch.is_empty() {
                format!("{path}\n\n{}", crate::i18n::tr!("diff-unchanged"))
            } else {
                format!("{path}\n\n{}", code_block("diff", &patch))
            }
        }
        // Markdown fences must stay outside Fluent's bidi-isolated placeables.
        None => format!(
            "{path}\n\n{}\n\n{}",
            crate::i18n::tr!("diff-after"),
            code_block("text", &diff.new_text)
        ),
    }
}

fn code_block(language: &str, text: &str) -> String {
    let fence = "~".repeat(
        text.split(|ch| ch != '~')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}{language}\n{text}\n{fence}")
}
