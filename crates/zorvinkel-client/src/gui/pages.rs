//! The setup pages of a connected device.
use super::{
    i18n::{Language, Text},
    session::{Session, WifiMode},
    theme,
};
use eframe::egui::{self, Color32, RichText};
use zorvinkel_adapters::simulation::setup::DEMO_WIFI_PASSWORD;
use zorvinkel_application::{power::PowerAction, setup::*};

impl Session {
    pub(super) fn action_button(&self, ui: &mut egui::Ui, button: egui::Button<'_>) -> bool {
        ui.add_enabled(self.active_action.is_none(), button)
            .clicked()
    }
    /// The whole Wi-Fi topic on one page: a switch between joining a network and the device's
    /// own Wi-Fi, and one place for what happens after a restart.
    pub fn wifi_page(&mut self, ui: &mut egui::Ui, lang: Language) {
        let join = self.simulation || self.snapshot.wifi_available;
        let own = self.simulation || self.snapshot.access_point_available;
        let own_active = matches!(
            self.snapshot.status.as_ref().map(|status| &status.network),
            Some(zorvinkel_core::NetworkStatus::AccessPointActive { .. })
        );
        let mut mode = self.wifi_mode.unwrap_or(if own_active && own {
            WifiMode::OwnNetwork
        } else {
            WifiMode::Join
        });
        if !own {
            mode = WifiMode::Join;
        } else if !join {
            mode = WifiMode::OwnNetwork;
        } else {
            theme::segmented(
                ui,
                &mut mode,
                &[
                    (WifiMode::Join, lang.t(Text::JoinWifi)),
                    (WifiMode::OwnNetwork, lang.t(Text::ProvideOwnWifi)),
                ],
            );
            self.wifi_mode = Some(mode);
            ui.add_space(theme::space::M);
        }
        match mode {
            WifiMode::Join => self.wifi(ui, lang),
            WifiMode::OwnNetwork => self.access_point(ui, lang),
        }
        self.restart_choice(ui, lang);
    }
    fn wifi(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(lang.t(Text::ChooseNetwork));
        if !self.snapshot.networks.is_empty() {
            theme::choice_list(ui, |ui| {
                for name in self.snapshot.networks.clone() {
                    if theme::list_row(ui, self.ssid == name, &name).clicked() {
                        self.ssid = name;
                        self.password.clear();
                    }
                }
            });
        }
        if self.action_button(ui, egui::Button::new(lang.t(Text::RefreshNetworks))) {
            self.submit(SetupRequest::Scan);
        }
        ui.add_space(theme::space::S);
        ui.separator();
        ui.add_space(theme::space::S);
        ui.label(lang.t(Text::WifiName));
        ui.add(
            theme::text_field(egui::TextEdit::singleline(&mut self.ssid))
                .char_limit(32)
                .desired_width(ui.available_width().min(theme::size::FIELD_WIDTH)),
        );
        ui.label(lang.t(Text::Password));
        password_field(
            ui,
            egui::TextEdit::singleline(&mut self.password)
                .char_limit(63)
                .password(!self.show_wifi_password),
            &mut self.show_wifi_password,
            lang,
        );
        if self.action_button(ui, egui::Button::new(lang.t(Text::GetSystemPassword))) {
            self.submit(SetupRequest::ReadSavedCredentials {
                ssid: self.ssid.clone(),
            });
        }
        if self.simulation {
            hint(ui, &lang.format(Text::DemoPassword, &[&DEMO_WIFI_PASSWORD]));
            hint(ui, lang.t(Text::DemoWrongPassword));
        } else {
            hint(ui, lang.t(Text::Wpa2Supported));
        }
        ui.add_space(theme::space::M);
        if self.action_button(ui, theme::primary_button(lang.t(Text::JoinWifi))) {
            let password = std::mem::take(&mut self.password);
            self.submit(SetupRequest::ConnectWifi {
                ssid: self.ssid.clone(),
                password,
            });
        }
    }
    /// Revert / Persist: offered while the device runs a configuration that is not the one it
    /// would come back with after a restart. Revert switches back to the persisted one at once;
    /// Persist makes the running one the persisted one.
    fn restart_choice(&mut self, ui: &mut egui::Ui, lang: Language) {
        if !self.snapshot.restart_differs {
            return;
        }
        ui.add_space(theme::space::S);
        let mut request = None;
        ui.horizontal(|ui| {
            let idle = self.active_action.is_none();
            if ui
                .add_enabled(idle, egui::Button::new(lang.t(Text::RestartRevert)))
                .on_hover_text(lang.t(Text::RestartHintOriginal))
                .clicked()
            {
                request = Some(SetupRequest::RestartRevert);
            }
            if ui
                .add_enabled(idle, theme::primary_button(lang.t(Text::RestartPersist)))
                .on_hover_text(lang.t(Text::RestartHintKeep))
                .clicked()
            {
                request = Some(SetupRequest::RestartCurrentAsDefault);
            }
        });
        if let Some(request) = request {
            self.submit(request);
        }
    }
    fn access_point(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(lang.t(Text::AccessPointIntro));
        let Some(stored) = self.snapshot.access_point.clone() else {
            ui.add_space(theme::space::S);
            ui.label(lang.t(Text::AccessPointNoCredentials));
            self.access_point_fields(ui, lang);
            return;
        };
        let active = self.snapshot.status.as_ref().is_some_and(|s| {
            matches!(
                s.network,
                zorvinkel_core::NetworkStatus::AccessPointActive { .. }
            )
        });
        let (state, color) = if active {
            (Text::AccessPointOn, theme::SUCCESS)
        } else {
            (Text::AccessPointOff, theme::TEXT_MUTED)
        };
        ui.label(RichText::new(lang.t(state)).color(color));
        ui.add_space(theme::space::S);
        self.access_point_fields(ui, lang);
        let valid = validate_access_point(&self.ap_ssid, &self.ap_password).is_ok();
        let changed = self.ap_ssid != stored.ssid || self.ap_password != stored.password;
        if !valid {
            ui.label(
                RichText::new(lang.t(Text::AccessPointInvalid))
                    .small()
                    .color(theme::DANGER),
            );
        } else if self.snapshot.access_point_proposed && !active {
            hint(ui, lang.t(Text::AccessPointProposal));
        }
        ui.add_space(theme::space::M);
        ui.horizontal_wrapped(|ui| {
            let settings = AccessPointSettings {
                ssid: self.ap_ssid.clone(),
                password: self.ap_password.clone(),
            };
            if active {
                if changed
                    && ui
                        .add_enabled(
                            valid && self.active_action.is_none(),
                            theme::primary_button(lang.t(Text::AccessPointApply)),
                        )
                        .clicked()
                {
                    self.submit(SetupRequest::UpdateAccessPoint(settings));
                }
                if self.action_button(ui, egui::Button::new(lang.t(Text::AccessPointTurnOff))) {
                    self.submit(SetupRequest::DisableAccessPoint);
                }
            } else if ui
                .add_enabled(
                    valid && self.active_action.is_none(),
                    theme::primary_button(lang.t(Text::AccessPointTurnOn)),
                )
                .clicked()
            {
                self.submit(SetupRequest::EnableAccessPoint(settings));
            }
        });
        // The code shows what the device reports, so it only appears once the network stands.
        if active {
            ui.add_space(theme::space::S);
            ui.separator();
            ui.add_space(theme::space::S);
            ui.horizontal(|ui| {
                draw_qr(ui, &wifi_qr(&stored), 190.0, lang);
                ui.add_space(theme::space::S);
                ui.vertical(|ui| {
                    theme::label_value(ui, lang.t(Text::WifiName), RichText::new(&stored.ssid));
                    theme::label_value(
                        ui,
                        lang.t(Text::Password),
                        RichText::new(&stored.password).monospace(),
                    );
                    ui.add_space(theme::space::XS);
                    hint(ui, lang.t(Text::ScanToJoin));
                });
            });
        }
    }
    fn access_point_fields(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(lang.t(Text::WifiName));
        ui.add(
            theme::text_field(egui::TextEdit::singleline(&mut self.ap_ssid))
                .char_limit(32)
                .desired_width(ui.available_width().min(theme::size::FIELD_WIDTH)),
        );
        ui.label(lang.t(Text::Password));
        password_field(
            ui,
            egui::TextEdit::singleline(&mut self.ap_password)
                .char_limit(63)
                .password(!self.show_ap_password)
                .font(egui::TextStyle::Monospace),
            &mut self.show_ap_password,
            lang,
        );
    }
    pub fn power(&mut self, ui: &mut egui::Ui, lang: Language) {
        theme::section_title(ui, lang.t(Text::PowerHeading));
        ui.label(lang.t(Text::PowerEndsConnection));
        ui.add_space(theme::space::S);
        let Some(action) = self.confirm_power else {
            ui.horizontal_wrapped(|ui| {
                if self.action_button(ui, egui::Button::new(lang.t(Text::Restart))) {
                    self.confirm_power = Some(PowerAction::Restart);
                }
                if self.action_button(ui, egui::Button::new(lang.t(Text::Shutdown))) {
                    self.confirm_power = Some(PowerAction::Shutdown);
                }
            });
            return;
        };
        let (question, confirm) = match action {
            PowerAction::Restart => (Text::ConfirmRestartDevice, Text::YesRestart),
            PowerAction::Shutdown => (Text::ConfirmShutdownDevice, Text::YesShutdown),
        };
        ui.label(lang.t(question));
        ui.horizontal_wrapped(|ui| {
            if self.action_button(ui, theme::danger_button(lang.t(confirm))) {
                self.confirm_power = None;
                self.submit(SetupRequest::PowerDevice(action));
            }
            if self.action_button(ui, egui::Button::new(lang.t(Text::Cancel))) {
                self.confirm_power = None;
            }
        });
    }
}
fn wifi_qr(ap: &AccessPointSettings) -> String {
    fn escape(value: &str) -> String {
        value
            .chars()
            .flat_map(|c| {
                if "\\;,:\"".contains(c) {
                    vec!['\\', c]
                } else {
                    vec![c]
                }
            })
            .collect()
    }
    format!(
        "WIFI:T:WPA;S:{};P:{};;",
        escape(&ap.ssid),
        escape(&ap.password)
    )
}
/// A hint below a field or an action.
pub(super) fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(theme::TEXT_MUTED)).wrap());
}

