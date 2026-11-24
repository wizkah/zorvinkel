//! The "System" page: read-only values the device reports about itself.
use super::{
    i18n::{Language, Text},
    session::Session,
    theme,
};
use eframe::egui::{self, RichText};

impl Session {
    pub fn system(&mut self, ui: &mut egui::Ui, lang: Language) {
        theme::section_title(ui, lang.t(Text::SystemInformation));
        let Some(info) = &self.snapshot.system_info else {
            ui.label(lang.t(Text::SystemNoValues));
            return;
        };
        ui.label(lang.t(Text::SystemRefreshed));
        ui.add_space(theme::space::S);
        egui::Grid::new("system_info")
            .num_columns(2)
            .spacing([theme::space::XL, theme::space::S])
            .show(ui, |ui| {
                let network = self
                    .snapshot
                    .status
                    .as_ref()
                    .map(|status| super::device_tab::network_label(&status.network, lang));
                let mut row = |label: Text, value: String| {
                    ui.label(RichText::new(lang.t(label)).color(theme::TEXT_MUTED));
                    ui.strong(value);
                    ui.end_row();
                };
                let mut optional = |label: Text, value: &Option<String>| {
                    if let Some(value) = value {
                        row(label, value.clone());
                    }
                };
                optional(Text::SystemNetwork, &network);
                optional(Text::SystemDevice, &info.model);
                optional(Text::SystemHostname, &info.hostname);
                optional(Text::SystemOperatingSystem, &info.os_name);
                optional(Text::SystemKernel, &info.kernel);
                if !info.addresses.is_empty() {
                    let addresses = info
                        .addresses
                        .iter()
                        .map(|entry| format!("{} ({})", entry.address, entry.interface))
                        .collect::<Vec<_>>()
                        .join("\n");
                    row(Text::SystemIpAddresses, addresses);
                }
                row(Text::SystemServiceVersion, info.version.clone());
                row(Text::SystemUptime, format_uptime(info.uptime_seconds, lang));
                row(Text::SystemLoad, format_load(info.load_milli, lang));
                if let Some(mhz) = info.cpu_mhz {
                    row(
                        Text::SystemCpuClock,
                        format_frequency(mhz, info.cpu_max_mhz, lang),
                    );
                }
                if let Some(temperature) = info.temperature_milli {
                    row(
                        Text::SystemTemperature,
                        format_temperature(temperature, lang),
                    );
                }
                if let Some(flags) = info.throttle_flags {
                    row(Text::SystemPowerSupply, format_throttle(flags, lang));
                }
                row(
                    Text::SystemFreeMemory,
                    format_free(info.memory_available, info.memory_total, lang),
                );
                row(
                    Text::SystemFreeStorage,
                    format_free(info.storage_free, info.storage_total, lang),
                );
            });
    }
}

