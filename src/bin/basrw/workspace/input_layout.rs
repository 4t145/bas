use gpui_kit::{
    App, Font, Pixels,
    base::input::{DisplayMap, Rope},
};

const COMPACT_MAX_ROWS: usize = 4;
const WINDOW_MIN_ROWS: usize = 4;
const WINDOW_MAX_ROWS: usize = 10;
const EXPANDED_MIN_ROWS: usize = 12;
const EXPANDED_MAX_ROWS: usize = 20;

pub(super) fn visible_rows(
    measured_rows: usize,
    logical_rows: usize,
    compact: bool,
    expanded: bool,
) -> usize {
    let (minimum, maximum) = if expanded {
        (EXPANDED_MIN_ROWS, EXPANDED_MAX_ROWS)
    } else if compact {
        (1, COMPACT_MAX_ROWS)
    } else {
        (WINDOW_MIN_ROWS, WINDOW_MAX_ROWS)
    };
    measured_rows.max(logical_rows).clamp(minimum, maximum)
}

/// Measures ordinary wrapped text with GPUI's shaping and display projection.
pub(super) struct InputLayout {
    map: DisplayMap,
    text: Rope,
}

impl InputLayout {
    pub(super) fn new(font: Font, font_size: Pixels) -> Self {
        Self {
            map: DisplayMap::new(font, font_size, None),
            text: Rope::default(),
        }
    }

    pub(super) fn measure(
        &mut self,
        text: &Rope,
        font: Font,
        font_size: Pixels,
        width: Option<Pixels>,
        cx: &mut App,
    ) -> usize {
        self.map.set_font(font, font_size, cx);
        self.map.on_layout_changed(width, cx);
        if &self.text != text {
            self.map.ensure_text_prepared(&self.text, cx);
            self.map
                .on_text_changed(text, &(0..self.text.len()), text, cx);
            self.text = text.clone();
        } else {
            self.map.ensure_text_prepared(text, cx);
        }
        self.map.display_row_count().max(1)
    }
}
