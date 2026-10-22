use super::*;
use bas::model::Timestamp;
use bas::service::HistoryOverviewEntry;

const TICK_WIDTH: f32 = 14.;
const HOVER_TICK_WIDTH: f32 = 20.;
const ACTIVE_TICK_HEIGHT: f32 = 4.;
const MARKER_HEIGHT: f32 = 12.;
const RAIL_CHROME_HEIGHT: f32 = 24.;

fn marker_tick(
    id: SharedString,
    group: SharedString,
    active: bool,
    bookmarked: bool,
    cx: &App,
) -> AnyElement {
    gds_uim::el! {
        #div { size_full; flex; items_center; justify_center; } [
            #div {
                id: id; test_support; w: TICK_WIDTH.px;
                h: $(if active { ACTIVE_TICK_HEIGHT } else { 2. }).px; rounded_full;
                bg: $(if active || bookmarked { cx.theme().primary } else { cx.theme().muted_foreground.opacity(0.7) });
                group_hover: group, $gds_uim::m! {
                    w: HOVER_TICK_WIDTH.px;
                    h: $(if active { ACTIVE_TICK_HEIGHT } else { 3. }).px;
                    bg: $cx.theme().primary;
                };
            }
        ]
    }.into_any_element()
}

fn marker_time(entry: &HistoryOverviewEntry) -> Timestamp {
    Timestamp::from_millis(u64::try_from(entry.timestamp_millis).unwrap_or_default())
}

fn marker_hint(entry: &HistoryOverviewEntry, bookmarked: bool) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        entry.preview,
        crate::ui::time::recent_time(marker_time(entry), Timestamp::now()),
        if bookmarked {
            crate::i18n::tr!("turn-marker-remove-bookmark")
        } else {
            crate::i18n::tr!("turn-marker-add-bookmark")
        },
        crate::i18n::tr!("turn-marker-wheel-hint"),
    )
}

