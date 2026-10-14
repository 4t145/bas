use super::*;
use bas::acp;
use gpui_kit::AnyWindowHandle;
use std::time::Duration;

fn update_usage(
    fixture: &Fixture,
    session: SessionId,
    cursor: u64,
    usage: acp::UsageUpdate,
    cx: &mut TestAppContext,
) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.update(
                Update::Event {
                    epoch: view.selected.epoch,
                    event: Event::new(
                        session,
                        Cursor::from_raw(cursor),
                        EventKind::Acp(acp::SessionUpdate::UsageUpdate(usage)),
                    ),
                },
                window,
                cx,
            );
        });
    })
    .expect("usage event ingress");
}

fn prepare_draft(fixture: &Fixture, compact: bool, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.composer.input.update(cx, |input, cx| {
                input.set_value("保留的草稿", window, cx);
                input.focus(window, cx);
            });
        });
        if compact {
            window.press("ctrl-shift-m", cx);
            window.bounds_changed(cx);
        }
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .expect("prepare composer draft");
}

fn assert_ring_in_toolbar(window: &mut Window, session: SessionId) {
    let usage = window.within("composer-toolbar").find("session-usage");
    assert_eq!(
        usage.bounds().size,
        size(px(layout::ICON_BUTTON_SIZE), px(layout::ICON_BUTTON_SIZE)),
    );
    let ring = window.within("composer-toolbar").find("usage-ring");
    assert_eq!(ring.bounds().size, size(px(16.), px(16.)));
    let button = window.find(format!("context-usage-{session}"));
    assert_eq!(button.role(), Some(gpui_kit::Role::Button));
    assert!(
        button
            .label()
            .map(crate::i18n::visible_text)
            .as_deref()
            .expect("usage accessible name")
            .contains("查看详情")
    );
}

fn open_details(fixture: &Fixture, cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("session-usage", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        fixture
            .view
            .read(cx)
            .window
            .menu
            .as_ref()
            .and_then(|menu| menu.window)
            .unwrap_or_else(|| fixture.window.into())
    })
    .expect("open usage details")
}

