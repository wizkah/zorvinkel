//! Conversion between the application's service entries and their wire form.
use zorvinkel_application::services as app;
use zorvinkel_protocol::services as wire;

pub fn to_wire(services: Vec<app::ServiceEntry>) -> Vec<wire::ServiceEntry> {
    services
        .into_iter()
        .map(|entry| wire::ServiceEntry {
            group: entry.group,
            name: entry.name,
            version: entry.version,
            state: match entry.state {
                app::ServiceState::Unknown => wire::ServiceState::Unknown,
                app::ServiceState::Active => wire::ServiceState::Active,
                app::ServiceState::Activating => wire::ServiceState::Activating,
                app::ServiceState::Inactive => wire::ServiceState::Inactive,
                app::ServiceState::Failed => wire::ServiceState::Failed,
            },
            actions: entry.actions.into_iter().map(action_to_wire).collect(),
            web: entry.web.map(|web| wire::WebLink {
                label: web.label,
                scheme: match web.scheme {
                    app::WebScheme::Http => wire::WebScheme::Http,
                    app::WebScheme::Https => wire::WebScheme::Https,
                },
                port: web.port,
                path: web.path,
            }),
        })
        .collect()
}
pub fn from_wire(services: Vec<wire::ServiceEntry>) -> Vec<app::ServiceEntry> {
    services
        .into_iter()
        .map(|entry| app::ServiceEntry {
            group: entry.group,
            name: entry.name,
            version: entry.version,
            state: match entry.state {
                wire::ServiceState::Unknown => app::ServiceState::Unknown,
                wire::ServiceState::Active => app::ServiceState::Active,
                wire::ServiceState::Activating => app::ServiceState::Activating,
                wire::ServiceState::Inactive => app::ServiceState::Inactive,
                wire::ServiceState::Failed => app::ServiceState::Failed,
            },
            actions: entry.actions.into_iter().map(action_from_wire).collect(),
            web: entry.web.map(|web| app::WebLink {
                label: web.label,
                scheme: match web.scheme {
                    wire::WebScheme::Http => app::WebScheme::Http,
                    wire::WebScheme::Https => app::WebScheme::Https,
                },
                port: web.port,
                path: web.path,
            }),
        })
        .collect()
}

pub fn action_to_wire(action: app::ServiceAction) -> wire::ServiceAction {
    match action {
        app::ServiceAction::Start => wire::ServiceAction::Start,
        app::ServiceAction::Stop => wire::ServiceAction::Stop,
        app::ServiceAction::Restart => wire::ServiceAction::Restart,
    }
}
pub fn action_from_wire(action: wire::ServiceAction) -> app::ServiceAction {
    match action {
        wire::ServiceAction::Start => app::ServiceAction::Start,
        wire::ServiceAction::Stop => app::ServiceAction::Stop,
        wire::ServiceAction::Restart => app::ServiceAction::Restart,
    }
}
