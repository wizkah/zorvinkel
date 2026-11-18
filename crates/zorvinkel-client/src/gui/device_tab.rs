//! Content of a device tab: connection control, messages and the setup pages.
use super::{
    i18n::{Language, Text},
    session::{ConnectionState, Page, Session},
    theme,
};
use eframe::egui::{self, RichText};
use zorvinkel_application::setup::{Availability, Function, SetupError};
use zorvinkel_core::IpReachability;

pub fn state_color(state: ConnectionState) -> egui::Color32 {
    match state {
        ConnectionState::Connected => theme::GREEN,
        ConnectionState::Working(_) => theme::AMBER,
        ConnectionState::Failed => theme::ERROR,
        ConnectionState::Disconnected | ConnectionState::OutOfRange => theme::MUTED,
    }
}
pub fn state_text(state: ConnectionState) -> Text {
    match state {
        ConnectionState::Connected => Text::StateConnected,
        ConnectionState::Working(action) => action.label(),
        ConnectionState::Failed => Text::StateFailed,
        ConnectionState::Disconnected => Text::StateDisconnected,
        ConnectionState::OutOfRange => Text::StateOutOfRange,
    }
}
/// State dot, text and a spinner while a request runs.
pub fn state_label(ui: &mut egui::Ui, state: ConnectionState, lang: Language) {
    if matches!(state, ConnectionState::Working(_)) {
        ui.add(egui::Spinner::new().size(12.0).color(theme::AMBER));
    } else {
        theme::status_dot(ui, state_color(state));
    }
    ui.label(RichText::new(lang.t(state_text(state))).color(state_color(state)));
}

