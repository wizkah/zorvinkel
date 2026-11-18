//! Constants of the authenticated IP endpoint (ADR-020, ADR-023).

/// TCP port of the endpoint. It carries the same secured channel as Bluetooth.
pub const PORT: u16 = 47_470;

/// DNS-SD service type under which the device announces the endpoint.
pub const SERVICE_TYPE: &str = "_zorvinkel._tcp.local.";

/// The device runs the endpoint, so a failed connection says something about the network.
pub const CAP_IP_REACHABILITY: u32 = 128;

/// The device issues and redeems one-time correlation proofs for a Bluetooth-to-IP switch.
pub const CAP_CORRELATION: u32 = 1024;

/// Length of a correlation proof: 256 random bits.
pub const CORRELATION_LEN: usize = 32;

/// One-time proof, issued inside one secured session and redeemed inside another.
#[derive(Clone, PartialEq, Eq)]
pub struct CorrelationToken(pub [u8; CORRELATION_LEN]);
impl std::fmt::Debug for CorrelationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CorrelationToken(..)")
    }
}
pub fn read_correlation(payload: &[u8]) -> Result<CorrelationToken, crate::status::CodecError> {
    payload
        .try_into()
        .map(CorrelationToken)
        .map_err(|_| crate::status::CodecError::Malformed)
}