fn format_uptime(seconds: u64, lang: Language) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => lang.format(Text::UptimeMinutes, &[&minutes]),
        (0, _) => lang.format(Text::UptimeHours, &[&hours, &minutes]),
        _ => lang.format(Text::UptimeDays, &[&days, &hours, &minutes]),
    }
}
fn format_load(load_milli: [u32; 3], lang: Language) -> String {
    let [one, five, fifteen] = load_milli.map(|load| format!("{:.2}", f64::from(load) / 1000.0));
    lang.decimal(format!("{one} / {five} / {fifteen}"))
}
fn format_bytes(bytes: u64, lang: Language) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        lang.decimal(format!("{value:.1} {}", UNITS[unit]))
    }
}
fn format_free(free: u64, total: u64, lang: Language) -> String {
    let percent = (free * 100).checked_div(total).unwrap_or(0);
    lang.format(
        Text::FreeOfTotal,
        &[
            &format_bytes(free, lang),
            &format_bytes(total, lang),
            &percent,
        ],
    )
}
fn format_frequency(mhz: u32, max_mhz: Option<u32>, lang: Language) -> String {
    match max_mhz {
        Some(max) => lang.format(Text::FrequencyWithMaximum, &[&mhz, &max]),
        None => format!("{mhz} MHz"),
    }
}
/// Firmware flags: bits 0–3 describe the present, bits 16–19 what happened since boot.
fn format_throttle(flags: u32, lang: Language) -> String {
    const PROBLEMS: [(u32, Text, Text); 4] = [
        (0, Text::ThrottleUnderVoltage, Text::ThrottleUnderVoltage),
        (
            1,
            Text::ThrottleFrequencyCapped,
            Text::ThrottleFrequencyCapped,
        ),
        (2, Text::ThrottleThrottled, Text::ThrottleThrottled),
        (
            3,
            Text::ThrottleTemperatureLimitNow,
            Text::ThrottleTemperatureLimitEarlier,
        ),
    ];
    let now: Vec<&str> = PROBLEMS
        .iter()
        .filter(|(bit, ..)| flags >> bit & 1 == 1)
        .map(|(_, text, _)| lang.t(*text))
        .collect();
    if !now.is_empty() {
        return lang.format(Text::ThrottleNow, &[&now.join(", ")]);
    }
    let earlier: Vec<&str> = PROBLEMS
        .iter()
        .filter(|(bit, ..)| flags >> (bit + 16) & 1 == 1)
        .map(|(.., text)| lang.t(*text))
        .collect();
    if earlier.is_empty() {
        lang.t(Text::ThrottleFine).into()
    } else {
        lang.format(Text::ThrottleEarlier, &[&earlier.join(", ")])
    }
}
fn format_temperature(milli: i32, lang: Language) -> String {
    lang.decimal(format!("{:.1} °C", f64::from(milli) / 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn values_are_shown_in_readable_german_units() {
        let de = Language::German;
        assert_eq!(format_uptime(59, de), "0 Min.");
        assert_eq!(format_uptime(3 * 3600 + 120, de), "3 Std. 2 Min.");
        assert_eq!(format_uptime(93_784, de), "1 T. 2 Std. 3 Min.");
        assert_eq!(format_load([50, 1250, 0], de), "0,05 / 1,25 / 0,00");
        assert_eq!(format_bytes(512, de), "512 B");
        assert_eq!(format_bytes(310 * 1024 * 1024, de), "310,0 MiB");
        assert_eq!(format_free(GIB, 4 * GIB, de), "1,0 GiB von 4,0 GiB (25 %)");
        assert_eq!(format_free(0, 0, de), "0 B von 0 B (0 %)");
        assert_eq!(format_temperature(47_800, de), "47,8 °C");
        assert_eq!(
            format_frequency(700, Some(1000), de),
            "700 MHz (maximal 1000 MHz)"
        );
        assert_eq!(format_throttle(0, de), "In Ordnung");
        assert_eq!(format_throttle(0x5, de), "Jetzt: Unterspannung, gedrosselt");
        assert_eq!(
            format_throttle(0x50000, de),
            "In Ordnung, seit dem Start aufgetreten: Unterspannung, gedrosselt"
        );
    }
    #[test]
    fn values_are_shown_in_readable_english_units() {
        let en = Language::English;
        assert_eq!(format_uptime(59, en), "0 min");
        assert_eq!(format_uptime(3 * 3600 + 120, en), "3 h 2 min");
        assert_eq!(format_uptime(93_784, en), "1 d 2 h 3 min");
        assert_eq!(format_load([50, 1250, 0], en), "0.05 / 1.25 / 0.00");
        assert_eq!(format_bytes(310 * 1024 * 1024, en), "310.0 MiB");
        assert_eq!(format_free(GIB, 4 * GIB, en), "1.0 GiB of 4.0 GiB (25%)");
        assert_eq!(format_temperature(47_800, en), "47.8 °C");
        assert_eq!(
            format_frequency(700, Some(1000), en),
            "700 MHz (maximum 1000 MHz)"
        );
        assert_eq!(format_throttle(0, en), "OK");
        assert_eq!(format_throttle(0x5, en), "Now: under-voltage, throttled");
        assert_eq!(
            format_throttle(0x80008, en),
            "Now: temperature limit reached"
        );
        assert_eq!(
            format_throttle(0x80000, en),
            "OK, occurred since start: temperature limit"
        );
    }
}
