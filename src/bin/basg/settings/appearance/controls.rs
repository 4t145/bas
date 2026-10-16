//! Appearance controls share layout, numeric stepping, inheritance and reset behavior.

use super::*;
use crate::ui;
use gpui_kit::{Entity, assets::IconName};

const CONTROLS_WIDTH: f32 = 320.;
const STEPPER_WIDTH: f32 = 112.;
const LINE_HEIGHT_STEP: f32 = 0.1;
const OPACITY_STEP: f32 = 0.05;

impl Field {
    const NUMBERS: [Self; 4] = [
        Self::UiSize,
        Self::ContentSize,
        Self::LineHeight,
        Self::Opacity,
    ];

    fn valid_number(self, value: f32) -> bool {
        value.is_finite()
            && match self {
                Self::Opacity => (0. ..=1.).contains(&value),
                _ => value > 0.,
            }
    }

    fn stepped(self, value: f32, increase: bool) -> Option<f32> {
        let step = match self {
            Self::LineHeight => LINE_HEIGHT_STEP,
            Self::Opacity => OPACITY_STEP,
            _ => 1.,
        };
        let next = value + if increase { step } else { -step };
        let next = if matches!(self, Self::LineHeight | Self::Opacity) {
            // Avoid accumulating binary rounding noise in persisted decimal settings.
            (next * 100.).round() / 100.
        } else {
            next
        };
        let next = if matches!(self, Self::Opacity) {
            next.clamp(0., 1.)
        } else {
            next
        };
        (self.valid_number(next) && next != value).then_some(next)
    }
}

