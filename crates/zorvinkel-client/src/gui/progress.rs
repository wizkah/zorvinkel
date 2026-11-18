use super::i18n::{Language, Text};
use eframe::egui;

const SVG: &str = include_str!("../../../../docs/logo/zorvinkel-progress.svg");
const WING_PERIOD: f64 = 8.2;
const GROUND_PERIOD: f64 = 1.6;
const SIZE: [usize; 2] = [420, 244];

pub struct ProgressIndicator {
    source: String,
    wing_keys: Vec<(f64, bool, bool)>,
    images: Option<Vec<egui::ColorImage>>,
    textures: Option<Vec<egui::TextureHandle>>,
}

impl ProgressIndicator {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Native SVG renderers ignore CSS animation. Sample the source keyframes explicitly.
        let style_start = SVG
            .find("<style>")
            .ok_or("Missing progress animation style")?;
        let style_end = SVG
            .find("</style>")
            .ok_or("Missing progress animation style end")?;
        let keys = SVG
            .split("@keyframes left-flap {")
            .nth(1)
            .and_then(|text| text.split("@keyframes right-flap").next())
            .ok_or("Missing progress wing keyframes")?;
        let mut wing_keys = Vec::new();
        for line in keys.lines().filter(|line| line.contains('%')) {
            let (percent, transform) = line
                .trim()
                .split_once('%')
                .ok_or("Invalid progress keyframe")?;
            wing_keys.push((
                percent.parse::<f64>()? / 100.0,
                transform.contains("scaleX(.55)"),
                transform.contains(".42, 0, .72, 1"),
            ));
        }
        let source = format!(
            "{}{}",
            &SVG[..style_start],
            &SVG[style_end + "</style>".len()..]
        );
        resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default())?;
        let mut indicator = Self {
            source,
            wing_keys,
            images: None,
            textures: None,
        };
        indicator.images = Some(indicator.render_layers()?);
        Ok(indicator)
    }

    fn render_layers(
        &self,
    ) -> Result<Vec<egui::ColorImage>, Box<dyn std::error::Error + Send + Sync>> {
        let mut images = Vec::new();
        for layer in 0..4 {
            let mut source = self.source.clone();
            for (name, index) in [("left-wing-motion", 1), ("right-wing-motion", 2)] {
                if layer != index {
                    source = source.replace(&format!("class=\"{name}\""), "display=\"none\"");
                }
            }
            if layer != 3 {
                source =
                    source.replace("<g clip-path=\"url(#body-clip)\">", "<g display=\"none\">");
            }
            let (size, height) = if layer == 0 {
                source = source.replace("clip-path=\"url(#terrain-clip)\"", "")
                    .replace("viewBox=\"0 0 1200 800\"", "viewBox=\"0 0 1200 180\"")
                    .replace("height=\"800\"", "height=\"180\"")
                    .replace("<rect x=\"35\" y=\"30\" width=\"1130\" height=\"740\" fill=\"#eaf4f7\" opacity=\".035\" />", "");
                ([420, 63], 180.0)
            } else {
                source = source.replace(
                    "<g clip-path=\"url(#terrain-clip)\" aria-hidden=\"true\">",
                    "<g display=\"none\">",
                );
                (SIZE, 800.0)
            };
            let tree = resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default())?;
            let mut pixels = resvg::tiny_skia::Pixmap::new(size[0] as u32, size[1] as u32)
                .ok_or("progress image unavailable")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(
                    size[0] as f32 / 1200.0,
                    size[1] as f32 / height,
                ),
                &mut pixels.as_mut(),
            );
            images.push(egui::ColorImage::from_rgba_premultiplied(
                size,
                pixels.data(),
            ));
        }
        Ok(images)
    }
    pub fn show(&mut self, ui: &mut egui::Ui, lang: Language) {
        if self.textures.is_none() {
            self.textures = Some(
                self.images
                    .take()
                    .expect("progress layers available")
                    .into_iter()
                    .enumerate()
                    .map(|(index, image)| {
                        ui.ctx().load_texture(
                            format!("progress-layer-{index}"),
                            image,
                            if index == 0 {
                                egui::TextureOptions::LINEAR_REPEAT
                            } else {
                                egui::TextureOptions::LINEAR
                            },
                        )
                    })
                    .collect(),
            );
        }
        let textures = self.textures.as_ref().expect("progress textures available");
        let time = ui.input(|input| input.time);
        let (rect, response) =
            ui.allocate_exact_size(super::theme::WELCOME_BIRD_SIZE, egui::Sense::hover());
        response.on_hover_text(lang.t(Text::RequestInProgress));
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let terrain = terrain_mesh(
            textures[0].id(),
            rect,
            (time % GROUND_PERIOD / GROUND_PERIOD) as f32,
        );
        let mut background = terrain.clone();
        background.texture_id = egui::TextureId::default();
        for vertex in &mut background.vertices {
            vertex.uv = egui::epaint::WHITE_UV;
            vertex.color = egui::Color32::from_rgba_unmultiplied(234, 244, 247, 9);
        }
        painter.add(egui::Shape::mesh(background));
        painter.add(egui::Shape::mesh(terrain));
        let amount = self.wing_amount(time) as f32;
        painter.add(egui::Shape::mesh(wing_mesh(
            textures[1].id(),
            rect,
            amount,
            true,
        )));
        painter.add(egui::Shape::mesh(wing_mesh(
            textures[2].id(),
            rect,
            amount,
            false,
        )));
        painter.image(
            textures[3].id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // Native vsync controls cadence; an additional timer can skip display refreshes.
        ui.ctx().request_repaint();
    }
    fn wing_amount(&self, time: f64) -> f64 {
        let phase = (time % WING_PERIOD) / WING_PERIOD;
        let pair = self
            .wing_keys
            .windows(2)
            .find(|pair| phase < pair[1].0)
            .expect("Progress keyframes cover the animation period");
        let fraction = (phase - pair[0].0) / (pair[1].0 - pair[0].0);
        if pair[0].1 == pair[1].1 {
            f64::from(pair[0].1)
        } else {
            let eased = ease(
                fraction,
                if pair[0].2 {
                    (0.42, 0.72)
                } else {
                    (0.35, 0.35)
                },
            );
            if pair[0].1 { 1.0 - eased } else { eased }
        }
    }
    #[cfg(test)]
    fn frame_source(&self, time: f64) -> String {
        let amount = self.wing_amount(time);
        let scale = 1.0 - 0.45 * amount;
        self.source
            .replace(
                "class=\"terrain-flow\"",
                &format!(
                    "transform=\"translate(0 {})\"",
                    time % GROUND_PERIOD / GROUND_PERIOD * 180.0
                ),
            )
            .replace(
                "class=\"left-wing-motion\"",
                &format!(
                    "transform=\"translate(710 0) skewY({}) scale({scale} 1) translate(-710 0)\"",
                    -13.8 * amount
                ),
            )
            .replace(
                "class=\"right-wing-motion\"",
                &format!(
                    "transform=\"translate(827 0) skewY({}) scale({scale} 1) translate(-827 0)\"",
                    13.8 * amount
                ),
            )
    }
}

