//! Content of a device tab: connection control, messages and the setup pages.
use super::host_keys::HostKeyStore;
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
        ConnectionState::Connected => theme::SUCCESS,
        ConnectionState::Working(_) => theme::WARNING,
        ConnectionState::Failed => theme::DANGER,
        ConnectionState::Disconnected | ConnectionState::OutOfRange => theme::TEXT_MUTED,
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
        ui.add(egui::Spinner::new().size(12.0).color(theme::WARNING));
    } else {
        theme::status_dot(ui, state_color(state));
    }
    ui.label(RichText::new(lang.t(state_text(state))).color(state_color(state)));
}

impl Session {
    pub fn show_tab(&mut self, ui: &mut egui::Ui, lang: Language, host_keys: &mut HostKeyStore) {
        if let Some(terminal) = &mut self.terminal {
            terminal.poll(&self.handle, host_keys);
        }
        self.header(ui, lang);
        if let Some(text) = self.error.and_then(error_text) {
            ui.add_space(theme::space::S);
            theme::message(
                ui,
                lang.t(text),
                theme::DANGER,
                theme::DANGER_SUBTLE,
            );
        } else if let Some(notice) = self.notice {
            ui.add_space(theme::space::S);
            theme::message(
                ui,
                lang.t(notice.text()),
                theme::ACCENT,
                theme::ACCENT_SUBTLE,
            );
        }
        ui.add_space(theme::space::L);
        if self.needs_login() {
            self.login_form(ui, lang);
        }
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
        self.show_pages(ui, lang, host_keys);
    }
    /// Registers this computer with an administrator login of the device (ADR-037).
    fn login_form(&mut self, ui: &mut egui::Ui, lang: Language) {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(lang.t(Text::LoginIntro));
            ui.add_space(theme::space::S);
            let width = ui.available_width().min(theme::size::FIELD_WIDTH);
            ui.label(lang.t(Text::LoginUser));
            ui.add(
                theme::text_field(egui::TextEdit::singleline(&mut self.login_user))
                    .desired_width(width),
            );
            ui.label(lang.t(Text::LoginPassword));
            let password = ui.add(
                theme::text_field(
                    egui::TextEdit::singleline(&mut *self.login_password).password(true),
                )
                .desired_width(width),
            );
            ui.add_space(theme::space::S);
            let ready = !self.login_user.trim().is_empty()
                && !self.login_password.is_empty()
                && self.active_action.is_none();
            let enter =
                password.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let button = ui.add_enabled(ready, theme::primary_button(lang.t(Text::LoginButton)));
            if ready && (button.clicked() || enter) {
                self.connect_with_login();
            }
        });
        ui.add_space(theme::space::L);
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
                        .color(theme::TEXT_MUTED),
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
    fn show_pages(&mut self, ui: &mut egui::Ui, lang: Language, host_keys: &mut HostKeyStore) {
        let wifi_offered =
            self.simulation || self.snapshot.wifi_available || self.snapshot.access_point_available;
        let system_offered = self.snapshot.system_info_available;
        let groups = self.service_groups();
        if (!wifi_offered && self.page == Page::Wifi)
            || (!system_offered && self.page == Page::System)
            || (self.page == Page::Services && !groups.contains(&self.service_group))
            || (!self.snapshot.power_available && self.page == Page::Power)
            || (!self.terminal_offered() && self.page == Page::Terminal)
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
                ui.spacing_mut().item_spacing.x = theme::space::XS;
                fn page_tab(ui: &mut egui::Ui, current: &mut Page, page: Page, label: &str) {
                    if theme::page_tab(ui, *current == page, label).clicked() {
                        *current = page;
                    }
                }
                if wifi_offered {
                    page_tab(ui, &mut self.page, Page::Wifi, lang.t(Text::PageWifi));
                }
                if system_offered {
                    page_tab(ui, &mut self.page, Page::System, lang.t(Text::PageSystem));
                }
                for group in &groups {
                    let selected = self.page == Page::Services && &self.service_group == group;
                    if theme::page_tab(ui, selected, group).clicked() {
                        self.page = Page::Services;
                        self.service_group = group.clone();
                    }
                }
                if self.snapshot.power_available {
                    page_tab(ui, &mut self.page, Page::Power, lang.t(Text::PageDevice));
                }
                if self.terminal_offered() {
                    page_tab(
                        ui,
                        &mut self.page,
                        Page::Terminal,
                        lang.t(Text::PageTerminal),
                    );
                }
            });
        });
        ui.add_space(theme::space::S);
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            match self.page {
                Page::Wifi => self.wifi_page(ui, lang),
                Page::System => self.system(ui, lang),
                Page::Services => self.services(ui, lang),
                Page::Power => self.power(ui, lang),
                Page::Terminal => self.terminal_page(ui, lang, host_keys),
            }
        });
    }
    /// Connection details shown beside the pages.
    pub fn show_status(&mut self, ui: &mut egui::Ui, lang: Language) {
        status_caption(ui, lang.t(Text::ConnectionStatus));
        theme::label_value(
            ui,
            lang.t(Text::ConnectionPath),
            RichText::new(self.path_text().map_or("–", |path| lang.t(path))),
        );
        theme::label_value(
            ui,
            lang.t(Text::LocalAccess),
            RichText::new(lang.t(if self.is_connected() {
                Text::StateConnected
            } else {
                Text::NotConnected
            })),
        );
        if let Some(status) = &self.snapshot.status {
            theme::label_value(
                ui,
                lang.t(Text::NetworkOnDevice),
                RichText::new(network_label(&status.network, lang)),
            );
            let (text, color) = reachability_label(status.ip_reachability);
            theme::label_value(
                ui,
                lang.t(Text::ReachableFromComputer),
                RichText::new(lang.t(text)).color(color),
            );
        }
        if self.is_connected() {
            ui.add_space(theme::space::M);
            status_caption(ui, lang.t(Text::FunctionsOverConnection));
            for (function, availability) in self.snapshot.functions() {
                let (offered, color) = availability_style(availability);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::space::S;
                    // Lines the tick up with the first line of a label that wraps.
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        theme::tick(ui, offered);
                    });
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(lang.t(function_label(function))).color(color),
                            )
                            .wrap(),
                        );
                        if !offered {
                            let hint = if function == Function::Terminal {
                                Text::NoAddressYet
                            } else {
                                Text::NotOfferedByDevice
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(lang.t(hint)).small().color(theme::TEXT_MUTED),
                                )
                                .wrap(),
                            );
                        }
                    });
                });
            }
        }
    }
}

