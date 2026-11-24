//! Design tokens and the few widgets built on them.
//!
//! Pages take colours, spacing, radii and text sizes from here instead of writing numbers, so
//! the look changes in one place. Colours are named by their role, not by their hue.
use eframe::egui::{self, Color32, RichText};
use zorvinkel_bird_avatars::AvatarSpec;

/// Buttons that start the main action, selected items, links.
pub const ACCENT: Color32 = Color32::from_rgb(22, 100, 150);
/// Hover and selection backgrounds, informational messages.
pub const ACCENT_SUBTLE: Color32 = Color32::from_rgb(229, 242, 249);
/// Background of a control while it is pressed.
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(207, 230, 242);
/// Window background behind cards.
pub const SURFACE: Color32 = Color32::from_rgb(245, 248, 250);
/// Cards, tiles, input fields and the tab bar.
pub const SURFACE_RAISED: Color32 = Color32::WHITE;
/// The status column beside a device tab.
pub const SURFACE_SIDEBAR: Color32 = Color32::from_rgb(235, 242, 246);
/// Fill of secondary buttons and of the segmented control's track.
pub const CONTROL: Color32 = Color32::from_rgb(236, 243, 247);
pub const TEXT: Color32 = Color32::from_rgb(14, 48, 69);
/// Labels, hints and inactive states.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(83, 107, 123);
pub const BORDER: Color32 = Color32::from_rgb(210, 225, 233);
pub const SUCCESS: Color32 = Color32::from_rgb(28, 128, 76);
pub const WARNING: Color32 = Color32::from_rgb(130, 78, 10);
pub const WARNING_SUBTLE: Color32 = Color32::from_rgb(253, 243, 224);
pub const DANGER: Color32 = Color32::from_rgb(161, 46, 46);
pub const DANGER_SUBTLE: Color32 = Color32::from_rgb(255, 236, 236);

/// Spacing scale: within a group `S`, between groups `L`.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

pub mod radius {
    /// Buttons, fields, rows and messages.
    pub const CONTROL: u8 = 8;
    /// Cards and device tiles.
    pub const CARD: u8 = 12;
}

pub mod text_size {
    pub const SMALL: f32 = 13.0;
    pub const BODY: f32 = 16.0;
    /// Heading inside a card.
    pub const TITLE: f32 = 18.0;
    /// Page heading and device name.
    pub const HEADING: f32 = 22.0;
}

pub mod size {
    /// Height of buttons and single-line fields.
    pub const CONTROL_HEIGHT: f32 = 36.0;
    /// Height of list rows and of the controls inside the segmented control.
    pub const ROW_HEIGHT: f32 = 32.0;
    /// Widest single-line input; longer values scroll inside the field.
    pub const FIELD_WIDTH: f32 = 420.0;
    /// Widest page content; beyond it lines get too long to read.
    pub const CONTENT_MAX_WIDTH: f32 = 880.0;
    pub const SIDEBAR_WIDTH: f32 = 240.0;
}

const BIRD_PNG: &[u8] = include_bytes!("../../../../docs/logo/zorvinkel-bird.png");

pub fn apply(context: &egui::Context) {
    // Without a fixed preference egui follows the system theme and, once the platform reports
    // it (on Windows only after start-up), switches to its own default style.
    context.set_theme(egui::ThemePreference::Light);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = SURFACE;
    visuals.faint_bg_color = ACCENT_SUBTLE;
    visuals.extreme_bg_color = SURFACE_RAISED;
    visuals.selection.bg_fill = ACCENT_SUBTLE;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    // Buttons paint `weak_bg_fill`, other widgets `bg_fill`; both follow the palette.
    for (widget, fill) in [
        (&mut visuals.widgets.inactive, CONTROL),
        (&mut visuals.widgets.hovered, ACCENT_SUBTLE),
        (&mut visuals.widgets.active, ACCENT_PRESSED),
        (&mut visuals.widgets.open, ACCENT_SUBTLE),
    ] {
        widget.corner_radius = egui::CornerRadius::same(radius::CONTROL);
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
        widget.expansion = 0.0;
    }
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(radius::CONTROL);
    context.set_visuals(visuals);
    context.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(space::S, space::S);
        style.spacing.button_padding = egui::vec2(space::L, space::S);
        style.spacing.interact_size.y = size::CONTROL_HEIGHT;
        for (kind, size) in [
            (egui::TextStyle::Body, text_size::BODY),
            (egui::TextStyle::Button, text_size::BODY),
            (egui::TextStyle::Heading, text_size::HEADING),
            (egui::TextStyle::Small, text_size::SMALL),
        ] {
            style
                .text_styles
                .insert(kind, egui::FontId::proportional(size));
        }
    });
}

pub const WELCOME_BIRD_SIZE: egui::Vec2 = egui::vec2(210.0, 122.0);

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE_RAISED)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(radius::CARD)
        .inner_margin(space::L)
}

/// Heading of a card; one step below the page heading.
pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(text_size::TITLE).strong());
}