impl Workspace {
    pub(in crate::workspace) fn render_turn_navigation(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.selected.id else {
            return gds_uim::el! { #div }.into_any_element();
        };
        let navigation = &self.selected.timeline.navigation;
        let active = self.active_marker();
        let at_end = !navigation.detached
            && active.is_none()
            && (self.selected.timeline.list.is_following_tail()
                || self
                    .selected
                    .timeline
                    .list
                    .is_scrolled_to_end()
                    .unwrap_or(false));
        let mut markers = gds_uim::el! { #div {
            id: $SharedString::from(format!("turn-prompts-{session}")); test_support;
            w_full; min_h_0; flex; flex_col; flex_shrink_1; overflow_y_scroll;
        } };
        let entries = navigation
            .overview
            .as_ref()
            .map(|overview| overview.entries.as_slice())
            .unwrap_or_default();
        let available = (f32::from(self.selected.timeline.list.viewport_bounds().size.height)
            - RAIL_CHROME_HEIGHT)
            .max(MARKER_HEIGHT);
        let marker_height = (available / entries.len().max(1) as f32).clamp(2., MARKER_HEIGHT);
        for entry in entries {
            let cursor = entry.cursor;
            let bookmarked = self
                .navigation
                .marker_bookmarks
                .get(&session)
                .is_some_and(|bookmarks| bookmarks.contains(&cursor));
            let marker_id = SharedString::from(format!("turn-marker-{cursor}"));
            let group = marker_id.clone();
            let hint: SharedString = marker_hint(entry, bookmarked).into();
            let preview = crate::ui::tooltip::wrap_tokens(&entry.preview);
            let at = marker_time(entry);
            let control_hint = if bookmarked {
                crate::i18n::tr!("turn-marker-remove-bookmark")
            } else {
                crate::i18n::tr!("turn-marker-add-bookmark")
            };
            let label = hint.clone();
            let tick = marker_tick(
                format!("turn-marker-line-{cursor}").into(),
                group.clone(),
                active == Some(cursor),
                bookmarked,
                cx,
            );
            let marker = gds_uim::el! { #div {
                id: marker_id; test_support; role: gpui_kit::Role::Button;
                aria_label: hint; group: group; h: $px(marker_height); min_h: 2.0.px;
                flex_shrink_0; w_full; cursor_pointer;
                hoverable_tooltip: $move |window, cx| {
                    let preview = preview.clone();
                    let at = crate::ui::time::recent_time(at, Timestamp::now());
                    let previous = shortcuts::hint(crate::i18n::tr!("turn-marker-previous"),
                        &shortcuts::PreviousConversationTurn, window);
                    let next = shortcuts::hint(crate::i18n::tr!("turn-marker-next"),
                        &shortcuts::NextConversationTurn, window);
                    let label = format!("{label}\n{previous}\n{next}");
                    crate::ui::tooltip::element("turn-marker-tooltip", label, move |_, cx| {
                        gds_uim::el! { #div { flex; flex_col; gap_2; } [
                            #div {
                                id: "turn-marker-preview"; test_support;
                                whitespace_normal; text_size: crate::ui::SMALL;
                                pb_2; border_b_1; border_color: $cx.theme().border;
                            } [$preview.clone()],
                            #div {
                                id: "turn-marker-metadata"; test_support;
                                flex; items_center; justify_between; gap_3;
                                text_xs; text_color: $cx.theme().muted_foreground;
                            } [
                                #div {
                                    id: "turn-marker-time"; test_support;
                                    role: gpui_kit::Role::Label; aria_label: $at.clone();
                                } [$at.clone()],
                                ..$bookmarked.then(|| gds_uim::el! {
                                    #div {
                                        id: "turn-marker-bookmark-label"; test_support;
                                        role: gpui_kit::Role::Label;
                                        aria_label: $crate::i18n::tr!("turn-marker-bookmarked");
                                        flex; items_center; gap_1; flex_shrink_0;
                                        rounded_sm; px_1; whitespace_nowrap;
                                        bg: $cx.theme().primary.opacity(0.15);
                                        text_color: $cx.theme().primary;
                                    } [
                                        #div {
                                            id: "turn-marker-bookmark-icon"; test_support;
                                            flex; items_center; flex_shrink_0;
                                        } [Icon(IconName::Bookmark) { xsmall; }],
                                        $crate::i18n::tr!("turn-marker-bookmarked"),
                                    ]
                                }),
                            ],
                            #div {
                                id: "turn-marker-shortcuts"; test_support;
                                flex; flex_col; gap_1; pt_2; border_t_1;
                                border_color: $cx.theme().border; text_xs;
                                text_color: $cx.theme().muted_foreground;
                            } [
                                #div [$control_hint],
                                #div {
                                    id: "turn-marker-previous-hint"; test_support;
                                    role: gpui_kit::Role::Label; aria_label: $previous.clone();
                                } [$previous.clone()],
                                #div {
                                    id: "turn-marker-next-hint"; test_support;
                                    role: gpui_kit::Role::Label; aria_label: $next.clone();
                                } [$next.clone()],
                                #div [$crate::i18n::tr!("turn-marker-wheel-hint")],
                            ],
                        ] }.into_any_element()
                    }, window, cx)
                };
                on_click: $cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                    if this.selected.id != Some(session) { return; }
                    if event.modifiers().control { this.toggle_marker_bookmark(session, cursor, cx); }
                    else { this.navigate_to_marker(cursor, cx); }
                });
            } [$tick] };
            markers = gds_uim::el! { $markers [$marker] };
        }
        let group: SharedString = "turn-marker-end".into();
        let end = gds_uim::el! { #div {
            id: $group.clone(); test_support; role: gpui_kit::Role::Button;
            aria_label: $crate::i18n::tr!("conversation-end"); group: $group.clone();
            hoverable_tooltip: $shortcuts::tooltip(crate::i18n::tr!("conversation-end"), ScrollConversationToEnd);
            h: MARKER_HEIGHT.px; w_full; flex_shrink_0; cursor_pointer;
            on_click: $cx.listener(|this, _, _, cx| this.scroll_conversation_to_end(cx));
        } [$marker_tick("turn-marker-line-end".into(), group, at_end, false, cx)] };
        gds_uim::el! { #div {
            id: "turn-navigation"; test_support; absolute; top_0; bottom_0; right: $px(-RAIL_WIDTH);
            flex; flex_col; justify_center; py_1; w: RAIL_WIDTH.px; flex_shrink_0;
        } [
            #div {
                id: "turn-shortcuts"; test_support; flex; flex_col; w_full; min_h_0; max_h_full; rounded_md;
                hover => { bg: $cx.theme().muted.opacity(0.24); };
                on_scroll_wheel: $cx.listener(|this, event, window, cx| {
                    this.on_navigation_wheel(event, window, cx);
                    cx.stop_propagation();
                });
            } [
                ..$navigation.overview_request.is_some().then(|| status::activity_icon(
                    "turn-overview-loading", IconName::LoaderCircle, cx.theme().muted_foreground, true)),
                ..$navigation.overview_error.as_ref().map(|error| gds_uim::el! {
                    #crate::ui::tooltip::bounded("turn-overview-error",
                        $gds_uim::el! { #div { text_color: $cx.theme().danger; } ["!"] },
                        $error.presentation(),
                    ) {
                        test_support; cursor_pointer;
                        on_click: $cx.listener(|this, _, _, cx| this.request_history_overview(0, cx));
                    }
                }),
                $markers, $end,
            ],
        ] }.into_any_element()
    }

    pub(in crate::workspace) fn render_history_location_status(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let navigation = &self.selected.timeline.navigation;
        let loading = navigation.location.is_some();
        let error = navigation.location_error.as_ref();
        if !loading && error.is_none() && !navigation.detached {
            return None;
        }
        Some(gds_uim::el! { #div {
            id: "history-location-status"; test_support;
            absolute; bottom_0; left_0; right_0; flex; justify_center; px_3;
        } [
            #div {
                flex; items_center; gap_2; px_3; py_1; max_w_full; rounded_md;
                bg: $cx.theme().background; text_size: crate::ui::SMALL;
            } [
                ..$loading.then(|| status::activity_icon("history-location-loading",
                    IconName::LoaderCircle, cx.theme().muted_foreground, true)),
                ..$error.map(|error| crate::ui::feedback("history-location-error", error.presentation(), cx.theme().danger).compact()),
                ..$((!loading && navigation.after.is_some()).then(|| gds_uim::el! {
                    #crate::ui::action_button("history-location-newer") {
                        label: $crate::i18n::tr!("turn-marker-load-newer");
                        on_click: $cx.listener(|this, _, _, cx| this.load_next_history(cx));
                    }
                })),
                ..$((!loading).then(|| gds_uim::el! {
                    #crate::ui::action_button("history-location-latest") {
                        label: $crate::i18n::tr!("conversation-end");
                        on_click: $cx.listener(|this, _, _, cx| this.scroll_conversation_to_end(cx));
                    }
                })),
            ],
        ] }.into_any_element())
    }
}
