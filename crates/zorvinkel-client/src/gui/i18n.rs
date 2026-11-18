//! Languages of the GUI and every text it shows.
//!
//! State holds typed identifiers (`Text` and the enums built on it), never rendered strings, so a
//! language switch changes everything on the next frame. A language is added by adding it to
//! `Language`, a column to every entry of `texts!` and its formatting rules; the compiler lists
//! every entry that still lacks the new column.
use std::fmt::Display;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    #[default]
    English,
    German,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::German];

    /// The language of the operating system's preferred locale, English if it is not supported.
    pub fn detect() -> Self {
        sys_locale::get_locale()
            .and_then(|tag| Self::from_tag(&tag))
            .unwrap_or_default()
    }
    /// Reads the primary subtag of a BCP 47 or POSIX locale name such as `de-DE` or `de_DE.UTF-8`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Self::English),
            "de" => Some(Self::German),
            _ => None,
        }
    }
    /// The locale tag stored for the language.
    pub fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::German => "de",
        }
    }
    /// The language's name in that language, for the selector.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::German => "Deutsch",
        }
    }
    pub fn t(self, text: Text) -> &'static str {
        match self {
            Self::English => text.english(),
            Self::German => text.german(),
        }
    }
    /// Fills `{0}`, `{1}`, … of a text; the position lets a language reorder the values.
    pub fn format(self, text: Text, values: &[&dyn Display]) -> String {
        fill(self.t(text), values)
    }
    /// Writes a number formatted with a decimal point the way the language writes it.
    pub fn decimal(self, number: String) -> String {
        match self {
            Self::English => number,
            Self::German => number.replace('.', ","),
        }
    }
}

/// Remembers the chosen language in a one-line file of the client's data directory.
pub struct LanguageStore {
    path: Option<std::path::PathBuf>,
}

impl LanguageStore {
    const FILE: &'static str = "language";

    pub fn in_directory(directory: &std::path::Path) -> Self {
        Self {
            path: Some(directory.join(Self::FILE)),
        }
    }
    /// For runs that must not touch the client's data, such as the simulation.
    pub fn none() -> Self {
        Self { path: None }
    }
    pub fn load(&self) -> Option<Language> {
        let text = std::fs::read_to_string(self.path.as_ref()?).ok()?;
        Language::from_tag(text.trim())
    }
    pub fn save(&self, language: Language) {
        let Some(path) = &self.path else {
            return;
        };
        if let Err(error) = std::fs::write(path, language.tag()) {
            eprintln!("Language setting not saved: {error}");
        }
    }
}

fn fill(template: &str, values: &[&dyn Display]) -> String {
    values
        .iter()
        .enumerate()
        .fold(template.to_string(), |text, (index, value)| {
            text.replace(&format!("{{{index}}}"), &value.to_string())
        })
}

macro_rules! texts {
    ($($variant:ident { en: $en:expr, de: $de:expr $(,)? }),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Text {
            $($variant),+
        }
        impl Text {
            #[cfg(test)]
            const ALL: &'static [Self] = &[$(Self::$variant),+];
            fn english(self) -> &'static str {
                match self {
                    $(Self::$variant => $en),+
                }
            }
            fn german(self) -> &'static str {
                match self {
                    $(Self::$variant => $de),+
                }
            }
        }
    };
}