/// Heading of a group in the status column.
fn status_caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .small()
            .strong()
            .color(theme::TEXT_MUTED),
    );
    ui.add_space(theme::space::XS);
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
        Function::Terminal => Text::FunctionTerminal,
    }
}
/// Whether the function gets a tick, and the colour of its name.
fn availability_style(availability: Availability) -> (bool, egui::Color32) {
    match availability {
        Availability::Available => (true, theme::TEXT),
        Availability::NotOffered => (false, theme::TEXT_MUTED),
    }
}

/// The message for an error; none for the first refusal, where the login form speaks for itself.
fn error_text(error: SetupError) -> Option<Text> {
    Some(match error {
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
        SetupError::LoginRequired => return None,
        SetupError::LoginRejected => Text::ErrorLoginRejected,
        SetupError::NotAdministrator => Text::ErrorNotAdministrator,
        SetupError::TooManyLogins => Text::ErrorTooManyLogins,
    })
}
fn reachability_label(reachability: IpReachability) -> (Text, egui::Color32) {
    match reachability {
        IpReachability::Unverified => (Text::ReachabilityUnverified, theme::TEXT_MUTED),
        IpReachability::Authenticated => (Text::ReachabilityAuthenticated, theme::SUCCESS),
        IpReachability::Unreachable => (Text::ReachabilityUnreachable, theme::WARNING),
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
        let mut host_keys = HostKeyStore::none();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default()
                .show(context, |ui| session.show_tab(ui, lang, &mut host_keys));
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
    fn a_device_that_does_not_know_this_computer_asks_for_an_administrator_login() {
        use zorvinkel_application::setup::SetupRequest;
        let mut session = Session::new("h".into(), "Pi".into(), false);
        assert!(!session.needs_login());
        for error in [
            SetupError::LoginRequired,
            SetupError::LoginRejected,
            SetupError::NotAdministrator,
            SetupError::TooManyLogins,
        ] {
            session.error = Some(error);
            assert!(session.needs_login(), "{error:?}");
            for lang in Language::ALL {
                render_in(&mut session, lang);
            }
        }
        session.error = Some(SetupError::Unavailable);
        assert!(!session.needs_login(), "other failures show no login form");

        session.error = Some(SetupError::LoginRequired);
        session.login_user = " pi ".into();
        *session.login_password = "secret".into();
        session.connect_with_login();
        assert!(
            session.login_password.is_empty(),
            "the password leaves the form"
        );
        assert!(session.error.is_none());
        let sent = session.take_outbox();
        let [
            SetupRequest::ConnectWithLogin {
                handle,
                user,
                password,
            },
        ] = &sent[..]
        else {
            panic!("expected one login request");
        };
        assert_eq!(
            (handle.as_str(), user.as_str(), password.as_str()),
            ("h", "pi", "secret")
        );
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
