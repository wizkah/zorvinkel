mod device_tab;
mod devices;
mod hub;
mod i18n;
mod pages;
mod progress;
mod services_page;
mod session;
mod system_page;
mod theme;
#[cfg(target_os = "macos")]
mod window_buttons;
mod worker;
use eframe::egui::{self, RichText};
use hub::{Hub, Owner};
use i18n::{Language, LanguageStore, Text};
use session::{Action, ConnectionState, Notice, Session};
use zorvinkel_adapters::simulation::setup::SimulatedSetup;
use zorvinkel_application::setup::*;

/// Seconds between automatic status reads of a connected device.
/// All tab headers share one height, whether or not they show a state and a close button.
const TAB_HEIGHT: f32 = 32.0;
/// Height of the tab bar. On macOS the native window controls sit inside it, so it is
/// chosen to put their vertical centre on the centre line of the tabs.
const TAB_BAR_HEIGHT: f32 = 44.0;
const REFRESH_INTERVAL: f64 = 5.0;
const SIMULATION_DELAY: std::time::Duration = std::time::Duration::from_millis(450);

#[derive(Clone, PartialEq, Eq, Debug)]
enum View {
    Devices,
    Device(String),
}
struct App {
    brand: theme::Brand,
    progress: progress::ProgressIndicator,
    simulation: bool,
    language: Language,
    language_store: LanguageStore,
    hub: Hub,
    sessions: Vec<Session>,
    view: View,
    searching: bool,
    search_error: Option<SetupError>,
}

pub fn export_app_icon(path: &std::path::Path) -> Result<(), image::ImageError> {
    let icon = theme::app_icon()?;
    image::save_buffer(
        path,
        &icon.rgba,
        icon.width,
        icon.height,
        image::ColorType::Rgba8,
    )
}

/// Simulated devices that can be connected side by side.
pub fn run(device_count: usize) -> eframe::Result {
    run_with(true, LanguageStore::none(), move |wake| {
        Hub::parallel_connections(
            SimulatedSetup::with_device_count(device_count),
            Box::new(move || {
                Box::new(SimulatedSetup::with_device_count(device_count).discovered())
            }),
            wake,
            SIMULATION_DELAY,
        )
    })
}

/// A real backend holds one connection at a time.
#[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
pub fn run_backend(
    backend: impl SetupBackend,
    settings_directory: &std::path::Path,
) -> eframe::Result {
    run_with(
        false,
        LanguageStore::in_directory(settings_directory),
        move |wake| Hub::single_connection(backend, wake, std::time::Duration::ZERO),
    )
}

fn run_with(
    simulation: bool,
    store: LanguageStore,
    hub: impl FnOnce(Box<dyn Fn() + Send + Sync>) -> Hub + 'static,
) -> eframe::Result {
    let icon = theme::app_icon().map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_fullsize_content_view(cfg!(target_os = "macos"))
            .with_titlebar_shown(!cfg!(target_os = "macos"))
            .with_title_shown(!cfg!(target_os = "macos"))
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        if simulation {
            "Zorvinkel – Simulation"
        } else {
            "Zorvinkel"
        },
        options,
        Box::new(move |creation| {
            theme::apply(&creation.egui_ctx);
            creation
                .egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Light));
            let context = creation.egui_ctx.clone();
            let mut app = App {
                brand: theme::Brand::load(&creation.egui_ctx)?,
                progress: progress::ProgressIndicator::new()?,
                simulation,
                language: store.load().unwrap_or_else(Language::detect),
                language_store: store,
                hub: hub(Box::new(move || context.request_repaint())),
                sessions: Vec::new(),
                view: View::Devices,
                searching: false,
                search_error: None,
            };
            app.search();
            Ok(Box::new(app))
        }),
    )
}