impl Session {
    pub fn show_tab(&mut self, ui: &mut egui::Ui, lang: Language) {
        self.header(ui, lang);
        if let Some(error) = self.error {
            ui.add_space(8.0);
            theme::message(
                ui,
                lang.t(error_text(error)),
                theme::ERROR,
                theme::ERROR_PALE,
            );
        } else if let Some(seconds) = self.expiry_warning() {
            ui.add_space(8.0);
            theme::message(
                ui,
                &lang.format(Text::SessionEndsIn, &[&seconds]),
                theme::AMBER,
                theme::BLUE_PALE,
            );
            ui.add_space(4.0);
            if self.active_action.is_none() && ui.button(lang.t(Text::RenewNow)).clicked() {
                self.connect();
            }
        } else if let Some(notice) = self.notice {
            ui.add_space(8.0);
            theme::message(ui, lang.t(notice.text()), theme::BLUE, theme::BLUE_PALE);
        }
        ui.add_space(12.0);
        if !self.is_connected() {
            return;
        }
        if !self.simulation
            && !self.snapshot.wifi_available
            && !self.snapshot.power_available
            && !self.snapshot.system_info_available
        {
            theme::card().show(ui, |ui| {
                ui.label(lang.t(Text::NoWifiSetupOffered));
            });
            return;
        }
        self.show_pages(ui, lang);
    }
    fn header(&mut self, ui: &mut egui::Ui, lang: Language) {
        let state = self.state();
        ui.horizontal(|ui| {
            theme::avatar(ui, &self.name, 40.0);
            ui.heading(&self.name);
            state_label(ui, state, lang);
            if let Some(path) = self.path_text() {
                ui.label(
                    RichText::new(format!("· {}", lang.t(path)))
                        .small()
                        .color(theme::MUTED),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.connection_button(ui, state, lang);
            });
        });
    }
    pub fn connection_button(&mut self, ui: &mut egui::Ui, state: ConnectionState, lang: Language) {
        let connected = state == ConnectionState::Connected;
        let enabled = self.active_action.is_none() && (connected || self.in_range);
        let button = if connected {
            egui::Button::new(lang.t(Text::Disconnect))
        } else {
            theme::primary_button(lang.t(Text::Connect))
        };
        if ui.add_enabled(enabled, button).clicked() {
            if connected {
                self.disconnect();
            } else {
                self.connect();
            }
        }
    }
    fn show_pages(&mut self, ui: &mut egui::Ui, lang: Language) {
        let wifi_offered =
            self.simulation || self.snapshot.wifi_available || self.snapshot.access_point_available;
        let system_offered = self.snapshot.system_info_available;
        let groups = self.service_groups();
        if (!wifi_offered && self.page == Page::Wifi)
            || (!system_offered && self.page == Page::System)
            || (self.page == Page::Services && !groups.contains(&self.service_group))
            || (!self.snapshot.power_available && self.page == Page::Power)
        {
            self.page = if wifi_offered {
                Page::Wifi
            } else if system_offered {
                Page::System
            } else {
                Page::Power
            };
            self.confirm_power = None;
        }
        ui.add_enabled_ui(self.active_action.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if wifi_offered {
                    ui.selectable_value(&mut self.page, Page::Wifi, lang.t(Text::PageWifi));
                }
                if system_offered {
                    ui.selectable_value(&mut self.page, Page::System, lang.t(Text::PageSystem));
                }
                for group in &groups {
                    let selected = self.page == Page::Services && &self.service_group == group;
                    if ui.selectable_label(selected, group).clicked() {
                        self.page = Page::Services;
                        self.service_group = group.clone();
                    }
                }
                if self.snapshot.power_available {
                    ui.selectable_value(&mut self.page, Page::Power, lang.t(Text::PageDevice));
                }
            });
        });
        ui.add_space(8.0);
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            match self.page {
                Page::Wifi => self.wifi_page(ui, lang),
                Page::System => self.system(ui, lang),
                Page::Services => self.services(ui, lang),
                Page::Power => self.power(ui, lang),
            }
        });
    }
    /// Connection details shown beside the pages.
    pub fn show_status(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.label(
            RichText::new(lang.t(Text::ConnectionStatus))
                .small()
                .strong()
                .color(theme::MUTED),
        );
        ui.add_space(4.0);
        ui.label(lang.t(Text::ConnectionPath));
        ui.strong(self.path_text().map_or("–", |path| lang.t(path)));
        ui.add_space(12.0);
        ui.label(lang.t(Text::LocalAccess));
        ui.strong(lang.t(if self.is_connected() {
            Text::StateConnected
        } else {
            Text::NotConnected
        }));
        if let Some(status) = &self.snapshot.status {
            ui.add_space(12.0);
            ui.label(lang.t(Text::NetworkOnDevice));
            ui.strong(network_label(&status.network, lang));
            ui.add_space(12.0);
            ui.label(lang.t(Text::ReachableFromComputer));
            let (text, color) = reachability_label(status.ip_reachability);
            ui.label(RichText::new(lang.t(text)).strong().color(color));
        }
        if self.is_connected() {
            ui.add_space(12.0);
            ui.label(lang.t(Text::FunctionsOverConnection));
            for (function, availability) in self.snapshot.functions() {
                let (mark, color) = availability_style(availability);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(mark).color(color));
                    ui.label(RichText::new(lang.t(function_label(function))).color(color));
                });
                if availability == Availability::NotOffered {
                    ui.label(
                        RichText::new(lang.t(Text::NotOfferedByDevice))
                            .small()
                            .color(theme::MUTED),
                    );
                }
            }
        }
        if self.is_connected() && !self.simulation {
            ui.add_space(12.0);
            ui.checkbox(
                &mut self.auto_renew,
                lang.t(Text::RenewSessionAutomatically),
            )
            .on_hover_text(lang.t(Text::RenewSessionHint));
        }
    }
}

fn function_label(function: Function) -> Text {
    match function {
        Function::Wifi => Text::FunctionWifi,
        Function::AccessPoint => Text::FunctionAccessPoint,
        Function::RestartOption => Text::FunctionRestartOption,
        Function::System => Text::FunctionSystem,
        Function::Services => Text::FunctionServices,
        Function::ServiceActions => Text::FunctionServiceActions,
        Function::Power => Text::FunctionPower,
    }
}
fn availability_style(availability: Availability) -> (&'static str, egui::Color32) {
    match availability {
        Availability::Available => ("✓", theme::GREEN),
        Availability::NotOffered => ("–", theme::MUTED),
    }
}

