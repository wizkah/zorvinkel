//! The "Devices" tab: a scrollable grid with one tile per device.
use super::{
    device_tab::state_label,
    i18n::{Language, Text},
    session::{ConnectionState, Session},
    theme,
};
use eframe::egui::{self, RichText};

const TILE: egui::Vec2 = egui::vec2(270.0, 124.0);
const AVATAR: f32 = 56.0;
const TILE_PADDING: f32 = 12.0;

pub enum TileEvent {
    /// Open the device tab without connecting.
    Open,
    Connect,
    Disconnect,
}

/// Draws all tiles and returns what the user did with which device.
pub fn grid(ui: &mut egui::Ui, sessions: &[Session], lang: Language) -> Vec<(usize, TileEvent)> {
    let mut events = Vec::new();
    let gap = ui.spacing().item_spacing.x;
    let columns = (((ui.available_width() + gap) / (TILE.x + gap)) as usize).max(1);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (row, chunk) in sessions.chunks(columns).enumerate() {
                ui.horizontal_top(|ui| {
                    for (offset, session) in chunk.iter().enumerate() {
                        if let Some(event) = tile(ui, session, lang) {
                            events.push((row * columns + offset, event));
                        }
                    }
                });
            }
        });
    events
}

fn tile(ui: &mut egui::Ui, session: &Session, lang: Language) -> Option<TileEvent> {
    let (rect, background) = ui.allocate_exact_size(TILE, egui::Sense::click());
    ui.painter().rect(
        rect,
        10,
        if background.hovered() {
            theme::BLUE_PALE
        } else {
            egui::Color32::WHITE
        },
        egui::Stroke::new(1.0_f32, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(TILE_PADDING);
    let state = session.state();
    let avatar = egui::Rect::from_min_size(inner.min, egui::vec2(AVATAR, AVATAR));
    theme::avatar(
        &mut ui.new_child(egui::UiBuilder::new().max_rect(avatar)),
        &session.name,
        AVATAR,
    );
    let text = egui::Rect::from_min_max(
        egui::pos2(avatar.max.x + TILE_PADDING, inner.min.y),
        inner.max,
    );
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    column.spacing_mut().item_spacing.y = 4.0;
    column.add(egui::Label::new(RichText::new(&session.name).strong()).truncate());
    column.horizontal(|ui| {
        state_label(ui, state, lang);
        if let Some(path) = session.path_text() {
            ui.label(
                RichText::new(format!("· {}", lang.t(path)))
                    .small()
                    .color(theme::MUTED),
            );
        }
    });
    column.add_space(2.0);
    let event = connection_button(&mut column, session, state, text.width(), lang);
    if background.double_clicked() {
        return Some(TileEvent::Open);
    }
    event
}

fn connection_button(
    ui: &mut egui::Ui,
    session: &Session,
    state: ConnectionState,
    width: f32,
    lang: Language,
) -> Option<TileEvent> {
    let connected = state == ConnectionState::Connected;
    let enabled = session.active_action.is_none() && (connected || session.in_range);
    let button = if connected {
        egui::Button::new(lang.t(Text::Disconnect))
    } else {
        theme::primary_button(lang.t(Text::Connect))
    };
    ui.add_enabled(enabled, button.min_size(egui::vec2(width, 28.0)))
        .clicked()
        .then_some(if connected {
            TileEvent::Disconnect
        } else {
            TileEvent::Connect
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_tiles_render_in_a_scroll_area() {
        let sessions: Vec<_> = (1..=30)
            .map(|n| Session::new(format!("d{n}"), format!("Photo booth {n:02}"), true))
            .collect();
        let context = egui::Context::default();
        let mut events = Vec::new();
        for _ in 0..2 {
            let _ = context.run(egui::RawInput::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    events = grid(ui, &sessions, Language::English);
                });
            });
        }
        assert!(events.is_empty());
    }
}