/// The one main action of a view.
pub fn primary_button(label: &str) -> egui::Button<'_> {
    filled_button(label, ACCENT)
}

/// Confirms an action that interrupts the device or a service, or replaces a trusted key.
pub fn danger_button(label: &str) -> egui::Button<'_> {
    filled_button(label, DANGER)
}

fn filled_button(label: &str, fill: Color32) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
        .fill(fill)
        .stroke(egui::Stroke::NONE)
}

/// Single-line field with the height of a button.
pub fn text_field(edit: egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    edit.margin(egui::Margin::symmetric(space::M as i8, 0))
        .min_size(egui::vec2(0.0, size::CONTROL_HEIGHT))
        .vertical_align(egui::Align::Center)
}

/// One choice out of a few that sit side by side on a shared track. It looks like a switch,
/// not like a row of buttons, so it is not mistaken for the action of the page.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    egui::Frame::new()
        .fill(CONTROL)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(radius::CONTROL)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (option, label) in options {
                    let chosen = *value == *option;
                    let text =
                        RichText::new(*label).color(if chosen { ACCENT } else { TEXT_MUTED });
                    let button = egui::Button::new(if chosen { text.strong() } else { text })
                        .min_size(egui::vec2(0.0, size::ROW_HEIGHT))
                        .corner_radius(radius::CONTROL - 2);
                    let button = if chosen {
                        button
                            .fill(SURFACE_RAISED)
                            .stroke(egui::Stroke::new(1.0_f32, BORDER))
                    } else {
                        button.frame_when_inactive(false)
                    };
                    if ui.add(button).clicked() {
                        *value = *option;
                    }
                }
            });
        });
}

/// A page tab inside a device tab: an underlined label, so it does not look like the pill
/// shaped tabs of the window above it. Hovering darkens the label and shows a faint line.
pub fn page_tab(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = egui::vec2(galley.size().x + 2.0 * space::M, size::ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = response.hovered();
    let (color, line) = match (selected, hovered) {
        (true, _) => (ACCENT, Some(ACCENT)),
        (false, true) => (TEXT, Some(BORDER)),
        (false, false) => (TEXT_MUTED, None),
    };
    let fade = |color: Color32| {
        if ui.is_enabled() {
            color
        } else {
            color.gamma_multiply(0.5)
        }
    };
    let text_pos = egui::pos2(
        rect.left() + space::M,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, fade(color));
    if let Some(line) = line {
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(2.0_f32, fade(line)),
        );
    }
    response
}

/// A framed list of choices as wide as a field; `add_rows` adds `list_row`s.
pub fn choice_list(ui: &mut egui::Ui, add_rows: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.set_max_width(size::FIELD_WIDTH);
        egui::Frame::new()
            .fill(SURFACE_RAISED)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .corner_radius(radius::CONTROL)
            .inner_margin(2)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                add_rows(ui);
            });
    });
}

/// One row of a `choice_list`, such as a Wi-Fi network found nearby.
pub fn list_row(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, size::ROW_HEIGHT), egui::Sense::click());
    let (fill, color) = if selected {
        (ACCENT_SUBTLE, ACCENT)
    } else if response.hovered() {
        (CONTROL, TEXT)
    } else {
        (Color32::TRANSPARENT, TEXT)
    };
    ui.painter().rect_filled(rect, radius::CONTROL - 2, fill);
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(
        egui::pos2(rect.left() + space::M, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    response
}

/// A label with its value below, as in the status column.
pub fn label_value(ui: &mut egui::Ui, label: &str, value: RichText) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).small().color(TEXT_MUTED));
        ui.add(egui::Label::new(value).wrap());
    });
    ui.add_space(space::XS);
}

/// A short tag on a tinted background, such as the simulation marker. Painted at its own
/// height so it does not stretch to the row it sits in.
pub fn badge(ui: &mut egui::Ui, text: &str, foreground: Color32, background: Color32) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, foreground);
    let padding = egui::vec2(space::S, space::XS);
    let (rect, _) = ui.allocate_exact_size(galley.size() + 2.0 * padding, egui::Sense::hover());
    ui.painter().rect_filled(rect, radius::CONTROL, background);
    ui.painter().galley(rect.min + padding, galley, foreground);
}

/// Small filled circle that marks a connection state.
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

/// A painted tick, or a dash when `on` is false. Painted because the bundled egui fonts have
/// no tick glyph.
pub fn tick(ui: &mut egui::Ui, on: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    let painter = ui.painter();
    if on {
        let stroke = egui::Stroke::new(2.0_f32, SUCCESS);
        let points = [
            rect.left_center() + egui::vec2(1.0, 0.0),
            rect.center_bottom() + egui::vec2(-1.5, -3.0),
            rect.right_top() + egui::vec2(-1.0, 3.0),
        ];
        painter.line_segment([points[0], points[1]], stroke);
        painter.line_segment([points[1], points[2]], stroke);
    } else {
        let y = rect.center().y;
        painter.line_segment(
            [
                egui::pos2(rect.left() + 3.0, y),
                egui::pos2(rect.right() - 3.0, y),
            ],
            egui::Stroke::new(1.5_f32, TEXT_MUTED),
        );
    }
}