impl App {
    fn search(&mut self) {
        if self.searching {
            return;
        }
        self.searching = true;
        self.search_error = None;
        self.hub
            .submit(Owner::Discovery, SetupRequest::Discover, false);
    }
    fn session_mut(&mut self, handle: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.handle == handle)
    }
    fn open_tab(&mut self, index: usize) {
        self.sessions[index].tab_open = true;
        self.view = View::Device(self.sessions[index].handle.clone());
    }
    fn close_tab(&mut self, handle: &str) {
        if let Some(session) = self.session_mut(handle) {
            session.tab_open = false;
            if session.is_connected() && session.active_action.is_none() {
                session.disconnect();
            }
        }
        if self.view == View::Device(handle.into()) {
            self.view = View::Devices;
        }
    }
    fn apply_tile_events(&mut self, events: Vec<(usize, devices::TileEvent)>) {
        for (index, event) in events {
            match event {
                devices::TileEvent::Open => self.open_tab(index),
                devices::TileEvent::Connect => {
                    self.open_tab(index);
                    self.sessions[index].connect();
                }
                devices::TileEvent::Disconnect => self.sessions[index].disconnect(),
            }
        }
    }
    /// Sends queued session requests and starts due background refreshes.
    fn dispatch(&mut self, now: f64) {
        for session in &mut self.sessions {
            let owner = Owner::Device(session.handle.clone());
            for request in session.take_outbox() {
                self.hub.submit(owner.clone(), request, false);
            }
            let due = now - session.last_refresh > REFRESH_INTERVAL;
            if !self.simulation && session.needs_renewal() && self.hub.is_idle(&owner) {
                // A new session over the same path; it runs unseen like a refresh.
                session.last_refresh = now;
                session.refresh_pending = true;
                let connect = SetupRequest::Connect(session.handle.clone());
                self.hub.submit(owner, connect, true);
            } else if !self.simulation
                && due
                && session.is_connected()
                && session.error.is_none()
                && session.active_action.is_none()
                && !session.refresh_pending
                && self.hub.is_idle(&owner)
            {
                session.last_refresh = now;
                session.refresh_pending = true;
                self.hub.submit(owner, SetupRequest::Refresh, true);
            }
        }
    }
    fn receive(&mut self) {
        for completed in self.hub.poll() {
            match completed.owner {
                Owner::Discovery => self.finish_search(completed.reply),
                Owner::Device(handle) => {
                    self.finish_device_request(&handle, completed.background, completed.reply)
                }
            }
        }
    }
    fn finish_search(&mut self, reply: worker::Reply) {
        self.searching = false;
        let single = self.hub.is_single_connection();
        match reply.result {
            Ok(update) => {
                self.search_error = update.error;
                self.merge_devices(update.snapshot.devices);
            }
            Err(error) => self.search_error = Some(error),
        }
        if single {
            // A search ends the only connection of a single-connection backend.
            for session in &mut self.sessions {
                if session.is_connected() {
                    session.mark_disconnected(Notice::EndedBySearch);
                }
            }
        }
    }
    /// Keeps a session per device; devices that vanished stay while their tab is open or
    /// they are still connected.
    fn merge_devices(&mut self, found: Vec<zorvinkel_core::DiscoveredDevice>) {
        for session in &mut self.sessions {
            session.in_range = found.iter().any(|d| d.handle == session.handle);
        }
        for device in found {
            if let Some(session) = self.session_mut(&device.handle) {
                session.name = device.display_name;
            } else {
                let simulation = self.simulation;
                self.sessions
                    .push(Session::new(device.handle, device.display_name, simulation));
            }
        }
        self.sessions
            .retain(|s| s.in_range || s.tab_open || s.is_connected());
        if let View::Device(handle) = &self.view
            && !self.sessions.iter().any(|s| &s.handle == handle)
        {
            self.view = View::Devices;
        }
    }
    fn finish_device_request(&mut self, handle: &str, background: bool, reply: worker::Reply) {
        let single = self.hub.is_single_connection();
        let Some(session) = self.session_mut(handle) else {
            return;
        };
        let finished = session.receive(reply.result, reply.cancelled, background);
        let took_over = single && finished == Some(Action::Connect) && session.is_connected();
        if took_over {
            for other in self.sessions.iter_mut().filter(|s| s.handle != handle) {
                if other.is_connected() {
                    other.mark_disconnected(Notice::TakenOver);
                }
            }
        }
    }
    fn show_tab_bar(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        let mut close = None;
        let mut show = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.brand.header_right(ui);
            if language_selector(ui, &mut self.language) {
                self.language_store.save(self.language);
            }
            // One explicitly placed row, vertically centred in the bar, shared by all tabs.
            let space = ui.available_rect_before_wrap();
            let half = (TAB_HEIGHT + 2.0) / 2.0;
            let row = egui::Rect::from_min_max(
                egui::pos2(space.min.x, space.center().y - half),
                egui::pos2(space.max.x, space.center().y + half),
            );
            let builder = egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Min));
            ui.scope_builder(builder, |ui| {
                // Start the tabs right of the native macOS window controls.
                #[cfg(target_os = "macos")]
                ui.add_space(80.0);
                if tab(
                    ui,
                    lang.t(Text::Devices),
                    None,
                    self.view == View::Devices,
                    lang,
                )
                .selected
                {
                    show = Some(View::Devices);
                }
                egui::ScrollArea::horizontal()
                    .id_salt("tabs")
                    // Shrink to the tab height so the row centres it like the "Devices" tab;
                    // a full-height scroll area would start at the top of the bar instead.
                    .auto_shrink([true, true])
                    .max_height(TAB_HEIGHT + 2.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for session in self.sessions.iter().filter(|s| s.tab_open) {
                                let selected = self.view == View::Device(session.handle.clone());
                                let response =
                                    tab(ui, &session.name, Some(session.state()), selected, lang);
                                if response.selected {
                                    show = Some(View::Device(session.handle.clone()));
                                }
                                if response.closed {
                                    close = Some(session.handle.clone());
                                }
                            }
                        });
                    });
            });
        });
        if let Some(handle) = close {
            self.close_tab(&handle);
        } else if let Some(view) = show {
            self.view = view;
        }
    }
    fn show_devices(&mut self, ui: &mut egui::Ui) {
        let lang = self.language;
        ui.horizontal(|ui| {
            ui.heading(lang.t(Text::Devices));
            if self.searching {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(lang.t(Text::Searching)).color(theme::MUTED));
            } else {
                ui.label(
                    RichText::new(lang.format(Text::FoundCount, &[&self.sessions.len()]))
                        .color(theme::MUTED),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.searching,
                        egui::Button::new(lang.t(Text::SearchAgain)),
                    )
                    .clicked()
                {
                    self.search();
                }
            });
        });
        if let Some(error) = self.search_error {
            theme::message(
                ui,
                lang.t(search_error_text(error)),
                theme::ERROR,
                theme::ERROR_PALE,
            );
        }
        ui.add_space(4.0);
        if self.sessions.is_empty() && !self.searching && self.search_error.is_none() {
            ui.label(RichText::new(lang.t(Text::NoDevicesFound)).color(theme::MUTED));
            return;
        }
        let events = devices::grid(ui, &self.sessions, lang);
        self.apply_tile_events(events);
    }
}

