use super::*;
use crate::text::inline::test_fonts::{MONO, WideMonoTextSystem};
use gpui::{Empty, TestApp};

#[test]
fn mixed_text_and_code_fit_inside_clipped_list_rows() {
    let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
    let mut window = app.open_window(|_, _| Empty);
    window.update(|_, window, _| {
        for height in [14., 18., 22.] {
            let text: SharedString = "📊 中文 gypq 日本語 한국어".into();
            let style = TextStyle {
                font_size: px(14.).into(),
                line_height: px(height).into(),
                ..Default::default()
            };
            let items = vec![MeasureItem::Text {
                text: text.clone(),
                links: vec![],
                highlights: vec![(
                    "📊 中文 ".len().."📊 中文 gypq".len(),
                    InlineHighlight {
                        font_family: Some(MONO.into()),
                        font_size_scale: Some(1.),
                        ..Default::default()
                    },
                )],
            }];
            let layout = layout_flow(&items, &[None], &style, None, window);
            let mut baselines = Vec::new();
            for fragment in &layout.fragments {
                let PositionedFragment::Text {
                    origin,
                    size,
                    text,
                    highlights,
                    font_size,
                    ..
                } = fragment
                else {
                    continue;
                };
                let runs = text_runs(text.len(), &style, highlights);
                let shaped = shape_line(text.clone(), *font_size, &runs, window);
                let baseline =
                    origin.y + (size.height - shaped.ascent - shaped.descent) / 2. + shaped.ascent;
                baselines.push(baseline);
                let code_padding = if highlights.iter().any(|(_, h)| h.font_size_scale.is_some()) {
                    *font_size * 0.125
                } else {
                    px(0.)
                };
                assert!(
                    baseline - shaped.ascent - code_padding >= px(-0.01),
                    "row clips the top of {text}"
                );
                assert!(
                    baseline + shaped.descent + code_padding <= layout.size.height + px(0.01),
                    "row clips the bottom of {text}"
                );
            }
            assert!(
                baselines
                    .windows(2)
                    .all(|pair| (pair[0] - pair[1]).abs() < px(0.01))
            );
        }
    });
}

#[test]
fn delayed_measurement_keeps_the_paragraph_font_size_and_line_height() {
    let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
    let mut window = app.open_window(|_, _| MeasurementProbe);
    window.draw();
}

struct MeasurementProbe;

impl gpui::Render for MeasurementProbe {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::canvas(
            |_, window, cx| {
                for font_size in [px(12.), px(14.), px(24.)] {
                    let text: SharedString = "正文 中文 `code`".into();
                    let mut flow = InlineFlow::new(
                        "font-probe",
                        vec![InlineFlowItem::Text {
                            state: Arc::new(Mutex::new(InlineState::default())),
                            text: text.clone(),
                            links: vec![],
                            highlights: vec![(
                                "正文 中文 ".len()..text.len(),
                                InlineHighlight {
                                    font_family: Some(MONO.into()),
                                    font_size_scale: Some(1.),
                                    ..Default::default()
                                },
                            )],
                        }],
                        None,
                    );
                    let line_height = font_size * 2.;
                    let (id, state) = window.with_text_style(
                        Some(gpui::TextStyleRefinement {
                            font_size: Some(font_size.into()),
                            line_height: Some(line_height.into()),
                            ..Default::default()
                        }),
                        |window| flow.request_layout(None, None, window, cx),
                    );
                    // GPUI evaluates measured layouts after the parent's style scope has exited.
                    window.compute_layout(
                        id,
                        size(
                            AvailableSpace::Definite(px(800.)),
                            AvailableSpace::MinContent,
                        ),
                        cx,
                    );
                    let layout = state.layout.lock().expect("layout");
                    let layout = layout.as_ref().expect("measured paragraph");
                    assert_eq!(layout.size.height, line_height);
                    assert_eq!(
                        layout.fragments.len(),
                        2,
                        "equal-size code must remain separate from body text"
                    );
                    for fragment in &layout.fragments {
                        if let PositionedFragment::Text {
                            font_size: actual, ..
                        } = fragment
                        {
                            assert_eq!(
                                *actual, font_size,
                                "inline code and body must inherit their paragraph size"
                            );
                        }
                    }
                }
            },
            |_, _, _, _| {},
        )
        .size_full()
    }
}

#[test]
fn code_background_covers_ascenders_and_descenders_at_each_font_size() {
    let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
    let mut window = app.open_window(|_, _| Empty);
    window.update(|_, window, _| {
        for font_size in [px(12.), px(18.), px(28.)] {
            for text in ["gypq/路径", "你好，世界 中文测试 日本語テスト 한글 테스트"]
            {
                let style = TextStyle {
                    font_family: MONO.into(),
                    ..Default::default()
                };
                let runs = text_runs(text.len(), &style, &[]);
                let mut line = shape_line(text.to_owned().into(), font_size, &runs, window);
                // Model a fallback font whose glyph extents exceed the primary font's cap height.
                if text.contains('世') {
                    *line = Arc::new(gpui::LineLayout {
                        font_size,
                        width: line.width(),
                        ascent: font_size * 1.1,
                        descent: font_size * 0.4,
                        len: text.len(),
                        ..Default::default()
                    });
                }
                let fragment = size(line.width(), font_size * 1.5);
                let background = code_background_bounds(fragment, &line, font_size);
                let baseline = (fragment.height - line.ascent - line.descent) / 2. + line.ascent;
                assert!(
                    background.top() <= baseline - line.ascent,
                    "top clips {text}"
                );
                assert!(
                    background.bottom() >= baseline + line.descent,
                    "bottom clips {text}"
                );
                assert_eq!(background.size.width, fragment.width);
            }
        }
    });
}