/// A labelled on/off switch; the response reports `changed` when it was clicked.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let mut response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(32.0, 18.0), egui::Sense::click());
            let fill = if *on { ACCENT } else { BORDER };
            ui.painter().rect_filled(rect, 9.0, fill);
            let knob = if *on {
                rect.right() - 9.0
            } else {
                rect.left() + 9.0
            };
            ui.painter()
                .circle_filled(egui::pos2(knob, rect.center().y), 7.0, Color32::WHITE);
            let label = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
            response.union(label)
        })
        .inner;
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response
}

pub fn message(ui: &mut egui::Ui, text: &str, foreground: Color32, background: Color32) {
    egui::Frame::new()
        .fill(background)
        .corner_radius(radius::CONTROL)
        .inner_margin(space::M)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(foreground));
        });
}

pub struct Brand {
    bird: egui::TextureHandle,
    wordmark: egui::TextureHandle,
}

impl Brand {
    pub fn load(context: &egui::Context) -> Result<Self, image::ImageError> {
        Ok(Self {
            bird: load_texture(context, "zorvinkel-bird", BIRD_PNG)?,
            wordmark: load_texture(
                context,
                "zorvinkel-wordmark",
                include_bytes!("../../../../docs/logo/zorvinkel-wordmark.png"),
            )?,
        })
    }

    /// Bird and wordmark for a right-to-left layout: the wordmark ends up on the far right.
    pub fn header_right(&self, ui: &mut egui::Ui) {
        ui.add(egui::Image::new(&self.wordmark).fit_to_exact_size(egui::vec2(150.0, 29.0)));
        ui.add_space(space::XS);
        ui.add(egui::Image::new(&self.bird).fit_to_exact_size(egui::vec2(76.0, 44.0)));
    }
}

/// Texture edge length of a rendered avatar; large enough for 2× displays at the sizes in use.
const AVATAR_PIXELS: u32 = 160;

/// Bird avatar derived from the name (cached per name); the SVG tile has its own rounded
/// background.
pub fn avatar(ui: &mut egui::Ui, name: &str, size: f32) {
    let id = egui::Id::new(("bird-avatar", name));
    let texture = match ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id))
    {
        Some(texture) => Some(texture),
        None => {
            let texture = render_avatar(ui.ctx(), name);
            if let Some(texture) = &texture {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(id, texture.clone()));
            }
            texture
        }
    };
    match texture {
        Some(texture) => {
            ui.add(egui::Image::new(&texture).fit_to_exact_size(egui::vec2(size, size)));
        }
        None => {
            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        }
    }
}

fn render_avatar(context: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
    let seed = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let svg = zorvinkel_bird_avatars::render_svg(&AvatarSpec::from_seed(seed), "avatar");
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixels = resvg::tiny_skia::Pixmap::new(AVATAR_PIXELS, AVATAR_PIXELS)?;
    let scale = AVATAR_PIXELS as f32 / tree.size().width();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixels.as_mut(),
    );
    let size = [AVATAR_PIXELS as usize; 2];
    Some(context.load_texture(
        format!("bird-avatar-{name}"),
        egui::ColorImage::from_rgba_premultiplied(size, pixels.data()),
        egui::TextureOptions::LINEAR,
    ))
}

fn load_texture(
    context: &egui::Context,
    name: &str,
    bytes: &[u8],
) -> Result<egui::TextureHandle, image::ImageError> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(context.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    ))
}

pub fn app_icon() -> Result<egui::IconData, image::ImageError> {
    let bird_size = if cfg!(target_os = "macos") { 400 } else { 460 };
    let bird = image::load_from_memory_with_format(BIRD_PNG, image::ImageFormat::Png)?
        .resize(bird_size, bird_size, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let mut canvas = icon_background();
    image::imageops::overlay(
        &mut canvas,
        &bird,
        i64::from((512 - bird.width()) / 2),
        i64::from((512 - bird.height()) / 2),
    );
    Ok(egui::IconData {
        width: 512,
        height: 512,
        rgba: canvas.into_raw(),
    })
}

fn icon_background() -> image::RgbaImage {
    let color = SURFACE_SIDEBAR.to_array();
    image::RgbaImage::from_fn(512, 512, |x, y| {
        let mut pixel = color;
        if cfg!(target_os = "macos") {
            // Inset the rounded tile to match the optical size of macOS Dock icons.
            let x = x as f32 + 0.5;
            let y = y as f32 + 0.5;
            let dx = x - x.clamp(128.0, 384.0);
            let dy = y - y.clamp(128.0, 384.0);
            let distance = dx.hypot(dy);
            pixel[3] = ((96.5 - distance).clamp(0.0, 1.0) * 255.0) as u8;
        }
        image::Rgba(pixel)
    })
}
