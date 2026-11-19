//! The terminal page: login, host key question and the embedded terminal (ADR-030, ADR-032).
use super::{
    host_keys::{HostKeyStore, Trust},
    i18n::{Language, Text},
    terminal_colors as colors, terminal_input, theme,
};
use eframe::egui::{self, Color32, FontId, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{runtime::Runtime, time::timeout};
use zeroize::Zeroizing;
use zorvinkel_application::system::NetworkAddress;
use zorvinkel_terminal::{
    ConnectError, ConnectionStatus, Credentials, ResumableEvent, ResumableTerminal, RetryPolicy,
    Screen, SshConnector, TerminalSize,
};

const SSH_PORT: u16 = 22;
const FONT_SIZE: f32 = 14.0;
const MIN_HEIGHT: f32 = 320.0;
/// Time the cleanup on the device (ADR-032) may take when the terminal is closed.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Exit status of a shell that could not find its command.
const COMMAND_NOT_FOUND: u32 = 127;

/// The address the client should use for SSH: a routable IPv4 address if there is one.
pub fn pick_address(addresses: &[NetworkAddress]) -> Option<IpAddr> {
    let usable = |address: &&NetworkAddress| match address.address {
        IpAddr::V4(ip) => !ip.is_loopback() && !ip.is_link_local(),
        // A link-local IPv6 address needs a scope that the device cannot tell us.
        IpAddr::V6(ip) => !ip.is_loopback() && (ip.segments()[0] & 0xffc0) != 0xfe80,
    };
    let mut candidates = addresses.iter().filter(usable);
    let first_v4 = addresses
        .iter()
        .filter(usable)
        .find(|address| address.address.is_ipv4());
    first_v4.or_else(|| candidates.next()).map(|a| a.address)
}

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
    address: IpAddr,
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
    address: IpAddr,
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
    password: Zeroizing<String>,
    notice: Option<Text>,
}
impl Default for TerminalPanel {
    fn default() -> Self {
        Self {
            stage: Stage::Login,
            user: String::new(),
            password: Zeroizing::new(String::new()),
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
        addresses: &[NetworkAddress],
        store: &mut HostKeyStore,
    ) {
        let address = pick_address(addresses);
        match &self.stage {
            Stage::Login => self.show_login(ui, lang, device, address, store),
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
        address: Option<IpAddr>,
        store: &HostKeyStore,
    ) {
        ui.label(lang.t(Text::TerminalIntro));
        ui.add_space(8.0);
        if let Some(notice) = self.notice {
            theme::message(ui, lang.t(notice), theme::ERROR, theme::ERROR_PALE);
            ui.add_space(8.0);
        }
        let Some(address) = address else {
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
            });
        ui.add_space(8.0);
        let ready = !self.user.trim().is_empty();
        let open = ui.add_enabled(ready, theme::primary_button(lang.t(Text::TerminalOpen)));
        if open.clicked() {
            let password = std::mem::take(&mut *self.password);
            match Credentials::password(self.user.trim(), password.to_string()) {
                Ok(credentials) => self.start(ui.ctx(), device, address, credentials, store),
                Err(_) => self.notice = Some(Text::TerminalLoginFailed),
            }
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        device: &str,
        address: IpAddr,
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
        let connector = SshConnector::new(
            SocketAddr::new(address, SSH_PORT).to_string(),
            credentials.clone(),
            Arc::new(verifier),
        );
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
            address,
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
                question.address,
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
                RichText::new(format!("· {}", running.address))
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
                            address: self.address,
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
        let (rect, response) = ui.allocate_exact_size(area, Sense::click());
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
            let events = ui.input(|input| input.events.clone());
            let bytes =
                terminal_input::bytes_for_events(&events, self.screen.view().application_cursor());
            if !bytes.is_empty() {
                let _ = self.terminal.try_input(bytes);
            }
        }
        self.paint(ui, rect, cell, font, focused);
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
        if !screen.hide_cursor() {
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
    /// The terminal needs an address of the device to reach its `sshd`.
    pub fn terminal_offered(&self) -> bool {
        self.terminal_address().is_some()
    }
    fn terminal_address(&self) -> Option<IpAddr> {
        pick_address(&self.snapshot.system_info.as_ref()?.addresses)
    }
    pub fn terminal_page(
        &mut self,
        ui: &mut egui::Ui,
        lang: Language,
        host_keys: &mut HostKeyStore,
    ) {
        let addresses = self
            .snapshot
            .system_info
            .as_ref()
            .map(|info| info.addresses.clone())
            .unwrap_or_default();
        let panel = self.terminal.get_or_insert_with(TerminalPanel::default);
        panel.show(ui, lang, &self.handle, &addresses, host_keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> NetworkAddress {
        NetworkAddress {
            interface: "eth0".into(),
            address: text.parse().unwrap(),
        }
    }

    fn render(panel: &mut TerminalPanel, addresses: &[NetworkAddress], lang: Language) {
        let context = egui::Context::default();
        let mut store = HostKeyStore::none();
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                panel.poll("pi", &store);
                panel.show(ui, lang, "pi", addresses, &mut store);
            });
        });
    }

    #[test]
    fn login_renders_with_and_without_an_address_in_both_languages() {
        for lang in Language::ALL {
            let mut panel = TerminalPanel::default();
            render(&mut panel, &[], lang);
            render(&mut panel, &[address("192.168.1.20")], lang);
        }
    }
    #[test]
    fn host_key_question_and_endings_render() {
        let credentials = Credentials::password("pi", "x".into()).unwrap();
        for changed in [false, true] {
            let mut panel = TerminalPanel {
                stage: Stage::HostKey(Box::new(HostKeyQuestion {
                    credentials: credentials.clone(),
                    address: "192.168.1.20".parse().unwrap(),
                    fingerprint: "SHA256:abc".into(),
                    changed,
                })),
                ..TerminalPanel::default()
            };
            render(&mut panel, &[address("192.168.1.20")], Language::English);
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
            render(&mut panel, &[], Language::German);
        }
    }

    #[test]
    fn prefers_a_routable_ipv4_address() {
        let addresses = [
            address("fe80::1"),
            address("2001:db8::7"),
            address("127.0.0.1"),
            address("169.254.3.4"),
            address("192.168.1.20"),
        ];
        assert_eq!(pick_address(&addresses), "192.168.1.20".parse().ok());
    }
    #[test]
    fn falls_back_to_a_global_ipv6_address() {
        let addresses = [address("fe80::1"), address("2001:db8::7")];
        assert_eq!(pick_address(&addresses), "2001:db8::7".parse().ok());
    }
    #[test]
    fn local_addresses_alone_are_no_address() {
        let addresses = [address("127.0.0.1"), address("::1"), address("fe80::2")];
        assert_eq!(pick_address(&addresses), None);
        assert_eq!(pick_address(&[]), None);
    }
}