fn assert_labels(window: &Window, labels: &[(&'static str, &str)]) {
    for (id, expected) in labels {
        assert_eq!(
            window
                .find(*id)
                .label()
                .map(crate::i18n::visible_text)
                .as_deref(),
            Some(*expected),
            "{id}"
        );
    }
}

fn assert_draft_preserved(fixture: &Fixture, window: &mut Window, cx: &mut App) {
    window.render_frame(cx);
    let workspace = fixture.view.read(cx);
    assert_eq!(workspace.composer.input.read(cx).value(), "保留的草稿");
    assert!(
        workspace
            .composer
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    );
}

#[gpui_kit::test]
fn usage_details_update_live_in_the_full_composer(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    prepare_draft(&fixture, false, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("session-usage").is_none());
    })
    .expect("no indicator before usage arrives");
    update_usage(
        &fixture,
        meta.id,
        1,
        acp::UsageUpdate::new(12_000, 100_000).cost(acp::Cost::new(0.05, "USD")),
        cx,
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_ring_in_toolbar(window, meta.id);
        assert!(fixture.view.read(cx).window.usage_details.is_none());
        window.hover("session-usage", cx);
    })
    .expect("full usage control");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window
                .find("usage-hint")
                .label()
                .map(crate::i18n::visible_text)
                .as_deref(),
            Some("上下文用量 12% · 12k / 100k · 查看详情")
        );
    })
    .expect("hover preview is available before opening details");
    let details = open_details(&fixture, cx);
    assert_eq!(details, fixture.window.into());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(details, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).window.usage_details, Some(meta.id));
        assert!(
            window.try_find("usage-hint").is_none(),
            "hover preview must not cover details while the pointer remains on the ring"
        );
        assert!(
            window.find("popup-menu").bounds().bottom()
                <= window.find("session-usage").bounds().top(),
            "details open above the ring so the trigger remains clickable"
        );
        assert_labels(
            window,
            &[
                ("usage-summary", "12% · 12k / 100k"),
                ("usage-used", "已用词元 · 12,000"),
                ("usage-capacity", "上下文容量 · 100,000"),
                ("usage-remaining", "剩余词元 · 88,000"),
                ("usage-cost", "会话累计费用 · 0.05 USD"),
            ],
        );
        window.click("usage-details", cx);
        window.render_frame(cx);
        assert!(window.try_find("usage-details").is_some());
        assert_eq!(
            window.find("popup-menu").focused(),
            Some(true),
            "clicking read-only details keeps Escape available"
        );
    })
    .expect("clicking details leaves the panel open");
    update_usage(
        &fixture,
        meta.id,
        2,
        acp::UsageUpdate::new(85_000, 100_000).cost(acp::Cost::new(0.10, "USD")),
        cx,
    );
    cx.update_window(details, |_, window, cx| {
        window.render_frame(cx);
        assert_labels(
            window,
            &[
                ("usage-summary", "85% · 85k / 100k"),
                ("usage-used", "已用词元 · 85,000"),
                ("usage-remaining", "剩余词元 · 15,000"),
                ("usage-cost", "会话累计费用 · 0.10 USD"),
            ],
        );
        window.press("escape", cx);
    })
    .expect("live full usage and Escape action");
    cx.update_window(details, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("usage-details").is_none());
        assert!(fixture.view.read(cx).window.usage_details.is_none());
        assert_draft_preserved(&fixture, window, cx);
    })
    .expect("live full usage and Escape dismissal");
    open_details(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click("composer-editor", cx);
    })
    .expect("outside click action");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("usage-details").is_none());
        assert!(fixture.view.read(cx).window.usage_details.is_none());
        assert_draft_preserved(&fixture, window, cx);
    })
    .expect("outside click restores editing");
}

#[gpui_kit::test]
fn compact_usage_popup_updates_without_clipping_or_changing_the_draft(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    prepare_draft(&fixture, true, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("session-usage").is_none());
    })
    .expect("compact mode also waits for real usage");
    update_usage(
        &fixture,
        meta.id,
        1,
        acp::UsageUpdate::new(12_000, 100_000).cost(acp::Cost::new(0.005, "EUR")),
        cx,
    );
    let compact_size = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_ring_in_toolbar(window, meta.id);
            window.viewport_size()
        })
        .expect("compact usage control");
    let popup = open_details(&fixture, cx);
    assert_ne!(
        popup,
        fixture.window.into(),
        "compact details use their own surface"
    );
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        let viewport = window.viewport_size();
        for id in [
            "usage-details",
            "usage-summary",
            "usage-used",
            "usage-capacity",
            "usage-remaining",
            "usage-cost",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.top() >= px(0.) && bounds.bottom() <= viewport.height,
                "{id}: {bounds:?}"
            );
            assert!(
                bounds.left() >= px(0.) && bounds.right() <= viewport.width,
                "{id}: {bounds:?}"
            );
        }
        assert_eq!(
            window
                .find("usage-cost")
                .label()
                .map(crate::i18n::visible_text)
                .as_deref(),
            Some("会话累计费用 · 0.0050 EUR")
        );
    })
    .expect("compact popup fits all details");
    update_usage(
        &fixture,
        meta.id,
        2,
        acp::UsageUpdate::new(50_000, 100_000),
        cx,
    );
    cx.update_window(popup, |_, window, cx| {
        window.render_frame(cx);
        assert_labels(
            window,
            &[
                ("usage-summary", "50% · 50k / 100k"),
                ("usage-remaining", "剩余词元 · 50,000"),
            ],
        );
        assert!(window.try_find("usage-cost").is_none());
        window.press("escape", cx);
    })
    .expect("native popup tracks incoming usage");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        assert!(fixture.view.read(cx).window.menu.is_none());
        assert!(!cx.windows().contains(&popup));
        assert_eq!(window.viewport_size(), compact_size);
        assert_draft_preserved(&fixture, window, cx);
    })
    .expect("closing compact details preserves the composer");
    let popup = open_details(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click("composer-editor", cx);
        assert!(fixture.view.read(cx).window.menu.is_none());
        assert!(!cx.windows().contains(&popup));
        assert_eq!(window.viewport_size(), compact_size);
        assert_draft_preserved(&fixture, window, cx);
    })
    .expect("compact outside click preserves editing");
}

