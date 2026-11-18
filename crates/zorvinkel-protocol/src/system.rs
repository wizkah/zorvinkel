//! Read-only system information of the device. It carries no secrets and needs an authorized session.
use crate::status::CodecError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The device reports its identity, load, uptime, memory, storage and addresses.
pub const CAP_SYSTEM_INFO: u32 = 64;

const MAX_VERSION: usize = 32;
const MAX_TEXT: usize = 64;
const MAX_INTERFACE: usize = 15;
const MAX_ADDRESSES: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkAddress {
    pub interface: String,
    pub address: IpAddr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInfo {
    /// Version of the device service.
    pub version: String,
    pub uptime_seconds: u64,
    /// Load averages over 1, 5 and 15 minutes in thousandths.
    pub load_milli: [u32; 3],
    pub memory_total: u64,
    pub memory_available: u64,
    pub storage_total: u64,
    pub storage_free: u64,
    /// SoC temperature in thousandths of a degree Celsius, when the device has a sensor.
    pub temperature_milli: Option<i32>,
    pub cpu_mhz: Option<u32>,
    pub cpu_max_mhz: Option<u32>,
    /// Raspberry Pi firmware flags (`vcgencmd get_throttled`), when the device has them.
    pub throttle_flags: Option<u32>,
    pub os_name: Option<String>,
    pub kernel: Option<String>,
    pub model: Option<String>,
    pub hostname: Option<String>,
    pub addresses: Vec<NetworkAddress>,
}

fn text_valid(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}
fn validate(info: &SystemInfo) -> Result<(), CodecError> {
    let version = info.version.as_bytes();
    let texts_valid = [&info.os_name, &info.kernel, &info.model, &info.hostname]
        .into_iter()
        .flatten()
        .all(|text| text_valid(text, MAX_TEXT));
    let addresses_valid = info.addresses.len() <= MAX_ADDRESSES
        && info
            .addresses
            .iter()
            .all(|entry| text_valid(&entry.interface, MAX_INTERFACE) && entry.interface.is_ascii());
    if version.is_empty()
        || version.len() > MAX_VERSION
        || !version.iter().all(|byte| (0x21..=0x7e).contains(byte))
        || info.memory_available > info.memory_total
        || info.storage_free > info.storage_total
        || !texts_valid
        || !addresses_valid
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

pub fn encode(info: &SystemInfo) -> Result<Vec<u8>, CodecError> {
    validate(info)?;
    let mut bytes = Vec::new();
    put_text(&mut bytes, &info.version);
    bytes.extend(info.uptime_seconds.to_le_bytes());
    for load in info.load_milli {
        bytes.extend(load.to_le_bytes());
    }
    for value in [
        info.memory_total,
        info.memory_available,
        info.storage_total,
        info.storage_free,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    put_optional(&mut bytes, info.temperature_milli.map(i32::to_le_bytes));
    put_optional(&mut bytes, info.cpu_mhz.map(u32::to_le_bytes));
    put_optional(&mut bytes, info.cpu_max_mhz.map(u32::to_le_bytes));
    put_optional(&mut bytes, info.throttle_flags.map(u32::to_le_bytes));
    for text in [&info.os_name, &info.kernel, &info.model, &info.hostname] {
        put_text(&mut bytes, text.as_deref().unwrap_or(""));
    }
    bytes.push(info.addresses.len() as u8);
    for entry in &info.addresses {
        put_text(&mut bytes, &entry.interface);
        match entry.address {
            IpAddr::V4(address) => {
                bytes.push(4);
                bytes.extend(address.octets());
            }
            IpAddr::V6(address) => {
                bytes.push(6);
                bytes.extend(address.octets());
            }
        }
    }
    Ok(bytes)
}
pub(crate) fn put_text(bytes: &mut Vec<u8>, text: &str) {
    bytes.push(text.len() as u8);
    bytes.extend(text.as_bytes());
}
fn put_optional<const N: usize>(bytes: &mut Vec<u8>, value: Option<[u8; N]>) {
    match value {
        Some(value) => {
            bytes.push(1);
            bytes.extend(value);
        }
        None => bytes.push(0),
    }
}

pub fn decode(payload: &[u8]) -> Result<SystemInfo, CodecError> {
    let mut reader = Reader(payload);
    let version = reader.text()?;
    let uptime_seconds = reader.u64()?;
    let load_milli = [reader.u32()?, reader.u32()?, reader.u32()?];
    let memory_total = reader.u64()?;
    let memory_available = reader.u64()?;
    let storage_total = reader.u64()?;
    let storage_free = reader.u64()?;
    let temperature_milli = reader.optional()?.map(i32::from_le_bytes);
    let cpu_mhz = reader.optional()?.map(u32::from_le_bytes);
    let cpu_max_mhz = reader.optional()?.map(u32::from_le_bytes);
    let throttle_flags = reader.optional()?.map(u32::from_le_bytes);
    let mut optional_text = || reader.text().map(|text| (!text.is_empty()).then_some(text));
    let (os_name, kernel, model, hostname) = (
        optional_text()?,
        optional_text()?,
        optional_text()?,
        optional_text()?,
    );
    let count = usize::from(reader.take(1)?[0]);
    if count > MAX_ADDRESSES {
        return Err(CodecError::Malformed);
    }
    let mut addresses = Vec::with_capacity(count);
    for _ in 0..count {
        let interface = reader.text()?;
        let address = match reader.take(1)?[0] {
            4 => IpAddr::V4(Ipv4Addr::from(reader.array::<4>()?)),
            6 => IpAddr::V6(Ipv6Addr::from(reader.array::<16>()?)),
            _ => return Err(CodecError::Malformed),
        };
        addresses.push(NetworkAddress { interface, address });
    }
    if !reader.0.is_empty() {
        return Err(CodecError::Malformed);
    }
    let info = SystemInfo {
        version,
        uptime_seconds,
        load_milli,
        memory_total,
        memory_available,
        storage_total,
        storage_free,
        temperature_milli,
        cpu_mhz,
        cpu_max_mhz,
        throttle_flags,
        os_name,
        kernel,
        model,
        hostname,
        addresses,
    };
    validate(&info)?;
    Ok(info)
}

pub(crate) struct Reader<'a>(pub(crate) &'a [u8]);
impl<'a> Reader<'a> {
    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8], CodecError> {
        if self.0.len() < count {
            return Err(CodecError::Malformed);
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(head)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        self.take(N)?.try_into().map_err(|_| CodecError::Malformed)
    }
    pub(crate) fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub(crate) fn text(&mut self) -> Result<String, CodecError> {
        let length = usize::from(self.take(1)?[0]);
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| CodecError::Malformed)
    }
    pub(crate) fn optional<const N: usize>(&mut self) -> Result<Option<[u8; N]>, CodecError> {
        match self.take(1)?[0] {
            0 => Ok(None),
            1 => self.array().map(Some),
            _ => Err(CodecError::Malformed),
        }
    }
}