struct TabResponse {
    selected: bool,
    closed: bool,
}
/// One tab header; the "Devices" tab has no state and cannot be closed.
fn tab(
    ui: &mut egui::Ui,
    title: &str,
    state: Option<ConnectionState>,
    selected: bool,
    lang: Language,
) -> TabResponse {
    let mut response = TabResponse {
        selected: false,
        closed: false,
    };
    egui::Frame::new()
        .fill(if selected {
            theme::BLUE_PALE
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if selected {
                theme::BORDER
            } else {
                egui::Color32::TRANSPARENT
            },
        ))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().interact_size.y = TAB_HEIGHT;
            ui.spacing_mut().button_padding.y = 0.0;
            ui.horizontal(|ui| {
                ui.set_height(TAB_HEIGHT);
                if let Some(state) = state {
                    if matches!(state, ConnectionState::Working(_)) {
                        ui.add(egui::Spinner::new().size(10.0).color(theme::AMBER));
                    } else {
                        theme::status_dot(ui, device_tab::state_color(state));
                    }
                }
                response.selected = ui
                    .add(egui::Button::selectable(selected, title).frame(false))
                    .clicked();
                if state.is_some() {
                    response.closed = ui
                        .add(egui::Button::new(RichText::new("×").color(theme::MUTED)).frame(false))
                        .on_hover_text(lang.t(Text::CloseTab))
                        .clicked();
                }
            });
        });
    response
}
/// Drop-down in the tab bar that changes the language of every text at once; true if it changed.
fn language_selector(ui: &mut egui::Ui, language: &mut Language) -> bool {
    let before = *language;
    egui::ComboBox::from_id_salt("language")
        .selected_text(language.native_name())
        .width(96.0)
        .show_ui(ui, |ui| {
            for option in Language::ALL {
                ui.selectable_value(language, option, option.native_name());
            }
        })
        .response
        .on_hover_text(language.t(Text::Language));
    *language != before
}
fn search_error_text(error: SetupError) -> Text {
    match error {
        SetupError::AccessDenied | SetupError::IdentityRejected => Text::SearchNotPermitted,
        _ => Text::SearchFailed,
    }
}