impl AppearancePage {
    fn setting_input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::UiSize => &self.form.ui_size,
            Field::ContentSize => &self.form.content_size,
            Field::LineHeight => &self.form.line_height,
            Field::Opacity => &self.form.opacity,
            Field::Fallbacks => &self.form.fallbacks,
            Field::ThemeFile => &self.form.theme_file,
            _ => unreachable!("select fields have no text input"),
        }
    }

    fn effective_number(field: Field, cx: &App) -> f32 {
        match field {
            Field::UiSize => f32::from(cx.theme().font_size),
            Field::ContentSize => f32::from(theme::content_font_size(cx)),
            Field::LineHeight => theme::content_line_height(cx),
            Field::Opacity => cx.global::<theme::Appearance>().content_opacity,
            _ => unreachable!("only numeric fields use a stepper"),
        }
    }

    pub(super) fn sync_numeric_placeholders(&self, window: &mut Window, cx: &mut Context<Self>) {
        for field in Field::NUMBERS {
            let value = Self::effective_number(field, cx).to_string();
            let input = self.setting_input(field);
            if input.read(cx).presentation().placeholder().as_ref() != value {
                input.update(cx, |input, cx| input.set_placeholder(value, window, cx));
            }
        }
    }

    fn draft_number(&self, field: Field, cx: &App) -> Option<f32> {
        let value = self.setting_input(field).read(cx).value();
        let size = if value.trim().is_empty() {
            Some(Self::effective_number(field, cx))
        } else {
            value.trim().parse::<f32>().ok()
        };
        size.filter(|size| field.valid_number(*size))
    }

    fn step_number(
        &mut self,
        field: Field,
        increase: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.document.is_none() {
            return;
        }
        if let Some(value) = self
            .draft_number(field, cx)
            .and_then(|value| field.stepped(value, increase))
        {
            self.setting_input(field).update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx)
            });
            self.flush_pending(window, cx);
        }
    }

    fn number_stepper(&self, field: Field, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let value = self.draft_number(field, cx);
        let input = self.setting_input(field);
        gds_uim::el! {
            #div {
                id: $format!("{}-stepper", field.id()); test_support;
                flex; items_center; flex_shrink_0; w: STEPPER_WIDTH.px;
                h: ui::COMPACT_CONTROL.px; border_1; rounded_md; border_color: $cx.theme().border;
            } [
                #ui::icon_button($format!("{}-decrease", field.id()), $IconName::Minus) {
                    accessibility_label: $crate::i18n::tr!("appearance-value-decrease", field = field.label());
                    tooltip: $crate::i18n::tr!("appearance-value-decrease", field = field.label());
                    disabled: $(self.document.is_none() || value.and_then(|value| field.stepped(value, false)).is_none());
                    on_click: $cx.listener(move |this, _, window, cx| this.step_number(field, false, window, cx));
                },
                #div { id: $field.id(); test_support; flex_1; min_w_0; } [
                    Input(input) { small; appearance: false; text_center; disabled: $self.document.is_none(); }
                ],
                #ui::icon_button($format!("{}-increase", field.id()), $IconName::Plus) {
                    accessibility_label: $crate::i18n::tr!("appearance-value-increase", field = field.label());
                    tooltip: $crate::i18n::tr!("appearance-value-increase", field = field.label());
                    disabled: $(self.document.is_none() || value.and_then(|value| field.stepped(value, true)).is_none());
                    on_click: $cx.listener(move |this, _, window, cx| this.step_number(field, true, window, cx));
                },
            ]
        }.into_any_element()
    }

    fn overridden(&self, field: Field, cx: &App) -> bool {
        let named = |select: &Entity<SelectState<SearchableVec<form::FontChoice>>>| {
            matches!(
                select.read(cx).selected_value(),
                Some(form::FontChoice::Named(_))
            )
        };
        match field {
            Field::UiFont => {
                named(&self.form.ui_font) || !self.form.ui_size.read(cx).value().is_empty()
            }
            Field::MonoFont => named(&self.form.mono_font),
            Field::UiSize
            | Field::ContentSize
            | Field::LineHeight
            | Field::Opacity
            | Field::Fallbacks
            | Field::ThemeFile => !self.setting_input(field).read(cx).value().is_empty(),
            _ => false,
        }
    }

    fn reset_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        if self.document.is_none() {
            return;
        }
        match field {
            Field::UiFont => {
                self.form.ui_font.update(cx, |select, cx| {
                    select.set_selected_value(&form::FontChoice::Default, window, cx)
                });
                self.form
                    .ui_size
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Field::MonoFont => self.form.mono_font.update(cx, |select, cx| {
                select.set_selected_value(&form::FontChoice::Default, window, cx)
            }),
            Field::UiSize
            | Field::ContentSize
            | Field::LineHeight
            | Field::Opacity
            | Field::Fallbacks
            | Field::ThemeFile => self
                .setting_input(field)
                .update(cx, |input, cx| input.set_value("", window, cx)),
            _ => unreachable!("selects include their default option"),
        }
        self.flush_pending(window, cx);
    }

    pub(super) fn row(
        &self,
        field: Field,
        last: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let controls = match field {
            Field::UiFont => gds_uim::el! { #div { flex; items_center; gap_2; flex_1; min_w_0; } [
                #div { flex_1; min_w_0; } [$self.control(field)],
                $self.number_stepper(Field::UiSize, cx),
            ] }
            .into_any_element(),
            Field::UiSize | Field::ContentSize | Field::LineHeight | Field::Opacity => {
                self.number_stepper(field, cx)
            }
            _ => {
                gds_uim::el! { #div { flex_1; min_w_0; } [$self.control(field)] }.into_any_element()
            }
        };
        gds_uim::el! {
            #div {
                id: $format!("{}-row", field.id()); test_support;
                flex; items_center; gap_3; py_3; flex_shrink_0;
                @$(!last) => { border_b_1; border_color: $cx.theme().border; };
            } [
                #div { flex_1; min_w_0; flex; flex_col; gap_1; } [
                    #div { id: $format!("{}-label", field.id()); test_support; } [$field.label()],
                    #div { text_color: $cx.theme().muted_foreground; text_size: ui::SMALL; } [$field.help()],
                ],
                #div { flex; items_center; justify_end; gap_2; w: CONTROLS_WIDTH.px; flex_shrink_0; } [
                    #div { w: ui::COMPACT_CONTROL.px; flex_shrink_0; } [
                        ..$self.overridden(field, cx).then(|| gds_uim::el! {
                            #ui::icon_button($format!("{}-default", field.id()), $IconName::RotateCcw) {
                                accessibility_label: $crate::i18n::tr!("appearance-reset-field", field = field.label());
                                tooltip: $crate::i18n::tr!("appearance-reset-field", field = field.label());
                                disabled: $self.document.is_none();
                                on_click: $cx.listener(move |this, _, window, cx| this.reset_field(field, window, cx));
                            }
                        }),
                    ],
                    $controls,
                ],
            ]
        }.into_any_element()
    }
}
