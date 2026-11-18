use eframe::egui::{self, Color32, RichText};
use zorvinkel_bird_avatars::AvatarSpec;

pub const BLUE: Color32 = Color32::from_rgb(22, 100, 150);
pub const BLUE_PALE: Color32 = Color32::from_rgb(229, 242, 249);
pub const BACKGROUND: Color32 = Color32::from_rgb(245, 248, 250);
pub const SIDEBAR: Color32 = Color32::from_rgb(235, 242, 246);
pub const MUTED: Color32 = Color32::from_rgb(83, 107, 123);
pub const AMBER: Color32 = Color32::from_rgb(130, 78, 10);
pub const GREEN: Color32 = Color32::from_rgb(28, 128, 76);
pub const ERROR: Color32 = Color32::from_rgb(161, 46, 46);
pub const ERROR_PALE: Color32 = Color32::from_rgb(255, 236, 236);
const BIRD_PNG: &[u8] = include_bytes!("../../../../docs/logo/zorvinkel-bird.png");

const INK: Color32 = Color32::from_rgb(14, 48, 69);
pub const BORDER: Color32 = Color32::from_rgb(210, 225, 233);

pub fn apply(context: &egui::Context) {
    // Without a fixed preference egui follows the system theme and, once the platform reports
    // it (on Windows only after start-up), switches to its own default style.
    context.set_theme(egui::ThemePreference::Light);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BACKGROUND;
    visuals.faint_bg_color = BLUE_PALE;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.selection.bg_fill = BLUE_PALE;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, BLUE);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(8);
        widget.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = egui::Stroke::new(1.0_f32, INK);
    }
    visuals.widgets.inactive.bg_fill = Color32::WHITE;
    visuals.widgets.hovered.bg_fill = BLUE_PALE;
    visuals.widgets.active.bg_fill = Color32::from_rgb(207, 230, 242);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    context.set_visuals(visuals);
    context.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 10.0);
        style.spacing.interact_size.y = 36.0;
        for (kind, size) in [
            (egui::TextStyle::Body, 16.0),
            (egui::TextStyle::Button, 16.0),
            (egui::TextStyle::Heading, 22.0),
            (egui::TextStyle::Small, 13.0),
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
        .fill(Color32::WHITE)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(14)
        .inner_margin(16)
}

pub fn primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
        .fill(BLUE)
        .stroke(egui::Stroke::NONE)
}

/// One choice out of a few that sit side by side; the chosen one is filled.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    ui.horizontal(|ui| {
        for (option, label) in options {
            let button = if *value == *option {
                primary_button(label)
            } else {
                egui::Button::new(*label)
            };
            if ui.add(button).clicked() {
                *value = *option;
            }
        }
    });
}

/// Small filled circle that marks a connection state.
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub fn message(ui: &mut egui::Ui, text: &str, foreground: Color32, background: Color32) {
    egui::Frame::new()
        .fill(background)
        .corner_radius(8)
        .inner_margin(12)
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
        ui.add_space(4.0);
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
    let color = SIDEBAR.to_array();
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
