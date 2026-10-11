use super::*;
use gpui_kit::{Bounds, TestAppContext, size, test::TestWindowExt};
use std::sync::{Arc, Mutex};

type CodeRegions = Arc<Mutex<HashMap<&'static str, Bounds<Pixels>>>>;

#[test]
fn upstream_text_style_preserves_component_theme() {
    let mut app = gpui_kit::TestApp::new();
    app.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
        let style = markdown_style(cx);
        assert_eq!(style.foreground(), cx.theme().foreground);
        assert_eq!(style.selection(), cx.theme().selection);
        assert_eq!(style.link(), cx.theme().link);
        assert_eq!(style.border(), cx.theme().border);
        assert_eq!(style.code_background(), cx.theme().muted);
        assert_eq!(style.is_dark(), cx.theme().is_dark());
        assert_eq!(
            style.table_head().background,
            Some(cx.theme().table_head.into()),
        );
        let radius = cx.theme().semantic_tokens().radius.md;
        assert_eq!(style.table().corner_radii.top_left, Some(radius.into()));
        assert!(gpui_kit::base::TextViewDefaults::global(cx).has_code_block_highlighter());
    });
}

struct StructuredMarkdown {
    state: Entity<TextViewState>,
    code_regions: CodeRegions,
}

impl Render for StructuredMarkdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let regions = Arc::clone(&self.code_regions);
        div()
            .size_full()
            .p_3()
            .bg(theme::content_background(cx))
            .child(
                div()
                    .id("structured-markdown")
                    .test_support()
                    .w_full()
                    .overflow_hidden()
                    .child(text_view(&self.state, true, cx).code_block_actions(
                        move |block, _, _| {
                            let id = if block.code().contains("nested-code") {
                                "nested-code-probe"
                            } else {
                                "root-code-probe"
                            };
                            let regions = Arc::clone(&regions);
                            div()
                                .on_children_prepainted(move |_, window, _| {
                                    // Code actions share the code block's ancestor clipping region.
                                    // Their own absolute bounds only locate the top-right toolbar.
                                    regions
                                        .lock()
                                        .expect("code regions")
                                        .insert(id, window.content_mask().bounds);
                                })
                                .id(id)
                                .test_support()
                                .size(px(1.))
                        },
                    )),
            )
    }
}