impl App {
    /// The title of what the visible tab is waiting for, if anything.
    fn progress_title(&self) -> Option<Text> {
        match &self.view {
            View::Devices => self.searching.then_some(Text::ProgressSearching),
            View::Device(handle) => self
                .sessions
                .iter()
                .find(|s| &s.handle == handle)?
                .active_action
                .map(Action::progress_title),
        }
    }
    fn show_progress(&mut self, context: &egui::Context) {
        let Some(title) = self.progress_title() else {
            return;
        };
        egui::Area::new(egui::Id::new("action_progress"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .constrain_to(context.content_rect().shrink(8.0))
            .movable(false)
            .interactable(false)
            .show(context, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.vertical_centered(|ui| {
                            self.progress.show(ui, self.language);
                            ui.add_space(12.0);
                            ui.label(self.language.t(title));
                        });
                    });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, context: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        window_buttons::center_in_bar(frame, f64::from(TAB_BAR_HEIGHT));
        #[cfg(not(target_os = "macos"))]
        let _ = frame;
        self.receive();
        let now = context.input(|input| input.time);
        self.dispatch(now);
        egui::TopBottomPanel::top("tabs")
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .exact_height(TAB_BAR_HEIGHT)
            .show(context, |ui| {
                self.show_tab_bar(ui);
            });
        let lang = self.language;
        if let View::Device(handle) = self.view.clone() {
            egui::SidePanel::right("status")
                .exact_width(230.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(16))
                .show(context, |ui| {
                    if let Some(session) = self.session_mut(&handle) {
                        session.show_status(ui, lang);
                    }
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(20))
            .show(context, |ui| match self.view.clone() {
                View::Devices => self.show_devices(ui),
                View::Device(handle) => {
                    if let Some(session) = self.session_mut(&handle) {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| session.show_tab(ui, lang));
                    }
                }
            });
        self.dispatch(now);
        self.show_progress(context);
        if self.searching
            || self
                .sessions
                .iter()
                .any(|s| s.active_action.is_some() || s.is_connected())
        {
            context.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(device_count: usize) -> App {
        let context = egui::Context::default();
        App {
            brand: theme::Brand::load(&context).unwrap(),
            progress: progress::ProgressIndicator::new().unwrap(),
            simulation: true,
            language: Language::English,
            language_store: LanguageStore::none(),
            hub: Hub::parallel_connections(
                SimulatedSetup::with_device_count(device_count),
                Box::new(move || {
                    Box::new(SimulatedSetup::with_device_count(device_count).discovered())
                }),
                || {},
                std::time::Duration::ZERO,
            ),
            sessions: Vec::new(),
            view: View::Devices,
            searching: false,
            search_error: None,
        }
    }
    fn settle(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.receive();
            app.dispatch(0.0);
            let busy = app.searching || app.sessions.iter().any(|s| s.active_action.is_some());
            if !busy {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "workers did not finish"
            );
            std::thread::yield_now();
        }
    }
    fn frame(app: &mut App) {
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            eframe::App::update(app, context, &mut eframe::Frame::_new_kittest());
        });
    }

    #[test]
    fn search_creates_one_session_per_device() {
        let mut app = app(30);
        app.search();
        settle(&mut app);
        assert_eq!(app.sessions.len(), 30);
        assert!(app.sessions.iter().all(|s| s.in_range && !s.tab_open));
        // A repeated search keeps the same sessions.
        app.search();
        settle(&mut app);
        assert_eq!(app.sessions.len(), 30);
    }
    #[test]
    fn connecting_from_the_grid_opens_a_tab_and_connects_in_parallel() {
        let mut app = app(30);
        app.search();
        settle(&mut app);
        app.apply_tile_events(
            (0..30)
                .map(|index| (index, devices::TileEvent::Connect))
                .collect(),
        );
        settle(&mut app);
        assert!(app.sessions.iter().all(|s| s.tab_open && s.is_connected()));
        assert_eq!(app.view, View::Device(app.sessions[29].handle.clone()));
    }
    #[test]
    fn opening_a_tile_only_opens_the_tab() {
        let mut app = app(3);
        app.search();
        settle(&mut app);
        app.apply_tile_events(vec![(1, devices::TileEvent::Open)]);
        settle(&mut app);
        assert!(app.sessions[1].tab_open);
        assert_eq!(app.sessions[1].state(), ConnectionState::Disconnected);
    }
    #[test]
    fn closing_a_connected_tab_disconnects_the_device() {
        let mut app = app(2);
        app.search();
        settle(&mut app);
        app.apply_tile_events(vec![(0, devices::TileEvent::Connect)]);
        settle(&mut app);
        let handle = app.sessions[0].handle.clone();
        app.close_tab(&handle);
        settle(&mut app);
        assert!(!app.sessions[0].tab_open && !app.sessions[0].is_connected());
        assert_eq!(app.view, View::Devices);
    }
    #[test]
    fn vanished_devices_are_dropped_unless_connected_or_open() {
        let mut app = app(3);
        app.search();
        settle(&mut app);
        app.apply_tile_events(vec![(0, devices::TileEvent::Open)]);
        app.merge_devices(Vec::new());
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].state(), ConnectionState::OutOfRange);
    }
    #[test]
    fn a_single_connection_backend_disconnects_the_previous_device() {
        let mut app = app(2);
        app.hub = Hub::single_connection(
            SimulatedSetup::with_device_count(2),
            || {},
            std::time::Duration::ZERO,
        );
        app.search();
        settle(&mut app);
        app.apply_tile_events(vec![
            (0, devices::TileEvent::Connect),
            (1, devices::TileEvent::Connect),
        ]);
        settle(&mut app);
        assert!(!app.sessions[0].is_connected());
        assert!(app.sessions[1].is_connected());
    }
    #[test]
    fn progress_overlay_follows_the_visible_tab_and_is_centered() {
        let mut app = app(2);
        assert_eq!(app.progress_title(), None);
        app.search();
        assert_eq!(app.progress_title(), Some(Text::ProgressSearching));
        settle(&mut app);
        app.apply_tile_events(vec![(0, devices::TileEvent::Connect)]);
        assert_eq!(app.progress_title(), Some(Text::ProgressConnect));
        let context = egui::Context::default();
        for _ in 0..3 {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 450.0),
                    )),
                    ..Default::default()
                },
                |context| app.show_progress(context),
            );
        }
        let rect = context
            .memory(|memory| memory.area_rect(egui::Id::new("action_progress")))
            .unwrap();
        assert!((rect.center().x - 300.0).abs() < 1.0);
        assert!((rect.center().y - 225.0).abs() < 1.0);
        // Other devices' work does not cover the device list.
        app.view = View::Devices;
        assert_eq!(app.progress_title(), None);
    }
    #[test]
    fn frames_render_for_the_grid_and_for_a_connected_tab() {
        let mut app = app(30);
        app.search();
        settle(&mut app);
        frame(&mut app);
        app.apply_tile_events(vec![(4, devices::TileEvent::Connect)]);
        settle(&mut app);
        frame(&mut app);
    }
}