#[gpui_kit::test]
fn usage_unknown_capacity_and_over_capacity_keep_honest_details(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    update_usage(&fixture, meta.id, 1, acp::UsageUpdate::new(5, 0), cx);
    let details = open_details(&fixture, cx);
    cx.update_window(details, |_, window, cx| {
        window.render_frame(cx);
        assert_labels(
            window,
            &[
                ("usage-summary", "5 · 容量未知"),
                ("usage-used", "已用词元 · 5"),
                ("usage-capacity", "上下文容量 · 未知"),
            ],
        );
        assert!(window.try_find("usage-remaining").is_none());
        assert!(window.try_find("usage-cost").is_none());
        let button = window.find(format!("context-usage-{}", meta.id));
        let name = button
            .label()
            .map(crate::i18n::visible_text)
            .expect("unknown capacity accessible name");
        assert!(!name.contains('%') && !name.contains("NaN") && !name.contains("inf"));
    })
    .expect("zero capacity does not invent a percentage");
    let cases = [
        (
            acp::UsageUpdate::new(125, 100),
            [
                ("usage-summary", "125% · 125 / 100"),
                ("usage-used", "已用词元 · 125"),
                ("usage-capacity", "上下文容量 · 100"),
                ("usage-remaining", "剩余词元 · 0"),
            ],
        ),
        (
            acp::UsageUpdate::new(5, 1_000),
            [
                ("usage-summary", "1% · 5 / 1k"),
                ("usage-used", "已用词元 · 5"),
                ("usage-capacity", "上下文容量 · 1,000"),
                ("usage-remaining", "剩余词元 · 995"),
            ],
        ),
        (
            acp::UsageUpdate::new(169, 200),
            [
                ("usage-summary", "85% · 169 / 200"),
                ("usage-used", "已用词元 · 169"),
                ("usage-capacity", "上下文容量 · 200"),
                ("usage-remaining", "剩余词元 · 31"),
            ],
        ),
    ];
    for (index, (usage, labels)) in cases.into_iter().enumerate() {
        update_usage(&fixture, meta.id, index as u64 + 2, usage, cx);
        cx.update_window(details, |_, window, cx| {
            window.render_frame(cx);
            assert_labels(window, &labels);
        })
        .expect("known capacity details round percentages and saturate remaining tokens");
    }
    cx.update_window(details, |_, window, cx| {
        window.press("escape", cx);
    })
    .expect("dismiss capacity details");
}