fn error_text(error: SetupError) -> Text {
    match error {
        SetupError::IdentityRejected => Text::ErrorIdentityRejected,
        SetupError::AccessDenied => Text::ErrorAccessDenied,
        SetupError::Busy => Text::ErrorBusy,
        SetupError::Unsupported => Text::ErrorUnsupported,
        SetupError::RecoveryFailed => Text::ErrorRecoveryFailed,
        SetupError::Disconnected => Text::ErrorDisconnected,
        SetupError::InvalidInput => Text::ErrorInvalidInput,
        SetupError::ConnectionRejected => Text::ErrorConnectionRejected,
        SetupError::SessionExpired => Text::ErrorSessionExpired,
        SetupError::NoActiveNetwork => Text::ErrorNoActiveNetwork,
        SetupError::Unavailable => Text::ErrorUnavailable,
        SetupError::CredentialsUnavailable => Text::ErrorCredentialsUnavailable,
    }
}
fn reachability_label(reachability: IpReachability) -> (Text, egui::Color32) {
    match reachability {
        IpReachability::Unverified => (Text::ReachabilityUnverified, theme::MUTED),
        IpReachability::Authenticated => (Text::ReachabilityAuthenticated, theme::GREEN),
        IpReachability::Unreachable => (Text::ReachabilityUnreachable, theme::AMBER),
    }
}