texts! {
    // Window and device list
    Language { en: "Language", de: "Sprache" },
    Devices { en: "Devices", de: "Geräte" },
    Searching { en: "Searching …", de: "Suche läuft …" },
    FoundCount { en: "{0} found", de: "{0} gefunden" },
    SearchAgain { en: "Search again", de: "Erneut suchen" },
    NoDevicesFound { en: "No devices found.", de: "Keine Geräte gefunden." },
    CloseTab { en: "Close tab", de: "Tab schließen" },
    SearchNotPermitted {
        en: "Searching is not permitted on this computer.",
        de: "Die Suche ist an diesem Computer nicht freigegeben.",
    },
    SearchFailed {
        en: "The search failed. Check Bluetooth and search again.",
        de: "Die Suche ist fehlgeschlagen. Prüfe Bluetooth und suche erneut.",
    },
    Connect { en: "Connect", de: "Verbinden" },
    Disconnect { en: "Disconnect", de: "Trennen" },
    Cancel { en: "Cancel", de: "Abbrechen" },
    RequestInProgress { en: "Request in progress", de: "Anfrage wird bearbeitet" },

    // Connection state
    StateConnected { en: "Connected", de: "Verbunden" },
    StateFailed { en: "Failed", de: "Fehlgeschlagen" },
    StateDisconnected { en: "Disconnected", de: "Getrennt" },
    StateOutOfRange { en: "Not found", de: "Nicht gefunden" },
    NotConnected { en: "Not connected", de: "Nicht verbunden" },
    PathBluetooth { en: "Bluetooth", de: "Bluetooth" },
    PathNetwork { en: "Network (IP)", de: "Netzwerk (IP)" },
    PathSimulation { en: "Simulation", de: "Simulation" },

    // Titles of running requests: the overlay and the short label of tiles and tabs
    ProgressSearching { en: "Searching for devices …", de: "Geräte werden gesucht …" },
    ProgressConnect { en: "Connecting …", de: "Verbindung wird hergestellt …" },
    ProgressDisconnect { en: "Disconnecting …", de: "Verbindung wird getrennt …" },
    ProgressScan { en: "Searching for Wi-Fi networks …", de: "WLANs werden gesucht …" },
    ProgressWifi { en: "Setting up the Wi-Fi connection …", de: "WLAN-Verbindung wird eingerichtet …" },
    ProgressCredentials { en: "Reading the credentials …", de: "Zugangsdaten werden gelesen …" },
    ProgressPersistence { en: "Saving the setting …", de: "Einstellung wird gespeichert …" },
    ProgressShutdown { en: "Shutting the device down …", de: "Gerät wird heruntergefahren …" },
    ProgressRestart { en: "Restarting the device …", de: "Gerät wird neu gestartet …" },
    ProgressServiceStart { en: "Starting the service …", de: "Dienst wird gestartet …" },
    ProgressServiceStop { en: "Stopping the service …", de: "Dienst wird gestoppt …" },
    ProgressServiceRestart { en: "Restarting the service …", de: "Dienst wird neu gestartet …" },
    ProgressAccessPoint { en: "Switching the device Wi-Fi …", de: "Geräte-WLAN wird umgeschaltet …" },
    ProgressAccessPointSettings { en: "Applying the credentials …", de: "Zugangsdaten werden übernommen …" },
    ProgressRestartOption { en: "Applying the restart option …", de: "Neustartoption wird übernommen …" },
    ProgressRefresh { en: "Reading the device state …", de: "Gerätezustand wird gelesen …" },
    LabelConnect { en: "Connecting …", de: "Verbindet …" },
    LabelDisconnect { en: "Disconnecting …", de: "Trennt …" },
    LabelScan { en: "Searching Wi-Fi …", de: "Sucht WLANs …" },
    LabelWifi { en: "Setting up Wi-Fi …", de: "Richtet WLAN ein …" },
    LabelCredentials { en: "Reading credentials …", de: "Liest Zugangsdaten …" },
    LabelPersistence { en: "Saving setting …", de: "Speichert Einstellung …" },
    LabelShutdown { en: "Shutting down …", de: "Fährt herunter …" },
    LabelRestart { en: "Restarting …", de: "Startet neu …" },
    LabelServiceStart { en: "Starting service …", de: "Startet Dienst …" },
    LabelServiceStop { en: "Stopping service …", de: "Stoppt Dienst …" },
    LabelServiceRestart { en: "Restarting service …", de: "Startet Dienst neu …" },
    LabelAccessPoint { en: "Switching device Wi-Fi …", de: "Schaltet Geräte-WLAN um …" },
    LabelAccessPointSettings { en: "Applying credentials …", de: "Übernimmt Zugangsdaten …" },
    LabelRestartOption { en: "Applying restart option …", de: "Übernimmt Neustartoption …" },
    LabelRefresh { en: "Reading device state …", de: "Liest Gerätezustand …" },

    // Notices of a device tab
    NoticeFallbackToBluetooth {
        en: "The network connection was lost. The connection now runs over Bluetooth.",
        de: "Die Netzwerkverbindung ging verloren. Die Verbindung läuft jetzt über Bluetooth.",
    },
    NoticeCancelled {
        en: "Cancelled. The current device state was read.",
        de: "Abgebrochen. Der aktuelle Gerätezustand wurde gelesen.",
    },
    NoticeShuttingDown {
        en: "The device is shutting down. To turn it on again, disconnect it from power or switch it on.",
        de: "Das Gerät fährt herunter. Zum erneuten Einschalten muss es vom Strom getrennt oder eingeschaltet werden.",
    },
    NoticeRestarting {
        en: "The device is restarting. Search for it again in about a minute.",
        de: "Das Gerät startet neu. Suche in etwa einer Minute erneut nach dem Gerät.",
    },
    NoticeEndedBySearch {
        en: "Connection ended by the search.",
        de: "Verbindung durch die Suche beendet.",
    },
    NoticeTakenOver {
        en: "Disconnected because another device was connected.",
        de: "Getrennt, weil ein anderes Gerät verbunden wurde.",
    },
    SessionEndsIn {
        en: "The secured session ends in {0} s. Renew it to stay connected.",
        de: "Die gesicherte Sitzung endet in {0} s. Erneuere sie, um verbunden zu bleiben.",
    },
    RenewNow { en: "Renew now", de: "Jetzt erneuern" },
    NoWifiSetupOffered {
        en: "This device does not offer Wi-Fi setup yet. Update the device service.",
        de: "Dieses Gerät bietet noch keine WLAN-Einrichtung an. Aktualisiere den Gerätedienst.",
    },

    // Errors
    ErrorIdentityRejected {
        en: "The device identity does not match the stored key. Have the administrator check the device.",
        de: "Die Geräteidentität stimmt nicht mit dem gespeicherten Schlüssel überein. Lass das Gerät vom Administrator prüfen.",
    },
    ErrorAccessDenied {
        en: "This computer is not authorised. Ask the administrator to authorise it specifically.",
        de: "Dieser Computer ist nicht freigegeben. Bitte den Administrator um eine gezielte Freigabe.",
    },
    ErrorBusy {
        en: "The device is still working on a request. Wait a moment and refresh the status.",
        de: "Das Gerät bearbeitet noch einen Auftrag. Warte kurz und aktualisiere den Status.",
    },
    ErrorUnsupported {
        en: "The device does not offer this function. Update the device service.",
        de: "Diese Funktion ist am Gerät nicht verfügbar. Aktualisiere den Gerätedienst.",
    },
    ErrorRecoveryFailed {
        en: "The previous Wi-Fi connection could not be fully restored. Check the status and set up the Wi-Fi again.",
        de: "Die vorherige WLAN-Verbindung konnte nicht vollständig wiederhergestellt werden. Prüfe den Status und richte das WLAN erneut ein.",
    },
    ErrorDisconnected {
        en: "Connection interrupted. Connect to the device again.",
        de: "Verbindung unterbrochen. Verbinde das Gerät erneut.",
    },
    ErrorInvalidInput {
        en: "Check the Wi-Fi name and use a password of 8–63 plain characters.",
        de: "Prüfe den WLAN-Namen und verwende ein Passwort mit 8–63 einfachen Zeichen.",
    },
    ErrorConnectionRejected {
        en: "Connection failed. Check the password and try again.",
        de: "Verbindung fehlgeschlagen. Prüfe das Passwort und versuche es erneut.",
    },
    ErrorSessionExpired {
        en: "The secured session has expired (after 10 minutes or 1024 messages, a security limit). Connect to the device again or turn on automatic renewal.",
        de: "Die gesicherte Sitzung ist abgelaufen (nach 10 Minuten oder 1024 Nachrichten, eine Sicherheitsgrenze). Verbinde das Gerät erneut oder aktiviere die automatische Erneuerung.",
    },
    ErrorNoActiveNetwork {
        en: "The device is not connected to any network. Set up a Wi-Fi first or turn on the device Wi-Fi.",
        de: "Das Gerät ist mit keinem Netzwerk verbunden. Richte zuerst ein WLAN ein oder schalte das Geräte-WLAN ein.",
    },
    ErrorUnavailable {
        en: "Request failed. Connect to the device again or restart the application.",
        de: "Anfrage fehlgeschlagen. Verbinde das Gerät erneut oder starte die Anwendung neu.",
    },
    ErrorCredentialsUnavailable {
        en: "Saved passwords are not available. Enter the password manually.",
        de: "Gespeicherte Passwörter sind nicht verfügbar. Gib das Passwort manuell ein.",
    },

    // Pages and the connection status beside them
    PageWifi { en: "Wi-Fi", de: "WLAN" },
    PageSystem { en: "System", de: "System" },
    PageDevice { en: "Device", de: "Gerät" },
    ConnectionStatus { en: "Connection status", de: "Verbindungsstatus" },
    ConnectionPath { en: "Connection path", de: "Verbindungsweg" },
    LocalAccess { en: "Local access", de: "Lokaler Zugang" },
    NetworkOnDevice { en: "Network on the device", de: "Netzwerk am Gerät" },
    ReachableFromComputer { en: "Reachable from this computer", de: "Vom Computer erreichbar" },
    FunctionsOverConnection { en: "Functions over this connection", de: "Funktionen über diese Verbindung" },
    NotOfferedByDevice {
        en: "Not offered by the device. Update the device service.",
        de: "Vom Gerät nicht angeboten. Aktualisiere den Gerätedienst.",
    },
    RenewSessionAutomatically { en: "Renew session automatically", de: "Sitzung automatisch erneuern" },
    RenewSessionHint {
        en: "The secured session ends after 10 minutes or 1024 messages. If renewal is off, the connection ends then.",
        de: "Die gesicherte Sitzung endet nach 10 Minuten oder 1024 Nachrichten. Ist die Erneuerung aus, endet die Verbindung dann.",
    },
    FunctionWifi { en: "Set up Wi-Fi", de: "WLAN einrichten" },
    FunctionAccessPoint { en: "Device's own Wi-Fi", de: "Eigenes WLAN des Geräts" },
    FunctionRestartOption { en: "Restart option", de: "Neustartoption" },
    FunctionSystem { en: "System information", de: "Systeminformationen" },
    FunctionServices { en: "Show services", de: "Dienste anzeigen" },
    FunctionServiceActions { en: "Control services", de: "Dienste steuern" },
    FunctionPower { en: "Shut down or restart the device", de: "Gerät ausschalten oder neu starten" },
    ReachabilityUnverified { en: "Not checked yet", de: "Noch nicht geprüft" },
    ReachabilityAuthenticated { en: "Reachable and confirmed", de: "Erreichbar und bestätigt" },
    ReachabilityUnreachable { en: "Not reachable", de: "Nicht erreichbar" },
    NetworkJoined { en: "Connected to {0}", de: "Mit {0} verbunden" },
    NetworkOwn { en: "Own Wi-Fi: {0}", de: "Eigenes WLAN: {0}" },

    // Wi-Fi page
    JoinWifi { en: "Join Wi-Fi", de: "Mit WLAN verbinden" },
    ProvideOwnWifi { en: "Provide own Wi-Fi", de: "Eigenes WLAN bereitstellen" },
    ChooseNetwork {
        en: "Choose a network or enter its name yourself.",
        de: "Wähle ein Netzwerk oder gib den Namen selbst ein.",
    },
    RefreshNetworks { en: "Refresh networks", de: "WLANs aktualisieren" },
    WifiName { en: "Wi-Fi name", de: "WLAN-Name" },
    Password { en: "Password", de: "Passwort" },
    GetSystemPassword { en: "Get Wi-Fi password from the system", de: "WLAN-Passwort vom System holen" },
    UseCachedPassword { en: "Use cached password", de: "Zwischengespeichertes Passwort nutzen" },
    DemoPassword { en: "Demo password: {0}", de: "Demo-Passwort: {0}" },
    DemoWrongPassword {
        en: "Any other password simulates a failed attempt.",
        de: "Ein anderes Passwort simuliert einen fehlgeschlagenen Versuch.",
    },
    Wpa2Supported { en: "WPA2 Personal is supported.", de: "Unterstützt wird WPA2-Personal." },
    ShowPassword { en: "Show password", de: "Passwort anzeigen" },
    HidePassword { en: "Hide password", de: "Passwort verbergen" },

    // Restart choice
    AfterRestart { en: "After the device restarts", de: "Nach einem Neustart des Geräts" },
    RestartUseOriginal { en: "Use the original settings again", de: "Ursprüngliche Einstellungen wieder verwenden" },
    RestartUseLastAccepted { en: "Use the last successful setting", de: "Letzte erfolgreiche Einstellung verwenden" },
    RestartKeepCurrent { en: "Keep the current setting", de: "Aktuelle Einstellung behalten" },
    RestartFollowChanges { en: "Also adopt later changes", de: "Spätere Änderungen ebenfalls übernehmen" },
    RestartSetAsDefault {
        en: "Set the current setting as the new default",
        de: "Aktuelle Einstellung als neuen Standard festlegen",
    },
    RestartHintOriginal {
        en: "After a restart the device's original configuration applies. The device reverts changes if the app does not report back within a minute.",
        de: "Nach einem Neustart gilt die ursprüngliche Konfiguration des Geräts. Änderungen nimmt das Gerät zurück, wenn sich die App nicht binnen einer Minute wieder meldet.",
    },
    RestartHintFixed {
        en: "The setting fixed now returns after every restart and after every reverted change. New settings are not kept.",
        de: "Die jetzt festgelegte Einstellung kehrt nach jedem Neustart und nach jeder zurückgenommenen Änderung wieder. Neue Einstellungen werden nicht behalten.",
    },
    RestartHintKeep {
        en: "Connections you set up successfully from now on, and the own Wi-Fi, survive a restart. Settings already saved stay unchanged.",
        de: "Verbindungen, die du ab jetzt erfolgreich einrichtest, und das eigene WLAN bleiben nach einem Neustart bestehen. Bereits gespeicherte Einstellungen bleiben unverändert.",
    },
    RestartSimulationHint {
        en: "The choice is saved on the simulated device. All settings are discarded when the demo ends.",
        de: "Die Auswahl wird am simulierten Gerät gespeichert. Beim Beenden der Demo werden alle Einstellungen verworfen.",
    },

    // Device Wi-Fi
    AccessPointIntro { en: "Other devices can join this Wi-Fi.", de: "Andere Geräte können diesem WLAN beitreten." },
    AccessPointNoCredentials {
        en: "There are no credentials for this device and no secure password could be proposed. Enter name and password yourself.",
        de: "Für dieses Gerät liegen keine Zugangsdaten vor und es konnte kein sicheres Passwort vorgeschlagen werden. Gib Name und Passwort selbst ein.",
    },
    AccessPointOn { en: "Device Wi-Fi is on.", de: "Geräte-WLAN ist eingeschaltet." },
    AccessPointOff { en: "Device Wi-Fi is off.", de: "Geräte-WLAN ist ausgeschaltet." },
    AccessPointInvalid {
        en: "The name may have 1–32 characters, the password needs 8–63 characters without umlauts.",
        de: "Der Name darf 1–32 Zeichen haben, das Passwort braucht 8–63 Zeichen ohne Umlaute.",
    },
    AccessPointProposal {
        en: "Proposal: a name based on the device name and a random password of the greatest possible length. You can change both.",
        de: "Vorschlag: Name nach dem Gerätenamen und ein zufälliges Passwort in der größtmöglichen Länge. Beides kannst du ändern.",
    },
    AccessPointApply { en: "Apply new credentials", de: "Neue Zugangsdaten übernehmen" },
    AccessPointTurnOff { en: "Turn off device Wi-Fi", de: "Geräte-WLAN ausschalten" },
    AccessPointTurnOn { en: "Turn on device Wi-Fi", de: "Geräte-WLAN einschalten" },
    ScanToJoin {
        en: "Scan with the phone's camera to join.",
        de: "Mit der Kamera des Telefons scannen, um beizutreten.",
    },
    QrFailed { en: "The QR code could not be created.", de: "QR-Code konnte nicht erstellt werden." },

    // Device page
    PowerHeading { en: "Shut down or restart the device", de: "Gerät ausschalten oder neu starten" },
    PowerEndsConnection {
        en: "This ends the connection to the device.",
        de: "Die Verbindung zum Gerät wird dabei beendet.",
    },
    Restart { en: "Restart", de: "Neu starten" },
    Shutdown { en: "Shut down", de: "Herunterfahren" },
    ConfirmRestartDevice {
        en: "Really restart the device? It will be unreachable for about a minute.",
        de: "Gerät wirklich neu starten? Es ist danach für etwa eine Minute nicht erreichbar.",
    },
    ConfirmShutdownDevice {
        en: "Really shut the device down? To turn it on again, disconnect it from power or switch it on at the device.",
        de: "Gerät wirklich herunterfahren? Zum Einschalten muss es danach vom Strom getrennt oder am Gerät eingeschaltet werden.",
    },
    YesRestart { en: "Yes, restart", de: "Ja, neu starten" },
    YesShutdown { en: "Yes, shut down", de: "Ja, herunterfahren" },

    // System page
    SystemInformation { en: "System information", de: "Systeminformationen" },
    SystemNoValues {
        en: "The device has not delivered any values just now.",
        de: "Das Gerät hat gerade keine Werte geliefert.",
    },
    SystemRefreshed { en: "The values are refreshed regularly.", de: "Die Werte werden regelmäßig aktualisiert." },
    SystemNetwork { en: "Network", de: "Netzwerk" },
    SystemDevice { en: "Device", de: "Gerät" },
    SystemHostname { en: "Hostname", de: "Hostname" },
    SystemOperatingSystem { en: "Operating system", de: "Betriebssystem" },
    SystemKernel { en: "Kernel", de: "Kernel" },
    SystemIpAddresses { en: "IP addresses", de: "IP-Adressen" },
    SystemServiceVersion { en: "Device service version", de: "Version des Gerätedienstes" },
    SystemUptime { en: "Uptime", de: "Laufzeit" },
    SystemLoad { en: "Load (1 / 5 / 15 min)", de: "Last (1 / 5 / 15 Min.)" },
    SystemCpuClock { en: "CPU clock", de: "CPU-Takt" },
    SystemTemperature { en: "Temperature", de: "Temperatur" },
    SystemPowerSupply { en: "Power supply", de: "Stromversorgung" },
    SystemFreeMemory { en: "Free memory", de: "Freier Arbeitsspeicher" },
    SystemFreeStorage { en: "Free storage", de: "Freier Speicherplatz" },
    UptimeMinutes { en: "{0} min", de: "{0} Min." },
    UptimeHours { en: "{0} h {1} min", de: "{0} Std. {1} Min." },
    UptimeDays { en: "{0} d {1} h {2} min", de: "{0} T. {1} Std. {2} Min." },
    FreeOfTotal { en: "{0} of {1} ({2}%)", de: "{0} von {1} ({2} %)" },
    FrequencyWithMaximum { en: "{0} MHz (maximum {1} MHz)", de: "{0} MHz (maximal {1} MHz)" },
    ThrottleNow { en: "Now: {0}", de: "Jetzt: {0}" },
    ThrottleFine { en: "OK", de: "In Ordnung" },
    ThrottleEarlier { en: "OK, occurred since start: {0}", de: "In Ordnung, seit dem Start aufgetreten: {0}" },
    ThrottleUnderVoltage { en: "under-voltage", de: "Unterspannung" },
    ThrottleFrequencyCapped { en: "clock capped", de: "Takt begrenzt" },
    ThrottleThrottled { en: "throttled", de: "gedrosselt" },
    ThrottleTemperatureLimitNow { en: "temperature limit reached", de: "Temperaturgrenze erreicht" },
    ThrottleTemperatureLimitEarlier { en: "temperature limit", de: "Temperaturgrenze" },

    // Services
    ServiceStart { en: "Start", de: "Starten" },
    ServiceStop { en: "Stop", de: "Stoppen" },
    ConfirmStopService { en: "Really stop {0}?", de: "{0} wirklich stoppen?" },
    ConfirmRestartService { en: "Really restart {0}?", de: "{0} wirklich neu starten?" },
    ConfirmStartService { en: "Start {0}?", de: "{0} starten?" },
    ServiceVersion { en: "Version {0}", de: "Version {0}" },
    ServiceNoAddress {
        en: "The web interface cannot be opened because the device reports no IP address.",
        de: "Die Weboberfläche ist nicht abrufbar, weil das Gerät keine IP-Adresse meldet.",
    },
    ServiceWebInterface { en: "Web interface", de: "Weboberfläche" },
    ServiceOpen { en: "Open {0}", de: "{0} öffnen" },
    ScanToOpen {
        en: "Scan with the phone's camera to open the page.",
        de: "Mit der Kamera des Telefons scannen, um die Seite zu öffnen.",
    },
    ServiceNotConfirmedReachable {
        en: "Not yet confirmed as reachable from this computer.",
        de: "Vom Computer noch nicht als erreichbar bestätigt.",
    },
    ServiceRunning { en: "Running", de: "Läuft" },
    ServiceStarting { en: "Starting", de: "Startet" },
    ServiceStopped { en: "Stopped", de: "Gestoppt" },
    ServiceUnknown { en: "Unknown", de: "Unbekannt" },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(template: &str) -> Vec<usize> {
        (0..10)
            .filter(|index| template.contains(&format!("{{{index}}}")))
            .collect()
    }

    #[test]
    fn locale_names_select_a_supported_language_by_their_primary_subtag() {
        assert_eq!(Language::from_tag("de-DE"), Some(Language::German));
        assert_eq!(Language::from_tag("de_AT.UTF-8"), Some(Language::German));
        assert_eq!(Language::from_tag("EN-us"), Some(Language::English));
        assert_eq!(Language::from_tag("en"), Some(Language::English));
        assert_eq!(Language::from_tag("fr-FR"), None);
        assert_eq!(Language::from_tag(""), None);
        assert_eq!(Language::from_tag("C"), None);
    }
    #[test]
    fn every_text_exists_in_every_language_with_the_same_placeholders() {
        for text in Text::ALL {
            for language in Language::ALL {
                assert!(!language.t(*text).is_empty(), "{text:?} in {language:?}");
            }
            assert_eq!(
                placeholders(text.english()),
                placeholders(text.german()),
                "{text:?}"
            );
        }
    }
    #[test]
    fn values_fill_their_position_whatever_the_order_in_the_language() {
        assert_eq!(
            Language::German.format(Text::FreeOfTotal, &[&"1,0 GiB", &"4,0 GiB", &25]),
            "1,0 GiB von 4,0 GiB (25 %)"
        );
        assert_eq!(Language::English.format(Text::FoundCount, &[&3]), "3 found");
        assert_eq!(fill("{1} before {0}", &[&"a", &"b"]), "b before a");
    }
    #[test]
    fn the_chosen_language_is_remembered_in_the_data_directory() {
        let directory =
            std::env::temp_dir().join(format!("zorvinkel-language-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let store = LanguageStore::in_directory(&directory);
        assert_eq!(store.load(), None);
        store.save(Language::German);
        assert_eq!(store.load(), Some(Language::German));
        std::fs::write(directory.join("language"), "unknown").unwrap();
        assert_eq!(store.load(), None);
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(LanguageStore::none().load(), None);
    }
    #[test]
    fn decimals_follow_the_language() {
        assert_eq!(Language::English.decimal("47.8".into()), "47.8");
        assert_eq!(Language::German.decimal("47.8".into()), "47,8");
    }
}
