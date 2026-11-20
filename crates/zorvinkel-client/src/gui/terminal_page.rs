//! The terminal page: login, host key question and the embedded terminal (ADR-030, ADR-032).
use super::terminal_mouse::{self, MouseEvent, Selection};
use super::{
    host_keys::{HostKeyStore, Trust},
    i18n::{Language, Text},
    terminal_colors as colors, terminal_input, theme,
    tunnel_broker::{LiveRoute, RoutedDialer, TunnelBroker},
};
use eframe::egui::{self, Color32, FontId, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{runtime::Runtime, time::timeout};
use zeroize::Zeroizing;
use zorvinkel_terminal::vt100::MouseProtocolMode;
use zorvinkel_terminal::{
    ConnectError, ConnectionStatus, Credentials, ResumableEvent, ResumableTerminal, RetryPolicy,
    Screen, SshConnector, TerminalSize,
};

/// How the terminal reaches `sshd`: over the network, or through the Bluetooth tunnel.
#[derive(Clone)]
pub enum Route {
    Ip(IpAddr),
    Bluetooth(Arc<TunnelBroker>),
}
impl Route {
    /// Both lead to the device the same way (same address, or both through Bluetooth).
    pub fn same_way(&self, other: &Route) -> bool {
        match (self, other) {
            (Self::Ip(a), Self::Ip(b)) => a == b,
            (Self::Bluetooth(_), Self::Bluetooth(_)) => true,
            _ => false,
        }
    }
    fn describe(&self, lang: Language) -> String {
        match self {
            Self::Ip(address) => address.to_string(),
            Self::Bluetooth(_) => lang.t(Text::PathBluetooth).to_owned(),
        }
    }
}
const FONT_SIZE: f32 = 14.0;
const MIN_HEIGHT: f32 = 320.0;
/// Time the cleanup on the device (ADR-032) may take when the terminal is closed.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Exit status of a shell that could not find its command.
const COMMAND_NOT_FOUND: u32 = 127;

enum Link {
    Connecting,
    Connected,
    Reconnecting(u32),
}
enum Ending {
    Finished(u32),
    Failed(String),
}
struct Running {
    terminal: ResumableTerminal,
    runtime: Runtime,
    screen: Screen,
    size: TerminalSize,
    link: Link,
    /// Fingerprint the device presented, as seen by the host key check.
    presented: Arc<Mutex<Option<String>>>,
    /// Kept until the first successful login, for the host key question only (ADR-032).
    credentials: Option<Credentials>,
    route: Route,
    /// Shared with the connector, which follows the device when its way changes.
    live_route: LiveRoute,
    selection: Option<Selection>,
    /// Cell of the last drag report, so a program is told only about changes.
    dragged_over: Option<(u16, u16)>,
    /// Part of a wheel step that did not fill a whole line yet.
    wheel_rest: f32,
}
impl Drop for Running {
    /// Gives the cleanup on the device a moment: the session key is removed once the terminal
    /// has really ended.
    fn drop(&mut self) {
        self.terminal.try_close();
        let terminal = &mut self.terminal;
        self.runtime.block_on(async {
            let _ = timeout(CLOSE_WAIT, async {
                while terminal.next_event().await.is_some() {}
            })
            .await;
        });
    }
}
struct HostKeyQuestion {
    credentials: Credentials,
    route: Route,
    fingerprint: String,
    changed: bool,
}
enum Stage {
    Login,
    Running(Box<Running>),
    HostKey(Box<HostKeyQuestion>),
    Ended(Ending),
}

pub struct TerminalPanel {
    stage: Stage,
    user: String,
    /// Doubles as the passphrase when a key file is given.
    password: Zeroizing<String>,
    /// Path of a private key file; empty means password login.
    key_file: String,
    notice: Option<Text>,
}
impl Default for TerminalPanel {
    fn default() -> Self {
        Self {
            stage: Stage::Login,
            user: String::new(),
            password: Zeroizing::new(String::new()),
            key_file: String::new(),
            notice: None,
        }
    }
}

impl TerminalPanel {
    /// Applies what the connection reported since the last frame.
    pub fn poll(&mut self, device: &str, store: &HostKeyStore) {
        let Stage::Running(running) = &mut self.stage else {
            return;
        };
        let mut next = None;
        loop {
            match running.terminal.try_next_event() {
                Ok(Some(ResumableEvent::Output(bytes))) => running.screen.process(&bytes),
                Ok(Some(ResumableEvent::Status(status))) => {
                    if let Some(stage) = running.apply(status, device, store) {
                        next = Some(stage);
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    next = Some(Next::Ended(Ending::Failed(String::new())));
                    break;
                }
            }
        }
        match next {
            Some(Next::Ended(ending)) => self.stage = Stage::Ended(ending),
            Some(Next::Login(notice)) => {
                self.notice = Some(notice);
                self.stage = Stage::Login;
            }
            Some(Next::HostKey(question)) => self.stage = Stage::HostKey(question),
            None => {}
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        lang: Language,
        device: &str,
        route: Option<Route>,
        store: &mut HostKeyStore,
    ) {
        if let Stage::Running(running) = &mut self.stage {
            running.follow(route.as_ref());
        }
        match &self.stage {
            Stage::Login => self.show_login(ui, lang, device, route, store),
            Stage::HostKey(_) => self.show_host_key(ui, lang, device, store),
            Stage::Ended(_) => self.show_ended(ui, lang),
            Stage::Running(_) => self.show_running(ui, lang),
        }
    }

    fn show_login(
        &mut self,
        ui: &mut egui::Ui,
        lang: Language,
        device: &str,
        route: Option<Route>,
        store: &HostKeyStore,
    ) {
        ui.label(lang.t(Text::TerminalIntro));
        ui.add_space(8.0);
        if let Some(notice) = self.notice {
            theme::message(ui, lang.t(notice), theme::ERROR, theme::ERROR_PALE);
            ui.add_space(8.0);
        }
        let Some(route) = route else {
            ui.label(lang.t(Text::TerminalNoAddress));
            return;
        };
        egui::Grid::new("terminal_login")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(lang.t(Text::TerminalUser));
                ui.text_edit_singleline(&mut self.user);
                ui.end_row();
                ui.label(lang.t(Text::TerminalPassword));
                ui.add(egui::TextEdit::singleline(&mut *self.password).password(true));
                ui.end_row();
                ui.label(lang.t(Text::TerminalKeyFile));
                ui.text_edit_singleline(&mut self.key_file)
                    .on_hover_text(lang.t(Text::TerminalKeyFileHint));
                ui.end_row();
            });
        ui.add_space(8.0);
        let ready = !self.user.trim().is_empty();
        let open = ui.add_enabled(ready, theme::primary_button(lang.t(Text::TerminalOpen)));
        if open.clicked() {
            match self.credentials_from_form() {
                Ok(credentials) => self.start(ui.ctx(), device, route, credentials, store),
                Err(notice) => self.notice = Some(notice),
            }
        }
    }

    /// Takes the secret out of the form: it is not kept after the click.
    fn credentials_from_form(&mut self) -> Result<Credentials, Text> {
        let secret = Zeroizing::new(std::mem::take(&mut *self.password));
        let user = self.user.trim();
        let path = self.key_file.trim();
        if path.is_empty() {
            return Credentials::password(user, secret.to_string())
                .map_err(|_| Text::TerminalLoginFailed);
        }
        let key_text = std::fs::read_to_string(path)
            .map(Zeroizing::new)
            .map_err(|_| Text::TerminalKeyUnreadable)?;
        let passphrase = (!secret.is_empty()).then_some(secret.as_str());
        Credentials::private_key(user, &key_text, passphrase).map_err(|_| Text::TerminalKeyInvalid)
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        device: &str,
        route: Route,
        credentials: Credentials,
        store: &HostKeyStore,
    ) {
        self.notice = None;
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                self.stage = Stage::Ended(Ending::Failed(error.to_string()));
                return;
            }
        };
        let known = store.stored(device).map(str::to_owned);
        let presented = Arc::new(Mutex::new(None));
        let recorder = presented.clone();
        let verifier = move |fingerprint: &str| {
            if let Ok(mut slot) = recorder.lock() {
                *slot = Some(fingerprint.to_owned());
            }
            known.as_deref() == Some(fingerprint)
        };
        let live_route: LiveRoute = Arc::new(Mutex::new(Some(route.clone())));
        let connector = {
            let context = ctx.clone();
            SshConnector::with_dialer(
                Arc::new(RoutedDialer {
                    route: live_route.clone(),
                    wake: Arc::new(move || context.request_repaint()),
                }),
                credentials.clone(),
                Arc::new(verifier),
            )
        };
        let size = TerminalSize::new(100, 30).expect("fixed size is not empty");
        let wake_context = ctx.clone();
        let terminal = {
            let _runtime = runtime.enter();
            ResumableTerminal::start(connector, size, RetryPolicy::default(), move || {
                wake_context.request_repaint();
            })
        };
        self.stage = Stage::Running(Box::new(Running {
            terminal,
            runtime,
            screen: Screen::new(size),
            size,
            link: Link::Connecting,
            presented,
            credentials: Some(credentials),
            route,
            live_route,
            selection: None,
            dragged_over: None,
            wheel_rest: 0.0,
        }));
    }

    fn show_host_key(
        &mut self,
        ui: &mut egui::Ui,
        lang: Language,
        device: &str,
        store: &mut HostKeyStore,
    ) {
        let Stage::HostKey(question) = &self.stage else {
            return;
        };
        let (text, accept) = if question.changed {
            (Text::TerminalHostKeyChanged, Text::TerminalReplaceKey)
        } else {
            (Text::TerminalHostKeyNew, Text::TerminalTrustKey)
        };
        let color = if question.changed {
            theme::ERROR
        } else {
            theme::AMBER
        };
        ui.label(RichText::new(lang.t(text)).color(color));
        ui.add_space(6.0);
        ui.label(RichText::new(&question.fingerprint).monospace().strong());
        ui.add_space(10.0);
        let mut decision = None;
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button(lang.t(accept))).clicked() {
                decision = Some(true);
            }
            if ui.button(lang.t(Text::TerminalCancel)).clicked() {
                decision = Some(false);
            }
        });
        let Some(trusted) = decision else {
            return;
        };
        let Stage::HostKey(question) = std::mem::replace(&mut self.stage, Stage::Login) else {
            return;
        };
        if trusted {
            store.remember(device, &question.fingerprint);
            self.start(
                ui.ctx(),
                device,
                question.route,
                question.credentials,
                store,
            );
        }
    }

    fn show_ended(&mut self, ui: &mut egui::Ui, lang: Language) {
        let Stage::Ended(ending) = &self.stage else {
            return;
        };
        let (text, color) = match ending {
            Ending::Finished(COMMAND_NOT_FOUND) => {
                (lang.t(Text::TerminalTmuxMissing).to_owned(), theme::ERROR)
            }
            Ending::Finished(code) => (lang.format(Text::TerminalFinished, &[code]), theme::MUTED),
            Ending::Failed(reason) => (lang.format(Text::TerminalFailed, &[reason]), theme::ERROR),
        };
        ui.label(RichText::new(text).color(color));
        ui.add_space(8.0);
        if ui.button(lang.t(Text::TerminalOpenAgain)).clicked() {
            self.stage = Stage::Login;
        }
    }

    fn show_running(&mut self, ui: &mut egui::Ui, lang: Language) {
        let mut close = false;
        let Stage::Running(running) = &mut self.stage else {
            return;
        };
        ui.horizontal(|ui| {
            let status = match running.link {
                Link::Connecting => lang.t(Text::TerminalConnecting).to_owned(),
                Link::Connected => lang.t(Text::TerminalConnected).to_owned(),
                Link::Reconnecting(attempt) => lang.format(Text::TerminalReconnecting, &[&attempt]),
            };
            let color = match running.link {
                Link::Connected => theme::GREEN,
                _ => theme::AMBER,
            };
            ui.label(RichText::new(status).small().color(color));
            ui.label(
                RichText::new(format!("· {}", running.route.describe(lang)))
                    .small()
                    .color(theme::MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button(lang.t(Text::TerminalClose)).clicked();
            });
        });
        ui.add_space(6.0);
        running.draw(ui);
        if close {
            self.stage = Stage::Login;
        }
    }
}

