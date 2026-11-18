//! Pages of the configured services: one tab per group, one block per service (ADR-021).
use super::{
    i18n::{Language, Text},
    pages::draw_qr,
    session::Session,
    theme,
};
use eframe::egui::{self, RichText};
use std::net::IpAddr;
use zorvinkel_application::{
    services::{ServiceAction, ServiceEntry, ServiceState},
    setup::SetupRequest,
    system::NetworkAddress,
};
use zorvinkel_core::IpReachability;

impl Session {
    /// Groups of the reported services, alphabetically.
    pub fn service_groups(&self) -> Vec<String> {
        let mut groups = self
            .snapshot
            .services
            .iter()
            .map(|service| service.group.clone())
            .collect::<Vec<_>>();
        groups.sort();
        groups.dedup();
        groups
    }

    pub fn services(&mut self, ui: &mut egui::Ui, lang: Language) {
        ui.heading(&self.service_group);
        let reachability = self
            .snapshot
            .status
            .as_ref()
            .map(|status| status.ip_reachability)
            .unwrap_or(IpReachability::Unverified);
        let addresses = self
            .snapshot
            .system_info
            .as_ref()
            .map(|info| info.addresses.clone())
            .unwrap_or_default();
        let services = self
            .snapshot
            .services
            .iter()
            .filter(|service| service.group == self.service_group)
            .cloned()
            .collect::<Vec<_>>();
        for (index, service) in services.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            show_service(ui, service, &addresses, reachability, lang);
            if self.snapshot.service_actions_available {
                self.service_actions(ui, service, lang);
            }
        }
    }

    fn service_actions(&mut self, ui: &mut egui::Ui, service: &ServiceEntry, lang: Language) {
        if service.actions.is_empty() {
            return;
        }
        ui.add_space(6.0);
        if let Some((name, action)) = self.confirm_service.clone()
            && name == service.name
        {
            ui.label(confirmation_question(&service.name, action, lang));
            ui.horizontal_wrapped(|ui| {
                if self.action_button(ui, theme::primary_button(lang.t(action_label(action)))) {
                    self.confirm_service = None;
                    self.submit(SetupRequest::ServiceAction { name, action });
                }
                if self.action_button(ui, egui::Button::new(lang.t(Text::Cancel))) {
                    self.confirm_service = None;
                }
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for action in &service.actions {
                if self.action_button(ui, egui::Button::new(lang.t(action_label(*action)))) {
                    if *action == ServiceAction::Start {
                        self.confirm_service = None;
                        self.submit(SetupRequest::ServiceAction {
                            name: service.name.clone(),
                            action: *action,
                        });
                    } else {
                        self.confirm_service = Some((service.name.clone(), *action));
                    }
                }
            }
        });
    }
}

fn action_label(action: ServiceAction) -> Text {
    match action {
        ServiceAction::Start => Text::ServiceStart,
        ServiceAction::Stop => Text::ServiceStop,
        ServiceAction::Restart => Text::Restart,
    }
}
fn confirmation_question(name: &str, action: ServiceAction, lang: Language) -> String {
    match action {
        ServiceAction::Stop => lang.format(Text::ConfirmStopService, &[&name]),
        ServiceAction::Restart => lang.format(Text::ConfirmRestartService, &[&name]),
        ServiceAction::Start => lang.format(Text::ConfirmStartService, &[&name]),
    }
}

fn show_service(
    ui: &mut egui::Ui,
    service: &ServiceEntry,
    addresses: &[NetworkAddress],
    reachability: IpReachability,
    lang: Language,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&service.name).strong().size(16.0));
        let (text, color) = state_label(service.state);
        ui.label(RichText::new(lang.t(text)).strong().color(color));
    });
    if let Some(version) = &service.version {
        ui.label(RichText::new(lang.format(Text::ServiceVersion, &[version])).color(theme::MUTED));
    }
    let Some(web) = &service.web else {
        return;
    };
    ui.add_space(6.0);
    let Some(host) = web_host(addresses) else {
        ui.small(lang.t(Text::ServiceNoAddress));
        return;
    };
    let url = web.url(host);
    ui.horizontal(|ui| {
        draw_qr(ui, &url, 140.0, lang);
        ui.vertical(|ui| {
            let caption = web
                .label
                .as_deref()
                .unwrap_or_else(|| lang.t(Text::ServiceWebInterface));
            if ui
                .add(theme::primary_button(
                    &lang.format(Text::ServiceOpen, &[&caption]),
                ))
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&url));
            }
            ui.monospace(&url);
            ui.small(lang.t(Text::ScanToOpen));
            if reachability != IpReachability::Authenticated {
                ui.small(
                    RichText::new(lang.t(Text::ServiceNotConfirmedReachable)).color(theme::AMBER),
                );
            }
        });
    });
}

/// IPv4 is preferred: it works in every home network and is short to read and scan.
fn web_host(addresses: &[NetworkAddress]) -> Option<IpAddr> {
    addresses
        .iter()
        .map(|entry| entry.address)
        .find(IpAddr::is_ipv4)
        .or_else(|| addresses.first().map(|entry| entry.address))
}

fn state_label(state: ServiceState) -> (Text, egui::Color32) {
    match state {
        ServiceState::Active => (Text::ServiceRunning, theme::GREEN),
        ServiceState::Activating => (Text::ServiceStarting, theme::AMBER),
        ServiceState::Inactive => (Text::ServiceStopped, theme::MUTED),
        ServiceState::Failed => (Text::StateFailed, theme::ERROR),
        ServiceState::Unknown => (Text::ServiceUnknown, theme::MUTED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn address(address: IpAddr) -> NetworkAddress {
        NetworkAddress {
            interface: "wlan0".into(),
            address,
        }
    }

    #[test]
    fn web_host_prefers_ipv4_and_falls_back_to_ipv6() {
        let v4 = IpAddr::from(Ipv4Addr::new(192, 168, 1, 42));
        let v6 = IpAddr::from(Ipv6Addr::LOCALHOST);
        assert_eq!(web_host(&[address(v6), address(v4)]), Some(v4));
        assert_eq!(web_host(&[address(v6)]), Some(v6));
        assert_eq!(web_host(&[]), None);
    }

    #[test]
    fn groups_are_unique_and_sorted() {
        let mut session = Session::new("simulation:device-1".into(), "Fotobox".into(), true);
        let entry = |group: &str| ServiceEntry {
            group: group.into(),
            name: "x".into(),
            version: None,
            state: ServiceState::Active,
            actions: Vec::new(),
            web: None,
        };
        session.snapshot.services = vec![entry("Musik"), entry("Foto"), entry("Musik")];
        assert_eq!(session.service_groups(), ["Foto", "Musik"]);
    }
}