/// Password field with the eye button beside it; both together are as wide as a plain field.
fn password_field(ui: &mut egui::Ui, edit: egui::TextEdit<'_>, visible: &mut bool, lang: Language) {
    ui.horizontal(|ui| {
        let width = ui.available_width().min(theme::size::FIELD_WIDTH)
            - theme::size::CONTROL_HEIGHT
            - ui.spacing().item_spacing.x;
        ui.add(theme::text_field(edit).desired_width(width.max(0.0)));
        visibility_toggle(ui, visible, lang);
    });
}

/// Square eye button that toggles password visibility. The icon is painted
/// because the bundled egui fonts have no eye glyph.
fn visibility_toggle(ui: &mut egui::Ui, visible: &mut bool, lang: Language) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(theme::size::CONTROL_HEIGHT, theme::size::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    if response.clicked() {
        *visible = !*visible;
    }
    let hint = if *visible {
        Text::HidePassword
    } else {
        Text::ShowPassword
    };
    let response = response.on_hover_text(lang.t(hint));
    let style = ui.style().interact(&response);
    let painter = ui.painter();
    painter.rect(
        rect,
        style.corner_radius,
        style.weak_bg_fill,
        style.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let center = rect.center();
    let (half_width, half_height) = (8.0, 4.5);
    let stroke = egui::Stroke::new(1.5_f32, style.fg_stroke.color);
    let outline: Vec<egui::Pos2> = (0..32)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 32.0;
            center + egui::vec2(half_width * angle.cos(), half_height * angle.sin())
        })
        .collect();
    painter.add(egui::Shape::closed_line(outline, stroke));
    painter.circle_filled(center, 2.2, style.fg_stroke.color);
    if *visible {
        let slash_from = center + egui::vec2(-7.0, 6.0);
        let slash_to = center + egui::vec2(7.0, -6.0);
        painter.line_segment([slash_from, slash_to], stroke);
    }
}