#[gpui_kit::test]
fn switching_threads_closes_usage_details_and_never_reuses_the_old_value(cx: &mut TestAppContext) {
    for compact in [false, true] {
        let fixture = fixture(cx);
        let first = session();
        let second = session();
        attach(&fixture, first.clone(), cx);
        prepare_draft(&fixture, compact, cx);
        update_usage(
            &fixture,
            first.id,
            1,
            acp::UsageUpdate::new(12_000, 100_000),
            cx,
        );
        let previous_details = open_details(&fixture, cx);
        cx.update_window(previous_details, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window
                    .find("usage-summary")
                    .label()
                    .map(crate::i18n::visible_text)
                    .as_deref(),
                Some("12% · 12k / 100k")
            );
        })
        .expect("first thread usage");
        attach(&fixture, second.clone(), cx);
        update_usage(
            &fixture,
            second.id,
            1,
            acp::UsageUpdate::new(900, 1_000),
            cx,
        );
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.try_find("usage-details").is_none());
            assert!(
                window
                    .try_find(format!("context-usage-{}", first.id))
                    .is_none()
            );
            assert_eq!(
                window
                    .find(format!("context-usage-{}", second.id))
                    .label()
                    .map(crate::i18n::visible_text)
                    .as_deref(),
                Some("上下文用量 90% · 900 / 1k · 查看详情")
            );
            assert!(fixture.view.read(cx).window.menu.is_none());
            assert!(fixture.view.read(cx).window.usage_details.is_none());
            if compact {
                assert!(!cx.windows().contains(&previous_details));
            }
        })
        .expect("thread switch closes old usage details");
        let details = open_details(&fixture, cx);
        cx.update_window(details, |_, window, cx| {
            window.render_frame(cx);
            assert_labels(
                window,
                &[
                    ("usage-summary", "90% · 900 / 1k"),
                    ("usage-used", "已用词元 · 900"),
                    ("usage-capacity", "上下文容量 · 1,000"),
                ],
            );
            window.press("escape", cx);
        })
        .expect("new thread opens its own current usage");
    }
}

#[gpui_kit::test]
fn switching_window_modes_closes_usage_details_and_restores_hover(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let meta = session();
    attach(&fixture, meta.clone(), cx);
    prepare_draft(&fixture, false, cx);
    update_usage(
        &fixture,
        meta.id,
        1,
        acp::UsageUpdate::new(12_000, 100_000),
        cx,
    );
    let details = open_details(&fixture, cx);
    cx.update_window(details, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).window.usage_details, Some(meta.id));
        assert_eq!(
            window
                .find("usage-summary")
                .label()
                .map(crate::i18n::visible_text)
                .as_deref(),
            Some("12% · 12k / 100k")
        );
        window.press("ctrl-shift-m", cx);
        window.bounds_changed(cx);
    })
    .expect("switch to compact mode with usage details open");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let workspace = fixture.view.read(cx);
        assert!(workspace.window.compact);
        assert!(workspace.window.usage_details.is_none());
        assert!(workspace.window.menu.is_none());
        assert!(window.try_find("usage-details").is_none());
        assert_ring_in_toolbar(window, meta.id);
        window.hover("session-usage", cx);
    })
    .expect("compact mode has no stale details state");
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window
                .find("usage-hint")
                .label()
                .map(crate::i18n::visible_text)
                .as_deref(),
            Some("上下文用量 12% · 12k / 100k · 查看详情"),
            "switching modes restores the hover preview"
        );
        window.press("ctrl-shift-m", cx);
        window.bounds_changed(cx);
    })
    .expect("hover works before returning to full mode");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let workspace = fixture.view.read(cx);
        assert!(!workspace.window.compact);
        assert!(workspace.window.usage_details.is_none());
        assert!(workspace.window.menu.is_none());
        assert!(
            window.try_find("usage-details").is_none(),
            "returning to full mode must not replay the previously open dropdown"
        );
        assert_draft_preserved(&fixture, window, cx);
    })
    .expect("full mode returns with the details closed");
    let details = open_details(&fixture, cx);
    assert_eq!(details, fixture.window.into());
    cx.update_window(details, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(fixture.view.read(cx).window.usage_details, Some(meta.id));
        assert_eq!(
            window
                .find("usage-summary")
                .label()
                .map(crate::i18n::visible_text)
                .as_deref(),
            Some("12% · 12k / 100k")
        );
        window.press("escape", cx);
    })
    .expect("usage details can be opened again after switching modes");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(fixture.view.read(cx).window.usage_details.is_none());
        assert!(window.try_find("usage-details").is_none());
        assert_draft_preserved(&fixture, window, cx);
    })
    .expect("reopened details dismiss normally");
}