pub(super) fn network_label(status: &zorvinkel_core::NetworkStatus, lang: Language) -> String {
    match status {
        zorvinkel_core::NetworkStatus::Disconnected => lang.t(Text::NotConnected).into(),
        zorvinkel_core::NetworkStatus::WifiConnected { ssid } => {
            lang.format(Text::NetworkJoined, &[ssid])
        }
        zorvinkel_core::NetworkStatus::AccessPointActive { ssid } => {
            lang.format(Text::NetworkOwn, &[ssid])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachability_is_labelled_without_claiming_more_than_was_checked() {
        let label = |reachability| Language::English.t(reachability_label(reachability).0);
        assert_eq!(label(IpReachability::Unverified), "Not checked yet");
        assert_eq!(
            label(IpReachability::Authenticated),
            "Reachable and confirmed"
        );
        assert_eq!(label(IpReachability::Unreachable), "Not reachable");
    }

    fn render(session: &mut Session) {
        render_in(session, Language::English);
    }
    fn render_in(session: &mut Session, lang: Language) {
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| session.show_tab(ui, lang));
        });
    }

    #[test]
    fn tab_renders_in_every_state_and_language() {
        let mut session = Session::new("h".into(), "Photo booth".into(), true);
        for lang in Language::ALL {
            render_in(&mut session, lang);
            session.in_range = false;
            render_in(&mut session, lang);
            session.error = Some(SetupError::Unavailable);
            render_in(&mut session, lang);
            session.connect();
            render_in(&mut session, lang);
            session.active_action = None;
            session.error = None;
            session.in_range = true;
        }
    }
    #[test]
    fn network_labels_name_the_network_in_the_chosen_language() {
        let joined = zorvinkel_core::NetworkStatus::WifiConnected {
            ssid: "Home".into(),
        };
        assert_eq!(
            network_label(&joined, Language::English),
            "Connected to Home"
        );
        assert_eq!(
            network_label(&joined, Language::German),
            "Mit Home verbunden"
        );
    }

    #[test]
    fn each_service_group_gets_its_own_page_and_vanished_groups_fall_back() {
        use zorvinkel_adapters::simulation::setup::SimulatedSetup;
        use zorvinkel_application::setup::{SetupBackend, SetupRequest};
        let mut backend = SimulatedSetup::default();
        backend.handle(SetupRequest::Discover).unwrap();
        let snapshot = backend
            .handle(SetupRequest::Connect("simulation:device-1".into()))
            .unwrap()
            .snapshot;
        let mut session = Session::new("h".into(), "Photo booth".into(), true);
        session.snapshot = snapshot;
        assert_eq!(session.service_groups(), ["Music", "Photo booth"]);
        session.page = Page::Services;
        session.service_group = "Music".into();
        render(&mut session);
        assert_eq!(session.page, Page::Services);
        session.snapshot.services.clear();
        render(&mut session);
        assert_ne!(session.page, Page::Services);
    }

    #[test]
    fn stopping_and_restarting_ask_first_while_starting_does_not() {
        use zorvinkel_adapters::simulation::setup::SimulatedSetup;
        use zorvinkel_application::services::{ServiceAction, ServiceState};
        use zorvinkel_application::setup::{SetupBackend, SetupRequest};
        let mut backend = SimulatedSetup::default();
        backend.handle(SetupRequest::Discover).unwrap();
        let snapshot = backend
            .handle(SetupRequest::Connect("simulation:device-1".into()))
            .unwrap()
            .snapshot;
        let mut session = Session::new("h".into(), "Photo booth".into(), true);
        session.snapshot = snapshot;
        session.page = Page::Services;
        session.service_group = "Music".into();
        session.confirm_service = Some(("Raspotify".into(), ServiceAction::Stop));
        render(&mut session);
        assert!(session.take_outbox().is_empty());
        // The simulated device applies actions to the services it lists.
        let update = backend
            .handle(SetupRequest::ServiceAction {
                name: "Raspotify".into(),
                action: ServiceAction::Stop,
            })
            .unwrap();
        let state = |name: &str| {
            update
                .snapshot
                .services
                .iter()
                .find(|service| service.name == name)
                .unwrap()
                .state
        };
        assert!(update.error.is_none());
        assert_eq!(state("Raspotify"), ServiceState::Inactive);
        assert_eq!(state("Mopidy"), ServiceState::Inactive);
        let refused = backend
            .handle(SetupRequest::ServiceAction {
                name: "Photo booth app".into(),
                action: ServiceAction::Stop,
            })
            .unwrap();
        assert!(refused.error.is_some());
    }

    #[test]
    fn every_function_has_a_label_and_a_mark_that_names_the_state() {
        let snapshot = zorvinkel_application::setup::SetupSnapshot::default();
        let labels = snapshot
            .functions()
            .map(|(function, _)| function_label(function));
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len());
        assert_ne!(
            availability_style(Availability::Available).0,
            availability_style(Availability::NotOffered).0
        );
    }

    #[test]
    fn the_wifi_page_covers_both_modes_and_one_restart_choice() {
        use zorvinkel_core::network::RestartPolicy;
        let mut session = Session::new("h".into(), "Photo booth".into(), false);
        session.snapshot.wifi_available = true;
        session.snapshot.access_point_available = true;
        session.snapshot.wifi_persistence = Some(false);
        session.snapshot.restart_policy = Some(RestartPolicy::LastAccepted);
        session.snapshot.status = Some(zorvinkel_core::DeviceStatus {
            network: zorvinkel_core::NetworkStatus::AccessPointActive { ssid: "Box".into() },
            ip_reachability: IpReachability::Unverified,
        });
        render(&mut session);
        // Without a choice the page follows the device: its own Wi-Fi is on.
        assert_eq!(
            session.wifi_mode,
            Some(super::super::session::WifiMode::OwnNetwork)
        );
        assert!(session.take_outbox().is_empty());
        session.wifi_mode = Some(super::super::session::WifiMode::Join);
        render(&mut session);
        // Only one of the two offered: no switch, that topic alone.
        session.wifi_mode = None;
        session.snapshot.wifi_available = false;
        render(&mut session);
        assert_eq!(session.wifi_mode, None);
        session.snapshot.access_point_available = false;
        session.snapshot.wifi_available = true;
        render(&mut session);
        assert_eq!(session.page, Page::Wifi);
    }
}