#[gpui_kit::test]
fn structured_markdown_keeps_nested_blocks_and_copies_a_complete_table(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
    });
    let state = cx.update(|cx| {
        cx.new(|cx| TextViewState::markdown(include_str!("assets/structured.md"), cx))
    });
    let regions = CodeRegions::default();
    let window = cx.open_window(size(px(840.), px(900.)), |_, _| StructuredMarkdown {
        state: state.clone(),
        code_regions: Arc::clone(&regions),
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        for width in [840., 420.] {
            window.resize(size(px(width), px(900.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            let document = window.find("structured-markdown").bounds();
            let nested = window.find("nested-code-probe").bounds();
            let root = window.find("root-code-probe").bounds();
            let copy = window.find("copy-markdown-table").bounds();
            let regions = regions.lock().expect("code regions");
            let nested_region = regions["nested-code-probe"];
            let root_region = regions["root-code-probe"];
            assert!(
                nested_region.left() > root_region.left(),
                "nested code retains list indentation: nested={nested_region:?}, root={root_region:?}, document={document:?}"
            );
            assert!(nested_region.size.width > px(100.), "nested code has usable width");
            assert!(nested_region.right() <= document.right());
            assert!(root_region.right() <= document.right());
            assert!(nested.right() <= document.right());
            assert!(root.right() <= document.right());
            assert!(
                nested.bottom() < root.top(),
                "code blocks retain document order"
            );
            assert!(
                copy.top() > root.bottom(),
                "table follows the root code block"
            );
            assert!(
                copy.right() <= document.right(),
                "table controls stay within the document"
            );
        }
        state.update(cx, |state, cx| {
            state.select_all(cx);
            let selected = state.selected_text();
            for text in [
                "一级条目",
                "列表内的第二段文字",
                "二级条目",
                "三级条目",
                "nested-code",
                "root-code",
                "模型",
                "a|b",
            ] {
                assert!(
                    selected.contains(text),
                    "selected output is missing {text:?}"
                );
            }
            assert!(selected.find("nested-code") < selected.find("root-code"));
        });
        window.click("copy-markdown-table", cx);
        let copied = cx
            .read_from_clipboard()
            .expect("copied table")
            .text()
            .expect("Markdown table");
        assert!(copied.starts_with("| 项目 | 说明 |\n"));
        assert!(
            copied.contains("| :-- | --: |"),
            "column alignments survive copying"
        );
        assert!(copied.contains("`模型`"), "inline code survives copying");
        assert!(copied.contains("**内容**"), "emphasis survives copying");
        assert!(
            copied.contains("a\\|b"),
            "pipes must not create an extra column"
        );
        assert_eq!(copied.lines().count(), 4);
    })
    .expect("Markdown window");
}

struct CopyDocuments {
    states: [Entity<TextViewState>; 2],
}

impl Render for CopyDocuments {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_3().flex().flex_col().gap_3().children(
            self.states.iter().enumerate().map(|(index, state)| {
                div()
                    .id(format!("copy-document-{index}"))
                    .test_support()
                    .w_full()
                    .child(view(state, cx))
            }),
        )
    }
}

#[gpui_kit::test]
fn code_and_table_copy_actions_follow_document_hover_and_keep_copy_content(
    cx: &mut TestAppContext,
) {
    const CODE: &str = "printf '中文'\n  echo ready";
    let markdown = format!(
        "Example\n\n```bash\n{CODE}\n```\n\n| Name | Value |\n| :--- | ---: |\n| `code` | **text** |"
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init_default(cx);
    });
    let states = cx.update(|cx| {
        [
            cx.new(|cx| TextViewState::markdown(&markdown, cx)),
            cx.new(|cx| TextViewState::markdown(&markdown, cx)),
        ]
    });
    let window = cx.open_window(size(px(840.), px(900.)), |_, _| CopyDocuments {
        states: states.clone(),
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for index in 0..2 {
            let document = format!("copy-document-{index}");
            for action in ["copy-markdown-code", "copy-markdown-table"] {
                assert!(!window.within(document.clone()).find(action).visible());
            }
        }
        for hovered in 0..2 {
            let document = format!("copy-document-{hovered}");
            window.hover(document.clone(), cx);
            for index in 0..2 {
                for action in ["copy-markdown-code", "copy-markdown-table"] {
                    assert_eq!(
                        window
                            .within(format!("copy-document-{index}"))
                            .find(action)
                            .visible(),
                        index == hovered,
                        "hover stays scoped to its document"
                    );
                }
            }
            let height = window.find(document.clone()).bounds().size.height;
            window
                .within(document.clone())
                .click("copy-markdown-code", cx);
            assert_eq!(
                cx.read_from_clipboard()
                    .expect("copied code")
                    .text()
                    .as_deref(),
                Some(CODE)
            );
            window
                .within(document.clone())
                .click("copy-markdown-table", cx);
            assert_eq!(
                cx.read_from_clipboard()
                    .expect("copied table")
                    .text()
                    .as_deref(),
                Some("| Name | Value |\n| :-- | --: |\n| `code` | **text** |")
            );
            window.simulate_mouse_move(gpui_kit::point(px(0.), px(0.)), cx);
            window.render_frame(cx);
            assert_eq!(window.find(document.clone()).bounds().size.height, height);
            for action in ["copy-markdown-code", "copy-markdown-table"] {
                assert!(!window.within(document.clone()).find(action).visible());
            }
        }
        states[0].update(cx, |state, cx| {
            state.select_all(cx);
            let selected = state.selected_text();
            assert!(selected.contains(CODE));
            assert!(selected.contains("Name") && selected.contains("text"));
        });
    })
    .expect("hover-only Markdown actions");
}
