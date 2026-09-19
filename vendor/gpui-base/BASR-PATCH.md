# Local GPUI Base patch

Source: `gpui-base` 0.6.1, upstream commit
[`96103905ea0c9c199db206ada7a9b2e3114e6339`](https://github.com/longbridge/gpui-kit/tree/96103905ea0c9c199db206ada7a9b2e3114e6339/crates/base).
The upstream source and Apache 2.0 license are retained.

`src/text/inline_flow.rs` sizes inline-code backgrounds from the shaped line's
ascent and descent, including fallback glyphs. Cap height excludes descenders
and CJK glyph extents, leaving the background shorter than the text.
The patch preserves upstream shaping, baseline alignment, wrapping and selection.

The same renderer now captures the paragraph text style before registering its
deferred layout callback. Reading `window.text_style()` inside that callback
loses the parent's font size and line height, making a paragraph with inline code
fall back to the window defaults. The regression exercises the actual deferred
measurement with several paragraph sizes.

`src/text/node.rs` keeps inline code at the surrounding text size (scale 1.0).
The application applies one body size to the composer, messages and code blocks;
the upstream hard-coded 0.875 inline-code scale otherwise breaks that sizing.
`src/text/inline.rs` preserves the code/body boundary at equal sizes, so the
code background and padding do not expand to the surrounding normal text.

Inline-flow baselines and row heights are derived from the shaped fragments'
ascent and descent, including inline-code background padding. Primary-font
metrics can use a signed descent and omit fallback glyphs; using them to shift
the fragments moved text below its measured row and clipped CJK / emoji lists.
The regression checks aligned baselines and complete glyph/background bounds
inside a clipped row at several line heights. The test text system models
signed font metrics and positive shaped descent, as on Linux.

Remove this Cargo patch when an upstream release includes the correction.

`src/input/base/element.rs` reserves the line-number margin only when line
numbers or folding controls are enabled. A composer with both disabled should
not retain an invisible 10px gutter. The application's toolbar regression
compares the actual caret position with the first button in both window modes.

The input state also exposes its visual row count so the composer can grow for
soft-wrapped inline attachments without estimating text widths.

Regression check:

```sh
nix develop .#gui -c cargo test --manifest-path vendor/gpui-base/Cargo.toml --target-dir target --lib text::inline_flow
```
