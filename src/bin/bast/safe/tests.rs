use super::*;

#[test]
fn escapes_terminal_actions_and_preserves_chinese_layout() {
    assert_eq!(
        Safe("中文\n\t\x1b]52;c;clipboard\x07\r\u{009b}2J\0\x7f").to_string(),
        "中文\n\t\\u001b]52;c;clipboard\\u0007\\u000d\\u009b2J\\u0000\\u007f",
    );
}

#[test]
fn split_chunks_cannot_reassemble_control_sequences() {
    let chunks = ["\x1b", "]52;c;", "payload", "\x1b", "\\", "\r", "中文"];
    let output: String = chunks
        .into_iter()
        .map(|chunk| Safe(chunk).to_string())
        .collect();
    assert_eq!(output, "\\u001b]52;c;payload\\u001b\\\\u000d中文");
    assert!(!output.chars().any(char::is_control));
}
