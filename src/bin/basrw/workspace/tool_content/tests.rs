use super::*;

#[test]
fn supplied_before_and_after_texts_show_actual_edits() {
    let preview = legacy_diff(
        &acp::Diff::new("/work/config.rs", "unchanged\nnew value\n")
            .old_text("unchanged\nold value\n"),
    );
    assert!(preview.contains("@@ -1,2 +1,2 @@"));
    assert!(preview.contains(" unchanged\n-old value\n+new value\n"));
    assert!(preview.contains("--- /work/config.rs\n+++ /work/config.rs"));
}

#[test]
fn missing_before_text_does_not_invent_an_added_file() {
    let preview = legacy_diff(&acp::Diff::new("/work/config.rs", "~~~\nvalue\n~~~"));
    assert!(preview.contains("修改后："));
    assert!(preview.contains("\n\n~~~~text\n~~~\nvalue\n~~~\n~~~~"));
    assert!(
        preview.ends_with("\n~~~~"),
        "the closing fence occupies its own line"
    );
    assert!(!preview.contains("+++"));
    assert!(!preview.contains("新增"));
}

#[test]
fn identical_texts_do_not_show_full_file_as_a_change() {
    let preview = legacy_diff(&acp::Diff::new("/work/config.rs", "kept").old_text("kept"));
    assert!(preview.contains("文件内容相同"));
    assert!(!preview.contains("@@"));
}
