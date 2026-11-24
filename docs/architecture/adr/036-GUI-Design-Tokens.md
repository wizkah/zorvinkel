# ADR-036 – GUI design tokens and shared widgets

**Status:** Accepted. Extends ADR-005; ADR-035 stays in force.  
**Date:** 2026-11-24

## Context

The client's look was spread over the pages. `theme.rs` held eleven colours named by hue (`BLUE`, `AMBER`) and four text sizes; spacing, radii, field widths and control heights were numbers written into each page. A visual check of the simulation on Windows showed what followed from it:

- Secondary buttons were egui's default grey. egui paints buttons with `weak_bg_fill`, which the theme never set, so they did not match the blue-grey palette and the theme's hover colour never reached them.
- The function list in the status column showed `✓` as an empty box: the bundled egui fonts have no tick glyph. Together with the green text it read as unchecked checkboxes.
- The status column did not scroll and did not wrap; its last entries were cut off.
- Single-line fields were about 20 px high next to 36 px buttons; forms put labels above (Wi-Fi) or beside (terminal) the field.
- The Wi-Fi mode switch and the page's main action were both filled blue and both said "Join Wi-Fi". Page tabs inside a device tab looked like the window's tabs above them. Found networks looked like plain text.
- Confirmations that shut a device down, stop a service or replace a trusted host key looked like any other main action.

## Decision

`theme.rs` is the single place for the look:

- **Colours by role:** accent (and subtle, pressed), surfaces (window, raised, sidebar), control fill, text and muted text, border, success, warning (and subtle), danger (and subtle). All widget states, including `weak_bg_fill`, are set from them.
- **Scales:** spacing `space::{XS, S, M, L, XL}` = 4/8/12/16/24; radii `radius::{CONTROL, CARD}` = 8/12; text sizes small/body/title/heading = 13/16/18/22; sizes for control height (36), row height (32), field width (420), content width (880) and sidebar width (240). Within a group items are `S` apart, between groups `M` or `L`.
- **Shared widgets:** primary and danger buttons (secondary is egui's default button with the theme), a segmented control on a shared track, underlined page tabs, a framed choice list with rows, a text field with control height, a label–value pair, a badge, a painted tick, the section title of a card, plus the existing switch, message and status dot.
- **Rules:** one primary button per view; danger buttons only for the confirmation of an interrupting or trust-replacing action; labels above fields; page content stops at the content width except the terminal; the status column scrolls; the simulation is marked by a badge in the tab bar.

Pages use these names instead of numbers. The terminal's own colour palette and the progress animation keep their values.

## Consequences

- A change of colour, spacing or size is made once in `theme.rs`.
- The client stays light-only (`ThemePreference::Light`); a dark palette would be a new set of the same roles.
- Glyphs outside the bundled fonts must be painted (eye, tick) or the fonts extended.
- Checked by build, clippy, tests and on screen in the Windows simulation (English and German); not looked at on macOS or Linux, not against a device, and no accessibility review (contrast measured by eye only).
