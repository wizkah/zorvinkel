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
            ui.add_space(8.0);
        }
        match mode {
            WifiMode::Join => self.wifi(ui, lang),
            WifiMode::OwnNetwork => self.access_point(ui, lang),
        }
        self.restart_choice(ui, lang);
    }
    fn wifi(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(lang.t(Text::ChooseNetwork));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for name in self.snapshot.networks.clone() {
                if ui.selectable_label(self.ssid == name, &name).clicked() {
                    self.ssid = name;
                    self.password.clear();
                }
            }
        });
        if self.action_button(ui, egui::Button::new(lang.t(Text::RefreshNetworks))) {
            self.submit(SetupRequest::Scan);
        }
        ui.separator();
        ui.label(lang.t(Text::WifiName));
        ui.add(
            egui::TextEdit::singleline(&mut self.ssid)
                .char_limit(32)
                .desired_width(ui.available_width().min(420.0)),
        );
        ui.label(lang.t(Text::Password));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.password)
                    .char_limit(63)
                    .password(!self.show_wifi_password)
                    .desired_width((ui.available_width() - 36.0).clamp(0.0, 384.0)),
            );
            visibility_toggle(ui, &mut self.show_wifi_password, lang);
        });
        ui.horizontal_wrapped(|ui| {
            if self.action_button(ui, egui::Button::new(lang.t(Text::GetSystemPassword))) {
                self.submit(SetupRequest::ReadSavedCredentials {
                    ssid: self.ssid.clone(),
                });
            }
            if self.action_button(ui, egui::Button::new(lang.t(Text::UseCachedPassword))) {
                self.submit(SetupRequest::ReadCachedCredentials {
                    ssid: self.ssid.clone(),
                });
            }
        });
        if self.simulation {
            ui.small(lang.format(Text::DemoPassword, &[&DEMO_WIFI_PASSWORD]));
            ui.small(lang.t(Text::DemoWrongPassword));
        } else {
            ui.small(lang.t(Text::Wpa2Supported));
        }
        ui.add_space(8.0);
        if self.action_button(ui, theme::primary_button(lang.t(Text::JoinWifi))) {
            let password = std::mem::take(&mut self.password);
            self.submit(SetupRequest::ConnectWifi {
                ssid: self.ssid.clone(),
                password,
            });
        }
    }
    /// The one place for what the device does after a restart: back to its original settings,
    /// or keep the current one, either following later changes or fixed as the default.
    fn restart_choice(&mut self, ui: &mut egui::Ui, lang: Language) {
        use zorvinkel_core::network::RestartPolicy;
        let persistence = self.snapshot.wifi_persistence;
        let default_offered = self.snapshot.restart_policy.is_some();
        if persistence.is_none() && !default_offered {
            return;
        }
        let fixed = matches!(
            self.snapshot.restart_policy,
            Some(RestartPolicy::DefaultProfile(_))
        );
        let following = !fixed && persistence == Some(true);
        let has_network = !matches!(
            self.snapshot.status.as_ref().map(|status| &status.network),
            None | Some(zorvinkel_core::NetworkStatus::Disconnected)
        );
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new(lang.t(Text::AfterRestart)).strong());
        let kept = fixed || following;
        let mut keep = kept;
        let mut follow = following || (!fixed && persistence.is_none());
        ui.add_enabled_ui(self.active_action.is_none(), |ui| {
            if ui
                .radio(
                    !kept,
                    if persistence.is_some() {
                        lang.t(Text::RestartUseOriginal)
                    } else {
                        lang.t(Text::RestartUseLastAccepted)
                    },
                )
                .clicked()
            {
                keep = false;
            }
            if ui.radio(kept, lang.t(Text::RestartKeepCurrent)).clicked() {
                keep = true;
            }
            // Following later changes needs the keep switch; a fixed default needs a network.
            if kept && persistence.is_some() && default_offered {
                ui.add_enabled_ui(has_network || fixed, |ui| {
                    ui.checkbox(&mut follow, lang.t(Text::RestartFollowChanges));
                });
            }
        });
        ui.small(lang.t(if !kept {
            Text::RestartHintOriginal
        } else if fixed {
            Text::RestartHintFixed
        } else {
            Text::RestartHintKeep
        }));
        if fixed
            && has_network
            && self.action_button(ui, egui::Button::new(lang.t(Text::RestartSetAsDefault)))
        {
            self.submit(SetupRequest::RestartCurrentAsDefault);
        }
        let request = if !keep && kept {
            // Leaving "keep" also ends a fixed default.
            Some(if persistence.is_some() {
                SetupRequest::SetWifiPersistence(false)
            } else {
                SetupRequest::RestartLastAccepted
            })
        } else if keep && !kept {
            // Without the choice to follow, the current setting is simply the default.
            Some(if persistence.is_some() {
                SetupRequest::SetWifiPersistence(true)
            } else {
                SetupRequest::RestartCurrentAsDefault
            })
        } else if keep && follow != (following || (!fixed && persistence.is_none())) {
            Some(if follow {
                SetupRequest::SetWifiPersistence(true)
            } else {
                SetupRequest::RestartCurrentAsDefault
            })
        } else {
            None
        };
        if let Some(request) = request {
            self.submit(request);
        }
        if self.simulation {
            ui.small(lang.t(Text::RestartSimulationHint));
        }
    }
    fn access_point(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(lang.t(Text::AccessPointIntro));
        let Some(stored) = self.snapshot.access_point.clone() else {
            ui.add_space(8.0);
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
        ui.add_space(8.0);
        ui.label(lang.t(if active {
            Text::AccessPointOn
        } else {
            Text::AccessPointOff
        }));
        ui.add_space(8.0);
        self.access_point_fields(ui, lang);
        let valid = validate_access_point(&self.ap_ssid, &self.ap_password).is_ok();
        let changed = self.ap_ssid != stored.ssid || self.ap_password != stored.password;
        if !valid {
            ui.small(lang.t(Text::AccessPointInvalid));
        } else if self.snapshot.access_point_proposed && !active {
            ui.small(lang.t(Text::AccessPointProposal));
        }
        ui.add_space(8.0);
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
            ui.separator();
            ui.horizontal(|ui| {
                draw_qr(ui, &wifi_qr(&stored), 190.0, lang);
                ui.vertical(|ui| {
                    ui.label(lang.t(Text::WifiName));
                    ui.strong(&stored.ssid);
                    ui.label(lang.t(Text::Password));
                    ui.monospace(&stored.password);
                    ui.add_space(4.0);
                    ui.small(lang.t(Text::ScanToJoin));
                });
            });
        }
    }
    fn access_point_fields(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(lang.t(Text::WifiName));
        ui.add(
            egui::TextEdit::singleline(&mut self.ap_ssid)
                .char_limit(32)
                .desired_width(ui.available_width().min(420.0)),
        );
        ui.label(lang.t(Text::Password));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.ap_password)
                    .char_limit(63)
                    .password(!self.show_ap_password)
                    .font(egui::TextStyle::Monospace)
                    .desired_width((ui.available_width() - 40.0).min(420.0)),
            );
            visibility_toggle(ui, &mut self.show_ap_password, lang);
        });
    }
    pub fn power(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.heading(lang.t(Text::PowerHeading));
        ui.label(lang.t(Text::PowerEndsConnection));
        ui.add_space(8.0);
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
            if self.action_button(ui, theme::primary_button(lang.t(confirm))) {
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
/// Small eye button that toggles password visibility. The icon is painted
/// because the bundled egui fonts have no eye glyph.
fn visibility_toggle(ui: &mut egui::Ui, visible: &mut bool, lang: Language) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
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
    painter.rect_filled(rect, style.corner_radius, style.weak_bg_fill);

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