fn wing_mesh(texture: egui::TextureId, rect: egui::Rect, amount: f32, left: bool) -> egui::Mesh {
    let mut mesh = egui::Mesh::with_texture(texture);
    let pivot = if left { 542.0 } else { 659.0 };
    let scale = 1.0 - 0.45 * amount;
    let skew = ((if left { -13.8_f32 } else { 13.8_f32 }) * amount)
        .to_radians()
        .tan();
    for uv in [
        egui::pos2(0.0, 0.0),
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
    ] {
        let x = uv.x * 1200.0;
        let y = uv.y * 800.0;
        let point = egui::pos2(pivot + scale * (x - pivot), y + skew * scale * (x - pivot));
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.min
                + egui::vec2(
                    point.x / 1200.0 * rect.width(),
                    point.y / 800.0 * rect.height(),
                ),
            uv,
            color: egui::Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    mesh
}
fn terrain_mesh(texture: egui::TextureId, rect: egui::Rect, phase: f32) -> egui::Mesh {
    let mut mesh = egui::Mesh::with_texture(texture);
    let vertex = |x: f32, y: f32| egui::epaint::Vertex {
        pos: rect.min + egui::vec2(x / 1200.0 * rect.width(), y / 800.0 * rect.height()),
        uv: egui::pos2(x / 1200.0, y / 180.0 - phase),
        color: egui::Color32::WHITE,
    };
    mesh.vertices.push(vertex(600.0, 400.0));
    for (corner, (x, y)) in [
        (1075.0, 120.0),
        (1075.0, 680.0),
        (125.0, 680.0),
        (125.0, 120.0),
    ]
    .into_iter()
    .enumerate()
    {
        for step in 0..=8 {
            let angle = (-1.0 + corner as f32 + step as f32 / 8.0) * std::f32::consts::FRAC_PI_2;
            mesh.vertices
                .push(vertex(x + 90.0 * angle.cos(), y + 90.0 * angle.sin()));
        }
    }
    let count = mesh.vertices.len() as u32 - 1;
    for index in 1..=count {
        mesh.indices
            .extend([0, index, if index == count { 1 } else { index + 1 }]);
    }
    mesh
}

fn ease(x: f64, controls: (f64, f64)) -> f64 {
    let mut low: f64 = 0.0;
    let mut high = 1.0;
    for _ in 0..20 {
        let t = (low + high) / 2.0;
        let curve_x = 3.0 * (1.0 - t).powi(2) * t * controls.0
            + 3.0 * (1.0 - t) * t * t * controls.1
            + t.powi(3);
        if curve_x < x {
            low = t;
        } else {
            high = t;
        }
    }
    let t = (low + high) / 2.0;
    3.0 * (1.0 - t) * t * t + t.powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_reuses_textures_after_initial_upload() {
        let mut indicator = ProgressIndicator::new().unwrap();
        let context = egui::Context::default();
        for frame in 0..12 {
            let output = context.run(
                egui::RawInput {
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default()
                        .show(context, |ui| indicator.show(ui, Language::English));
                },
            );
            if frame > 1 {
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .repaint_delay
                        .is_zero()
                );
                assert!(
                    output.textures_delta.set.is_empty(),
                    "animation must not upload or rasterize new frames"
                );
            }
        }
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(210.0, 122.0));
        let first = terrain_mesh(egui::TextureId::default(), rect, 0.0);
        let second = terrain_mesh(egui::TextureId::default(), rect, 0.01);
        assert_eq!(first.vertices[0].pos, second.vertices[0].pos);
        assert_ne!(first.vertices[0].uv, second.vertices[0].uv);
        assert_ne!(
            wing_mesh(egui::TextureId::default(), rect, 0.0, true).vertices[0].pos,
            wing_mesh(egui::TextureId::default(), rect, 1.0, true).vertices[0].pos
        );
    }
    #[test]
    fn terrain_remains_visible_and_moves_at_gui_size() {
        let indicator = ProgressIndicator::new().unwrap();
        let render = |time| {
            let source = indicator.frame_source(time);
            let tree =
                resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default()).unwrap();
            let mut pixels = resvg::tiny_skia::Pixmap::new(96, 64).unwrap();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(0.08, 0.08),
                &mut pixels.as_mut(),
            );
            pixels
        };
        // Both samples are inside the initial glide; only terrain moves.
        let first = render(0.0);
        let second = render(0.3);
        let visible_changes = first
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(second.data().as_chunks::<4>().0)
            .filter(|(a, b)| a != b && a[3].abs_diff(b[3]) > 40)
            .count();
        assert!(
            visible_changes > 100,
            "Terrain must remain visible at display resolution: {visible_changes}"
        );
    }

    #[test]
    fn embedded_animation_renders_distinct_frames_throughout_a_cycle() {
        let indicator = ProgressIndicator::new().unwrap();
        let mut first = None;
        let mut changed = false;
        for frame in 0..164 {
            let source = indicator.frame_source(frame as f64 * 0.05);
            let tree =
                resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default()).unwrap();
            let mut pixels = resvg::tiny_skia::Pixmap::new(96, 64).unwrap();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(0.08, 0.08),
                &mut pixels.as_mut(),
            );
            assert!(
                pixels
                    .data()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] > 0)
            );
            if let Some(first) = &first {
                changed |= first != pixels.data();
            } else {
                first = Some(pixels.data().to_vec());
            }
        }
        assert!(changed);
    }
}