/// Draws `text` as QR code of about `size` points, with the quiet zone the scanners expect.
pub(super) fn draw_qr(ui: &mut egui::Ui, text: &str, size: f32, lang: Language) {
    let Ok(code) = qrcode::QrCode::new(text) else {
        ui.label(lang.t(Text::QrFailed));
        return;
    };
    let modules = code.width();
    let unit = (size / (modules + 8) as f32).floor();
    let size = (modules + 8) as f32 * unit;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    for y in 0..modules {
        for x in 0..modules {
            if code[(x, y)] == qrcode::Color::Dark {
                let start = rect.min + egui::vec2((x + 4) as f32 * unit, (y + 4) as f32 * unit);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(start, egui::vec2(unit, unit)),
                    0.0,
                    Color32::BLACK,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::new("simulation:device-1".into(), "Fotobox 01".into(), false)
    }

    #[test]
    fn wifi_qr_escapes_reserved_characters_and_uses_current_device_credentials() {
        let settings = AccessPointSettings {
            ssid: "Demo;WLAN".into(),
            password: "secret:demo".into(),
        };
        assert_eq!(
            wifi_qr(&settings),
            "WIFI:T:WPA;S:Demo\\;WLAN;P:secret\\:demo;;"
        );
    }
    #[test]
    fn access_point_page_offers_editable_proposal_and_shows_qr_only_when_active() {
        let mut app = session();
        app.snapshot.access_point_available = true;
        app.snapshot.access_point_proposed = true;
        app.snapshot.access_point = Some(AccessPointSettings {
            ssid: "Zorvinkel-Box".into(),
            password: "A".repeat(63),
        });
        app.ap_ssid = "Zorvinkel-Box".into();
        app.ap_password = "A".repeat(63);
        let context = egui::Context::default();
        let render = |app: &mut Session| {
            let _ = context.run(egui::RawInput::default(), |context| {
                egui::CentralPanel::default()
                    .show(context, |ui| app.access_point(ui, Language::English));
            });
        };
        render(&mut app);
        // Editing is possible and an invalid edit is not submitted.
        app.ap_password = "short".into();
        render(&mut app);
        assert!(app.take_outbox().is_empty());
        assert!(validate_access_point(&app.ap_ssid, &app.ap_password).is_err());
        app.snapshot.status = Some(zorvinkel_core::DeviceStatus {
            network: zorvinkel_core::NetworkStatus::AccessPointActive {
                ssid: "Zorvinkel-Box".into(),
            },
            ip_reachability: zorvinkel_core::IpReachability::Unverified,
        });
        render(&mut app);
        app.snapshot.access_point = None;
        render(&mut app);
    }
    #[test]
    fn action_row_height_is_stable_while_a_request_runs() {
        let mut app = session();
        let context = egui::Context::default();
        let mut heights = Vec::new();
        for active in [false, true] {
            app.active_action = active.then_some(super::super::session::Action::Scan);
            let _ = context.run(egui::RawInput::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let top = ui.cursor().top();
                    app.action_button(ui, egui::Button::new("Refresh"));
                    heights.push(ui.cursor().top() - top);
                });
            });
        }
        assert!(
            heights
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < 0.1)
        );
    }
}
