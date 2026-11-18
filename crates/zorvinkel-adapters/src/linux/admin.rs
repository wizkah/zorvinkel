//! Root-only local commands; peer identity comes from the kernel, never from request data.
use super::runtime::RuntimeState;
use crate::ble::BleResult;
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    time::timeout,
};
use zorvinkel_application::{
    registration::{SessionControl, authorize_by_local_admin, grant_by_local_admin},
    state::StateStore,
};
use zorvinkel_core::registry::ClientKey;

pub fn bind(path: &Path) -> BleResult<UnixListener> {
    // The parent is created and owned by systemd. Never unlink an existing listener.
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}
pub async fn serve(listener: UnixListener, state: Arc<Mutex<RuntimeState>>) -> BleResult<()> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        if stream.peer_cred()?.uid() != 0 {
            continue;
        }
        let result = timeout(Duration::from_secs(1), async {
            let mut bytes = [0u8; 100];
            let mut length = 0;
            loop {
                if length == bytes.len() {
                    return Err("request too long".into());
                }
                let read = stream.read(&mut bytes[length..length + 1]).await?;
                if read == 0 {
                    return Err("incomplete request".into());
                }
                length += read;
                if bytes[length - 1] == b'\n' {
                    break;
                }
            }
            let request = std::str::from_utf8(&bytes[..length])?;
            let response = match execute(
                &mut *state.lock().map_err(|_| "runtime lock failed")?,
                request,
            ) {
                Ok(response) => response,
                Err(_) => "ERROR\n".into(),
            };
            stream.write_all(response.as_bytes()).await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            eprintln!("Local administration request failed");
        }
    }
}
pub fn execute(state: &mut RuntimeState, request: &str) -> BleResult<String> {
    let parts: Vec<_> = request.split_whitespace().collect();
    if parts == ["public-key"] {
        return Ok(format!("{}\n", encode_key(state.public_key())));
    }
    if parts.len() == 2 && parts[0] == "wifi-persistence" {
        match parts[1] {
            "on" | "off" => super::wifi::set_persistence(&state.root, parts[1] == "on")?,
            "status" => {
                return Ok(if super::wifi::persistence_enabled(&state.root) {
                    "on\n"
                } else {
                    "off\n"
                }
                .into());
            }
            _ => return Err("invalid command".into()),
        }
        return Ok("OK\n".into());
    }
    if parts.len() != 2 {
        return Err("invalid command".into());
    }
    let target = decode_key(parts[1])?;
    match parts[0] {
        "grant" => {
            let now = state.registration_seconds();
            grant_by_local_admin(
                &mut state.store,
                target,
                now,
                now.checked_add(300).ok_or("clock limit")?,
            )
            .map_err(|_| "grant rejected")?;
        }
        "authorize" => {
            authorize_by_local_admin(&mut state.store, target)
                .map_err(|_| "authorization rejected")?;
        }
        "revoke" => {
            state
                .store
                .update(|state| Ok(state.registry.revoke(&target)?))
                .map_err(|_| "revocation rejected")?;
            state
                .sessions
                .terminate(&target)
                .map_err(|_| "session termination failed")?;
        }
        _ => return Err("invalid command".into()),
    }
    Ok("OK\n".into())
}
pub fn encode_key(key: &[u8]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn decode_key(key: &str) -> BleResult<ClientKey> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("expected 32-byte public key in hexadecimal".into());
    }
    let bytes = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&key[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()?;
    ClientKey::new(bytes).map_err(|_| "invalid public key".into())
}