enum Next {
    Ended(Ending),
    Login(Text),
    HostKey(Box<HostKeyQuestion>),
}

impl Running {
    /// Passes the device's current way on to the connector; it applies from the next attempt.
    fn follow(&mut self, route: Option<&Route>) {
        if let Ok(mut live) = self.live_route.lock() {
            *live = route.cloned();
        }
        if let Some(route) = route {
            self.route = route.clone();
        }
    }
    /// `Some` when the connection ended for good and the page has to change.
    fn apply(
        &mut self,
        status: ConnectionStatus,
        device: &str,
        store: &HostKeyStore,
    ) -> Option<Next> {
        match status {
            ConnectionStatus::Connecting => self.link = Link::Connecting,
            ConnectionStatus::Connected => {
                self.link = Link::Connected;
                self.screen.reset(self.size);
                // From here on the session key carries the terminal (ADR-032).
                self.credentials = None;
            }
            ConnectionStatus::Reconnecting { attempt } => self.link = Link::Reconnecting(attempt),
            ConnectionStatus::Finished(code) => {
                return Some(Next::Ended(Ending::Finished(code)));
            }
            ConnectionStatus::Failed(error) => return Some(self.failure(error, device, store)),
        }
        None
    }
    fn failure(&mut self, error: ConnectError, device: &str, store: &HostKeyStore) -> Next {
        match error {
            ConnectError::HostKeyRejected => {
                let presented = self.presented.lock().ok().and_then(|slot| slot.clone());
                match (self.credentials.take(), presented) {
                    (Some(credentials), Some(fingerprint)) => {
                        let changed = store.check(device, &fingerprint) == Trust::Changed;
                        Next::HostKey(Box::new(HostKeyQuestion {
                            credentials,
                            route: self.route.clone(),
                            fingerprint,
                            changed,
                        }))
                    }
                    _ => Next::Login(Text::TerminalLoginAgain),
                }
            }
            ConnectError::AuthenticationFailed => Next::Login(if self.credentials.is_some() {
                Text::TerminalLoginFailed
            } else {
                Text::TerminalLoginAgain
            }),
            ConnectError::Network(reason) | ConnectError::Refused(reason) => {
                Next::Ended(Ending::Failed(reason))
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(FONT_SIZE);
        let cell = ui
            .painter()
            .layout_no_wrap("M".into(), font.clone(), Color32::WHITE)
            .size();
        let available = ui.available_size();
        let area = Vec2::new(available.x, available.y.max(MIN_HEIGHT));
        let (rect, response) = ui.allocate_exact_size(area, Sense::click_and_drag());
        let id = response.id;
        if response.clicked()
            || !ui.memory(|memory| memory.has_focus(id)) && ui.memory(|m| m.focused().is_none())
        {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        self.fit_to(rect, cell);
        let focused = ui.memory(|memory| memory.has_focus(id));
        if focused {
            self.send_keyboard(ui);
        }
        self.handle_pointer(ui, &response, rect, cell);
        self.paint(ui, rect, cell, font, focused);
    }

    /// Copies the selection on request; otherwise sends what was typed.
    fn send_keyboard(&mut self, ui: &egui::Ui) {
        let events = ui.input(|input| input.events.clone());
        let copy_shortcut = |event: &egui::Event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::C,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.ctrl && modifiers.shift
            )
        };
        let copy_requested = events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy) || copy_shortcut(event));
        if copy_requested {
            // A plain Ctrl+C without a selection still reaches the program as ^C.
            if self.copy_selection(ui.ctx()) || events.iter().any(copy_shortcut) {
                return;
            }
        }
        let bytes =
            terminal_input::bytes_for_events(&events, self.screen.view().application_cursor());
        if !bytes.is_empty() {
            self.selection = None;
            self.screen.scroll_to_present();
            let _ = self.terminal.try_input(bytes);
        }
    }
    /// `true` when there was a selection to copy.
    fn copy_selection(&mut self, ctx: &egui::Context) -> bool {
        let Some(selection) = self.selection.take().filter(|s| !s.is_empty()) else {
            return false;
        };
        let (_, columns) = self.screen.view().size();
        let (first_row, first_column, last_row, last_column) = selection.text_range(columns.get());
        let text =
            self.screen
                .view()
                .contents_between(first_row, first_column, last_row, last_column);
        ctx.copy_text(text);
        true
    }

    /// Wheel and buttons: reported to programs that ask for the mouse (`tmux` does), otherwise
    /// they scroll the local history and select text. Shift keeps the local behaviour.
    fn handle_pointer(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, cell: Vec2) {
        let (rows, columns) = self.screen.view().size();
        // The cell under a position, kept inside the screen.
        let cell_at = move |position: egui::Pos2| {
            let offset = position - rect.min;
            let column = (offset.x / cell.x)
                .floor()
                .clamp(0.0, f32::from(columns.get() - 1));
            let row = (offset.y / cell.y)
                .floor()
                .clamp(0.0, f32::from(rows.get() - 1));
            (row as u16, column as u16)
        };
        let (mode, encoding) = {
            let view = self.screen.view();
            (view.mouse_protocol_mode(), view.mouse_protocol_encoding())
        };
        let local = ui.input(|input| input.modifiers.shift) || mode == MouseProtocolMode::None;
        let report = |terminal: &ResumableTerminal, event, (row, column)| {
            let bytes = terminal_mouse::report(mode, encoding, event, row, column);
            if !bytes.is_empty() {
                let _ = terminal.try_input(bytes);
            }
        };
        let hover = response.hover_pos().map(cell_at);
        if let Some(position) = hover {
            self.wheel_rest += ui.input(|input| input.raw_scroll_delta.y) / cell.y;
            let lines = self.wheel_rest.trunc();
            self.wheel_rest -= lines;
            if lines != 0.0 {
                if local {
                    self.screen.scroll_back(lines as isize);
                } else {
                    let event = if lines > 0.0 {
                        MouseEvent::WheelUp
                    } else {
                        MouseEvent::WheelDown
                    };
                    for _ in 0..lines.abs() as usize {
                        report(&self.terminal, event, position);
                    }
                }
            }
        }
        let pointer = |origin: bool| {
            let position = if origin {
                ui.input(|input| input.pointer.press_origin())
            } else {
                response.interact_pointer_pos()
            };
            position.map(cell_at)
        };
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(start) = pointer(true) else { return };
            if local {
                self.selection = Some(Selection::start(start));
            } else {
                self.dragged_over = Some(start);
                report(&self.terminal, MouseEvent::Press, start);
            }
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let Some(current) = pointer(false) else {
                return;
            };
            if local {
                if let Some(selection) = &mut self.selection {
                    selection.extend_to(current);
                }
            } else if self.dragged_over != Some(current) {
                self.dragged_over = Some(current);
                report(&self.terminal, MouseEvent::Drag, current);
            }
        } else if response.drag_stopped_by(egui::PointerButton::Primary) {
            if !local && let Some(current) = pointer(false) {
                report(&self.terminal, MouseEvent::Release, current);
            }
            self.dragged_over = None;
        } else if response.clicked_by(egui::PointerButton::Primary) {
            self.selection = None;
            if !local && let Some(current) = pointer(false) {
                report(&self.terminal, MouseEvent::Press, current);
                report(&self.terminal, MouseEvent::Release, current);
            }
        }
    }

    /// Makes the terminal as large as the area allows.
    fn fit_to(&mut self, rect: Rect, cell: Vec2) {
        let columns = (rect.width() / cell.x).floor().clamp(1.0, 500.0) as u16;
        let rows = (rect.height() / cell.y).floor().clamp(1.0, 200.0) as u16;
        let Some(size) = TerminalSize::new(columns, rows) else {
            return;
        };
        if size != self.size {
            self.size = size;
            self.screen.resize(size);
            let _ = self.terminal.try_resize(size);
        }
    }

    fn paint(&self, ui: &egui::Ui, rect: Rect, cell: Vec2, font: FontId, focused: bool) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, colors::DEFAULT_BACKGROUND);
        let screen = self.screen.view();
        let (rows, columns) = screen.size();
        for row in 0..rows.get() {
            let top = rect.top() + f32::from(row) * cell.y;
            let mut run = Run::default();
            for column in 0..columns.get() {
                let Some(content) = screen.cell(row, column) else {
                    continue;
                };
                let (mut foreground, mut background) = (
                    colors::color(content.fgcolor(), colors::DEFAULT_FOREGROUND),
                    colors::color(content.bgcolor(), colors::DEFAULT_BACKGROUND),
                );
                if content.inverse() {
                    std::mem::swap(&mut foreground, &mut background);
                }
                let left = rect.left() + f32::from(column) * cell.x;
                if background != colors::DEFAULT_BACKGROUND {
                    let width = if content.is_wide() {
                        2.0 * cell.x
                    } else {
                        cell.x
                    };
                    painter.rect_filled(
                        Rect::from_min_size(egui::pos2(left, top), Vec2::new(width, cell.y)),
                        0.0,
                        background,
                    );
                }
                if self
                    .selection
                    .is_some_and(|selection| selection.contains(row, column))
                {
                    let width = if content.is_wide() {
                        2.0 * cell.x
                    } else {
                        cell.x
                    };
                    painter.rect_filled(
                        Rect::from_min_size(egui::pos2(left, top), Vec2::new(width, cell.y)),
                        0.0,
                        Color32::from_rgba_unmultiplied(120, 160, 255, 90),
                    );
                }
                if content.underline() {
                    let y = top + cell.y - 1.0;
                    painter.line_segment(
                        [egui::pos2(left, y), egui::pos2(left + cell.x, y)],
                        Stroke::new(1.0_f32, foreground),
                    );
                }
                if content.is_wide_continuation() {
                    continue;
                }
                let text = if content.has_contents() {
                    content.contents()
                } else {
                    " "
                };
                if run.color != foreground {
                    run.flush(&painter, top, cell, &font);
                    run = Run::starting(left, foreground);
                }
                run.text.push_str(text);
            }
            run.flush(&painter, top, cell, &font);
        }
        if !screen.hide_cursor() && screen.scrollback() == 0 {
            let (row, column) = screen.cursor_position();
            let origin = rect.min + Vec2::new(f32::from(column) * cell.x, f32::from(row) * cell.y);
            let cursor = Rect::from_min_size(origin, cell);
            if focused {
                painter.rect_filled(cursor, 0.0, colors::DEFAULT_FOREGROUND.gamma_multiply(0.6));
            } else {
                painter.rect_stroke(
                    cursor,
                    0.0,
                    Stroke::new(1.0_f32, colors::DEFAULT_FOREGROUND),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

/// Consecutive cells of one colour, painted as a single piece of text.
struct Run {
    left: f32,
    color: Color32,
    text: String,
}
impl Default for Run {
    fn default() -> Self {
        Self::starting(0.0, Color32::TRANSPARENT)
    }
}
impl Run {
    fn starting(left: f32, color: Color32) -> Self {
        Self {
            left,
            color,
            text: String::new(),
        }
    }
    fn flush(&self, painter: &egui::Painter, top: f32, _cell: Vec2, font: &FontId) {
        if self.text.trim().is_empty() {
            return;
        }
        painter.text(
            egui::pos2(self.left, top),
            egui::Align2::LEFT_TOP,
            &self.text,
            font.clone(),
            self.color,
        );
    }
}

impl super::session::Session {
    /// The terminal needs a way to the device's `sshd`: the tunnel when the device is connected
    /// over Bluetooth (which is where the network did not work), else its network address.
    pub fn terminal_offered(&self) -> bool {
        self.terminal_route().is_some()
    }
    fn terminal_route(&self) -> Option<Route> {
        if self.snapshot.tunnel_available
            && self.snapshot.transport == Some(zorvinkel_application::setup::Transport::Ble)
        {
            return Some(Route::Bluetooth(self.tunnel_broker.clone()));
        }
        let address = self.snapshot.system_info.as_ref()?.ssh_address()?;
        Some(Route::Ip(address))
    }
    /// The connector waits for a tunnel and the worker is free to open one.
    pub fn wants_tunnel(&self) -> bool {
        !self.tunnel_job && self.is_connected() && self.tunnel_broker.has_asks()
    }
    /// Hands the result of an `OpenTunnel` job to the connector that asked.
    pub fn finish_tunnel(
        &mut self,
        result: Result<
            zorvinkel_application::setup::SetupUpdate,
            zorvinkel_application::setup::SetupError,
        >,
    ) {
        use super::tunnel_broker::TunnelFailure;
        use zorvinkel_application::setup::SetupError;
        self.tunnel_job = false;
        let answer = match result {
            Ok(update) => match update.error {
                None => match update.snapshot.tunnel.take::<tokio::io::DuplexStream>() {
                    Some(stream) => Ok(Box::new(stream) as Box<dyn zorvinkel_terminal::ByteStream>),
                    None => Err(TunnelFailure::Unreachable),
                },
                Some(SetupError::Busy) => Err(TunnelFailure::Busy),
                Some(SetupError::Unsupported) => Err(TunnelFailure::NotOffered),
                Some(_) => Err(TunnelFailure::Unreachable),
            },
            Err(_) => Err(TunnelFailure::Unreachable),
        };
        self.tunnel_broker.answer(answer);
    }
    pub fn terminal_page(
        &mut self,
        ui: &mut egui::Ui,
        lang: Language,
        host_keys: &mut HostKeyStore,
    ) {
        let route = self.terminal_route();
        let panel = self.terminal.get_or_insert_with(TerminalPanel::default);
        panel.show(ui, lang, &self.handle, route, host_keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(panel: &mut TerminalPanel, route: Option<Route>, lang: Language) {
        let context = egui::Context::default();
        let mut store = HostKeyStore::none();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                panel.poll("pi", &store);
                panel.show(ui, lang, "pi", route.clone(), &mut store);
            });
        });
    }

    #[test]
    fn login_renders_with_and_without_an_address_in_both_languages() {
        for lang in Language::ALL {
            let mut panel = TerminalPanel::default();
            render(&mut panel, None, lang);
            render(&mut panel, "192.168.1.20".parse().ok().map(Route::Ip), lang);
            render(&mut panel, Some(Route::Bluetooth(Default::default())), lang);
        }
    }
    #[test]
    fn host_key_question_and_endings_render() {
        let credentials = Credentials::password("pi", "x".into()).unwrap();
        for changed in [false, true] {
            let mut panel = TerminalPanel {
                stage: Stage::HostKey(Box::new(HostKeyQuestion {
                    credentials: credentials.clone(),
                    route: Route::Bluetooth(Default::default()),
                    fingerprint: "SHA256:abc".into(),
                    changed,
                })),
                ..TerminalPanel::default()
            };
            render(
                &mut panel,
                "192.168.1.20".parse().ok().map(Route::Ip),
                Language::English,
            );
        }
        for ending in [
            Ending::Finished(0),
            Ending::Finished(COMMAND_NOT_FOUND),
            Ending::Failed("timed out".into()),
        ] {
            let mut panel = TerminalPanel {
                stage: Stage::Ended(ending),
                ..TerminalPanel::default()
            };
            render(&mut panel, None, Language::German);
        }
    }
}
