//! ChatGPT subscription credentials and OAuth sign-in.
//!
//! Donor provenance: `src/llm/auth.ts` (`SubscriptionCredential`,
//! `parseSubscription`, `record`, `loginSubscription`,
//! `refreshSubscription`, `cancelled`, `checkAbort`). Same PKCE flow
//! (S256 challenge, loopback callback, token exchange), same authorize
//! parameters, same fixed errors.
//!
//! The donor serves the loopback callback with `Bun.serve` and opens the
//! browser concurrently; this synchronous port serves one callback with
//! `std::net::TcpListener` after the injected browser opener returns. The
//! opener must therefore return promptly (launching a browser process or
//! driving the callback from another thread); the listener accepts within
//! the callback deadline after it returns. Randomness comes from the OS
//! (`/dev/urandom` with a time-seeded fallback); SHA-256 is implemented
//! locally so PKCE needs no dependency.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::settings::json::{parse_json, Json};

use super::errors::LlmError;
use super::request::{fetch_llm_response, CancelToken, FetchInit, FetchMethod, LlmFetch, TimeoutFetch};

/// OAuth client id (donor `CLIENT_ID`).
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Default token endpoint.
pub const DEFAULT_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Default authorize endpoint.
pub const DEFAULT_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
/// Default loopback callback port.
pub const DEFAULT_CALLBACK_PORT: u16 = 1455;
/// Default callback wait (donor `callbackTimeoutMs`).
pub const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_millis(180_000);
/// Default token fetch deadline (donor `fetchTimeoutMs`).
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_millis(30_000);

/// A stored subscription credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionCredential {
    /// OAuth access token.
    pub access_token: String,
    /// OAuth refresh token.
    pub refresh_token: String,
    /// Token type (`Bearer`).
    pub token_type: String,
    /// Expiry as Unix milliseconds.
    pub expires_at: i64,
    /// Granted scopes.
    pub scopes: Vec<String>,
}

/// Browser launcher: start a sign-in for `url` and return promptly; the
/// loopback listener accepts after this returns, so a test driver must
/// complete the callback from another thread. Observes `cancel` while
/// waiting.
pub type BrowserOpener = Arc<dyn Fn(&str, &CancelToken) -> Result<(), LlmError> + Send + Sync>;

/// Subscription auth configuration (donor `SubscriptionAuthOptions`).
#[derive(Clone, Default)]
pub struct SubscriptionAuthOptions {
    /// Authorize endpoint override.
    pub authorize_url: Option<String>,
    /// Token endpoint override.
    pub token_url: Option<String>,
    /// Loopback callback port (`0` binds an ephemeral port for tests).
    pub callback_port: Option<u16>,
    /// Callback wait deadline.
    pub callback_timeout: Option<Duration>,
    /// Token fetch deadline.
    pub fetch_timeout: Option<Duration>,
    /// Browser launcher override.
    pub open_browser: Option<BrowserOpener>,
    /// Token fetch override (no implicit network client exists).
    pub fetch: Option<Arc<dyn LlmFetch>>,
}

/// Require a JSON object (donor `record`).
pub fn record(value: &Json) -> Result<&Vec<(String, Json)>, LlmError> {
    match value {
        Json::Object(members) => Ok(members),
        _ => Err(LlmError::settings("Invalid LLM settings data.")),
    }
}

fn member<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

fn nonempty(value: &Json) -> Result<String, LlmError> {
    match value {
        Json::String(text) if !text.trim().is_empty() => Ok(text.clone()),
        _ => Err(LlmError::settings("Invalid subscription credential.")),
    }
}

/// Parse a stored subscription credential (donor `parseSubscription`).
pub fn parse_subscription(value: &Json) -> Result<SubscriptionCredential, LlmError> {
    let item = record(value)?;
    let expires = member(item, "expiresAt");
    let scopes = member(item, "scopes");
    let (Some(Json::Number(expires)), Some(Json::Array(scopes))) = (expires, scopes) else {
        return Err(LlmError::settings("Invalid subscription credential."));
    };
    if !expires.is_finite() {
        return Err(LlmError::settings("Invalid subscription credential."));
    }
    let mut parsed = Vec::with_capacity(scopes.len());
    for scope in scopes {
        parsed.push(nonempty(scope)?);
    }
    Ok(SubscriptionCredential {
        access_token: nonempty(member(item, "accessToken").unwrap_or(&Json::Null))?,
        refresh_token: nonempty(member(item, "refreshToken").unwrap_or(&Json::Null))?,
        token_type: nonempty(member(item, "tokenType").unwrap_or(&Json::Null))?,
        expires_at: *expires as i64,
        scopes: parsed,
    })
}

/// Cancelled sign-in error (donor `cancelled`).
pub fn cancelled() -> LlmError {
    LlmError::settings("Subscription sign-in cancelled.")
}

/// Throw when sign-in was cancelled (donor `checkAbort`).
pub fn check_abort(cancel: &CancelToken) -> Result<(), LlmError> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    Ok(())
}

/// Current Unix time in milliseconds (saturating).
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Fill `out` with OS randomness, falling back to a time-seeded stream.
pub(crate) fn random_bytes(out: &mut [u8]) {
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| {
            let mut filled = 0;
            while filled < out.len() {
                let count = file.read(&mut out[filled..])?;
                if count == 0 {
                    break;
                }
                filled += count;
            }
            Ok(())
        })
        .is_ok()
        && out.iter().any(|byte| *byte != 0)
    {
        return;
    }
    let mut state = (now_ms() as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(std::process::id() as u64 | 1);
    for byte in out.iter_mut() {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
    }
}

/// A random v4 UUID (donor `crypto.randomUUID`).
pub(crate) fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    random_bytes(&mut bytes);
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url-encode bytes without padding.
pub(crate) fn base64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut word = 0u32;
        for byte in chunk {
            word = (word << 8) | u32::from(*byte);
        }
        word <<= 8 * (3 - chunk.len());
        for index in 0..chunk.len() + 1 {
            out.push(BASE64URL[((word >> (18 - index * 6)) & 63) as usize] as char);
        }
    }
    out
}

/// Base64url-decode text, tolerating missing padding.
pub(crate) fn base64url_decode(text: &str) -> Result<Vec<u8>, ()> {
    let mut values = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        let value = BASE64URL.iter().position(|candidate| *candidate == byte).ok_or(())?;
        values.push(value as u32);
    }
    if values.len() % 4 == 1 {
        return Err(());
    }
    let mut out = Vec::with_capacity(values.len() / 4 * 3);
    for chunk in values.chunks(4) {
        let mut word = 0u32;
        for value in chunk {
            word = (word << 6) | value;
        }
        word <<= 6 * (4 - chunk.len());
        for index in 0..chunk.len() - 1 {
            out.push((word >> (16 - index * 8)) as u8);
        }
    }
    Ok(out)
}

/// SHA-256 digest (PKCE `S256`; local so OAuth needs no dependency).
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    let bits = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for chunk in message.chunks(64) {
        let block: &[u8; 64] = chunk.try_into().expect("sha256 input is padded to blocks");
        let mut schedule = [0u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let at = index * 4;
            *word = u32::from_be_bytes([block[at], block[at + 1], block[at + 2], block[at + 3]]);
        }
        for index in 16..64 {
            let x = schedule[index - 15];
            let y = schedule[index - 2];
            let small_x = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
            let small_y = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(small_x)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(small_y);
        }
        let mut working = state;
        for (index, word) in schedule.iter().enumerate() {
            let [a, b, c, d, e, f, g, h] = working;
            let big_e = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let sum1 = h
                .wrapping_add(big_e)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(*word);
            let big_a = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let sum0 = big_a.wrapping_add(majority);
            working = [sum1.wrapping_add(sum0), a, b, c, d.wrapping_add(sum1), e, f, g];
        }
        for (slot, word) in state.iter_mut().zip(working) {
            *slot = slot.wrapping_add(word);
        }
    }
    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// `application/x-www-form-urlencoded` encoding (donor `URLSearchParams`).
pub(crate) fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Decode one query/form component (`+` becomes a space).
pub(crate) fn form_decode(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let raw = text.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'+' {
            bytes.push(b' ');
            index += 1;
        } else if raw[index] == b'%' && index + 2 < raw.len() + 1 {
            let hex = &text[index + 1..(index + 3).min(text.len())];
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    index += 3;
                    continue;
                }
            }
            bytes.push(b'%');
            index += 1;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Parse a query string into decoded pairs.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.find('=') {
            Some(index) => (form_decode(&pair[..index]), form_decode(&pair[index + 1..])),
            None => (form_decode(pair), String::new()),
        })
        .collect()
}

fn token_failed() -> LlmError {
    LlmError::settings("Subscription token request failed. Try signing in again.")
}

fn token_request_inner(
    options: &SubscriptionAuthOptions,
    form: &[(String, String)],
    cancel: &CancelToken,
    previous_scopes: &[String],
) -> Result<SubscriptionCredential, LlmError> {
    let Some(fetcher) = options.fetch.clone() else {
        return Err(token_failed());
    };
    let timeout = TimeoutFetch::new(fetcher, options.fetch_timeout.unwrap_or(DEFAULT_FETCH_TIMEOUT));
    let body = form
        .iter()
        .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let response = fetch_llm_response(
        options.token_url.as_deref().unwrap_or(DEFAULT_TOKEN_URL),
        &FetchInit {
            method: FetchMethod::Post,
            headers: vec![(
                "content-type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            )],
            body: Some(body),
        },
        &timeout,
        cancel,
    )?;
    if !response.ok() {
        return Err(token_failed());
    }
    let text = std::str::from_utf8(&response.body).map_err(|_| token_failed())?;
    let value = parse_json(text).map_err(|_| token_failed())?;
    let item = record(&value).map_err(|_| token_failed())?;
    let expires = match member(item, "expires_in") {
        Some(Json::Number(expires)) if expires.is_finite() && *expires > 0.0 => *expires,
        _ => return Err(token_failed()),
    };
    let expires_at = now_ms() as f64 + expires * 1000.0;
    if !expires_at.is_finite() {
        return Err(token_failed());
    }
    let token_type = match member(item, "token_type").unwrap_or(&Json::String("Bearer".to_string())) {
        Json::String(token_type) if !token_type.trim().is_empty() => token_type.clone(),
        _ => return Err(token_failed()),
    };
    let access_token = nonempty(member(item, "access_token").unwrap_or(&Json::Null)).map_err(|_| token_failed())?;
    let refresh_token = nonempty(member(item, "refresh_token").unwrap_or(&Json::Null)).map_err(|_| token_failed())?;
    let scopes = match member(item, "scope") {
        Some(Json::String(scope)) => scope.split_whitespace().map(str::to_string).collect(),
        _ => previous_scopes.to_vec(),
    };
    Ok(SubscriptionCredential {
        access_token,
        refresh_token,
        token_type,
        expires_at: expires_at as i64,
        scopes,
    })
}

/// Exchange one token form (donor `tokenRequest`).
///
/// The donor's catch collapses every failure into the retry message (or
/// cancellation); this port does the same explicitly.
fn token_request(
    options: &SubscriptionAuthOptions,
    form: &[(String, String)],
    cancel: &CancelToken,
    previous_scopes: &[String],
) -> Result<SubscriptionCredential, LlmError> {
    check_abort(cancel)?;
    let result = token_request_inner(options, form, cancel, previous_scopes);
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    result.map_err(|error| if error == cancelled() { error } else { token_failed() })
}

/// Write one minimal HTTP response and flush.
fn respond(stream: &mut std::net::TcpStream, status: u16, reason: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

/// Read one HTTP request head (bounded at 64 KiB).
fn read_request(stream: &mut std::net::TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while head.len() < 65_536 {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// Run the PKCE loopback sign-in (donor `loginSubscription`).
pub fn login_subscription(
    options: &SubscriptionAuthOptions,
    cancel: &CancelToken,
) -> Result<SubscriptionCredential, LlmError> {
    check_abort(cancel)?;
    let mut random = [0u8; 32];
    random_bytes(&mut random);
    let state = base64url_encode(&random);
    random_bytes(&mut random);
    let verifier = base64url_encode(&random);
    let listener = TcpListener::bind(("127.0.0.1", options.callback_port.unwrap_or(DEFAULT_CALLBACK_PORT)))
        .map_err(|_| {
            LlmError::settings(
                "Could not open the local sign-in callback. Close other sign-in windows or applications using port 1455, then retry.",
            )
        })?;
    let port = listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|_| LlmError::settings("Subscription sign-in failed. Try again."))?;
    let redirect = format!("http://localhost:{port}/auth/callback");
    let challenge = base64url_encode(&sha256(verifier.as_bytes()));
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect.as_str()),
        ("scope", "openid profile email offline_access"),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "pi"),
    ];
    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let base = options.authorize_url.as_deref().unwrap_or(DEFAULT_AUTHORIZE_URL);
    let separator = if base.contains('?') { "&" } else { "?" };
    let authorize = format!("{base}{separator}{query}");
    match options.open_browser.clone() {
        Some(opener) => opener(&authorize, cancel)?,
        None => {
            check_abort(cancel)?;
            return Err(LlmError::settings("Sign-in browser is unavailable."));
        }
    }
    check_abort(cancel)?;
    let deadline = Instant::now() + options.callback_timeout.unwrap_or(DEFAULT_CALLBACK_TIMEOUT);
    listener
        .set_nonblocking(true)
        .map_err(|_| LlmError::settings("Subscription sign-in failed. Try again."))?;
    let (stream, code) = loop {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if Instant::now() >= deadline {
            return Err(LlmError::settings("Subscription sign-in timed out. Try again."));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let mut stream = stream;
                stream
                    .set_read_timeout(Some(Duration::from_millis(5_000)))
                    .map_err(|_| LlmError::settings("Subscription sign-in failed. Try again."))?;
                let head = read_request(&mut stream)
                    .map_err(|_| LlmError::settings("Subscription sign-in failed. Try again."))?;
                let request_line = head.lines().next().unwrap_or("");
                let mut parts = request_line.split_whitespace();
                let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let (path, query) = match target.find('?') {
                    Some(index) => (&target[..index], &target[index + 1..]),
                    None => (target, ""),
                };
                if path != "/auth/callback" {
                    let _ = respond(&mut stream, 404, "Not Found", "Not found");
                    continue;
                }
                if method != "GET" {
                    let _ = respond(&mut stream, 405, "Method Not Allowed", "Method not allowed");
                    continue;
                }
                let pairs = parse_query(query);
                let state_matches = pairs.iter().any(|(key, value)| key == "state" && value == &state);
                if !state_matches {
                    let _ = respond(&mut stream, 400, "Bad Request", "Sign-in failed. State mismatch.");
                    return Err(LlmError::settings("Subscription sign-in state mismatch."));
                }
                let failed = pairs.iter().any(|(key, _)| key == "error");
                let code = pairs
                    .iter()
                    .find(|(key, _)| key == "code")
                    .map(|(_, value)| value.clone());
                match (failed, code) {
                    (false, Some(code)) if !code.is_empty() => {
                        respond(
                            &mut stream,
                            200,
                            "OK",
                            "Authorization received. You can return to the game.",
                        )
                        .map_err(|_| LlmError::settings("Subscription sign-in failed. Try again."))?;
                        break (stream, code);
                    }
                    _ => {
                        let _ = respond(&mut stream, 400, "Bad Request", "Sign-in was not authorized.");
                        return Err(LlmError::settings("Subscription sign-in was not authorized."));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return Err(LlmError::settings("Subscription sign-in failed. Try again.")),
        }
    };
    drop(stream);
    drop(listener);
    check_abort(cancel)?;
    token_request(
        options,
        &[
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("client_id".to_string(), CLIENT_ID.to_string()),
            ("redirect_uri".to_string(), redirect),
            ("code".to_string(), code),
            ("code_verifier".to_string(), verifier),
        ],
        cancel,
        &[],
    )
}

/// Refresh one subscription credential (donor `refreshSubscription`).
pub fn refresh_subscription(
    options: &SubscriptionAuthOptions,
    credential: &SubscriptionCredential,
    cancel: &CancelToken,
) -> Result<SubscriptionCredential, LlmError> {
    token_request(
        options,
        &[
            ("grant_type".to_string(), "refresh_token".to_string()),
            ("client_id".to_string(), CLIENT_ID.to_string()),
            ("refresh_token".to_string(), credential.refresh_token.clone()),
        ],
        cancel,
        &credential.scopes,
    )
}

#[cfg(test)]
mod tests {
    use super::super::request::LlmResponse;
    use super::*;

    fn tokens_body() -> String {
        "{\"access_token\":\"test-access\",\"refresh_token\":\"test-refresh\",\"token_type\":\"Bearer\",\"expires_in\":3600,\"scope\":\"openid profile\",\"id_token\":\"must-not-save\"}".to_string()
    }

    fn token_fetcher() -> Arc<dyn LlmFetch> {
        Arc::new(|_: &str, init: &FetchInit| {
            assert_eq!(init.method, FetchMethod::Post);
            assert_eq!(init.header("content-type"), Some("application/x-www-form-urlencoded"));
            Ok(LlmResponse {
                status: 200,
                content_type: Some("application/json".to_string()),
                body: tokens_body().into_bytes(),
            })
        })
    }

    fn callback_url(authorize: &str) -> String {
        let redirect = authorize
            .split("redirect_uri=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .map(form_decode)
            .expect("redirect");
        let state = authorize
            .split("state=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .expect("state")
            .to_string();
        format!("{redirect}?state={state}&code=test-code")
    }

    type BrowserHandles = Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<(u16, String)>>>>;

    fn get(url: &str) -> (u16, String) {
        let without_scheme = url.strip_prefix("http://").expect("http");
        let (host, path) = match without_scheme.find('/') {
            Some(index) => (&without_scheme[..index], &without_scheme[index..]),
            None => (without_scheme, "/"),
        };
        let host = host.replace("localhost", "127.0.0.1");
        let mut stream = std::net::TcpStream::connect(host).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").expect("write");
        let mut body = Vec::new();
        stream.read_to_end(&mut body).expect("read");
        let text = String::from_utf8_lossy(&body).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(0);
        let payload = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, payload)
    }

    #[test]
    fn sha256_matches_reference_vectors() {
        assert_eq!(
            base64url_encode(&sha256(b"")),
            "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU"
        );
        assert_eq!(
            base64url_encode(&sha256(b"abc")),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
    }

    #[test]
    fn base64url_round_trips_without_padding() {
        for bytes in [vec![], vec![0xfbu8], vec![1, 2, 3, 4, 5], (0..64u8).collect::<Vec<_>>()] {
            let encoded = base64url_encode(&bytes);
            assert!(!encoded.contains('='));
            assert_eq!(base64url_decode(&encoded).unwrap(), bytes);
        }
        assert!(base64url_decode("***").is_err());
    }

    #[test]
    fn form_encoding_matches_url_search_params() {
        assert_eq!(form_encode("a b+c/d?e=f&g"), "a+b%2Bc%2Fd%3Fe%3Df%26g");
        assert_eq!(form_decode("a+b%2Bc"), "a b+c");
        assert_eq!(random_uuid().len(), 36);
        assert_ne!(random_uuid(), random_uuid());
    }

    #[test]
    fn parse_subscription_validates_shape() {
        let value = parse_json(
            "{\"accessToken\":\"a\",\"refreshToken\":\"r\",\"tokenType\":\"Bearer\",\"expiresAt\":42,\"scopes\":[\"openid\"]}",
        )
        .unwrap();
        let credential = parse_subscription(&value).unwrap();
        assert_eq!(credential.expires_at, 42);
        assert_eq!(credential.scopes, vec!["openid".to_string()]);
        for body in [
            "{\"accessToken\":\"\",\"refreshToken\":\"r\",\"tokenType\":\"Bearer\",\"expiresAt\":42,\"scopes\":[]}",
            "{\"accessToken\":\"a\",\"refreshToken\":\"r\",\"tokenType\":\"Bearer\",\"expiresAt\":\"soon\",\"scopes\":[]}",
            "{\"accessToken\":\"a\",\"refreshToken\":\"r\",\"tokenType\":\"Bearer\",\"expiresAt\":42,\"scopes\":{}}",
            "[1,2]",
        ] {
            assert!(parse_subscription(&parse_json(body).unwrap()).is_err(), "{body}");
        }
    }

    #[test]
    fn refresh_posts_the_rotating_form_and_rejects_bad_replacements() {
        let options = SubscriptionAuthOptions {
            fetch: Some(Arc::new(|url: &str, init: &FetchInit| {
                assert_eq!(url, DEFAULT_TOKEN_URL);
                assert_eq!(
                    init.body.as_deref(),
                    Some("grant_type=refresh_token&client_id=app_EMoamEEZ73f0CkXaXp7hrann&refresh_token=old-refresh")
                );
                Ok(LlmResponse {
                    status: 200,
                    content_type: Some("application/json".to_string()),
                    body: tokens_body().into_bytes(),
                })
            })),
            ..SubscriptionAuthOptions::default()
        };
        let credential = SubscriptionCredential {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: 1,
            scopes: vec!["openid".to_string()],
        };
        let refreshed = refresh_subscription(&options, &credential, &CancelToken::new()).unwrap();
        assert_eq!(refreshed.refresh_token, "test-refresh");
        assert_eq!(refreshed.scopes, vec!["openid".to_string(), "profile".to_string()]);
        assert!(refreshed.expires_at > now_ms());

        let broken = SubscriptionAuthOptions {
            fetch: Some(Arc::new(|_: &str, _: &FetchInit| {
                Ok(LlmResponse {
                    status: 200,
                    content_type: Some("application/json".to_string()),
                    body: "{\"access_token\":\"new\",\"expires_in\":3600}".as_bytes().to_vec(),
                })
            })),
            ..SubscriptionAuthOptions::default()
        };
        assert_eq!(
            refresh_subscription(&broken, &credential, &CancelToken::new())
                .unwrap_err()
                .to_string(),
            "Subscription token request failed. Try signing in again."
        );
    }

    #[test]
    fn loopback_login_verifies_pkce_and_exchanges_the_code() {
        let challenge: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
        let seen = Arc::clone(&challenge);
        let browser: BrowserHandles = Arc::new(std::sync::Mutex::new(Vec::new()));
        let driver = Arc::clone(&browser);
        let options = SubscriptionAuthOptions {
            callback_port: Some(0),
            fetch: Some(Arc::new(move |_: &str, init: &FetchInit| {
                let pairs = parse_query(init.body.as_deref().unwrap_or(""));
                let field = |key: &str| {
                    pairs
                        .iter()
                        .find(|(name, _)| name == key)
                        .map(|(_, value)| value.clone())
                };
                assert_eq!(field("grant_type").as_deref(), Some("authorization_code"));
                assert_eq!(field("client_id").as_deref(), Some(CLIENT_ID));
                assert_eq!(field("code").as_deref(), Some("test-code"));
                let verifier = field("code_verifier").unwrap_or_default();
                assert!(!verifier.is_empty());
                assert_eq!(
                    base64url_encode(&sha256(verifier.as_bytes())),
                    seen.lock().unwrap().clone().unwrap_or_default()
                );
                Ok(LlmResponse {
                    status: 200,
                    content_type: Some("application/json".to_string()),
                    body: tokens_body().into_bytes(),
                })
            })),
            open_browser: Some({
                let challenge = Arc::clone(&challenge);
                Arc::new(move |url: &str, _: &CancelToken| {
                    assert!(url.contains("code_challenge_method=S256"));
                    assert!(url.contains(&form_encode("openid profile email offline_access")));
                    let presented = url
                        .split("code_challenge=")
                        .nth(1)
                        .and_then(|rest| rest.split('&').next())
                        .map(form_decode)
                        .unwrap_or_default();
                    assert!(!presented.is_empty());
                    *challenge.lock().unwrap() = Some(presented);
                    let target = callback_url(url);
                    driver.lock().unwrap().push(std::thread::spawn(move || get(&target)));
                    Ok(())
                })
            }),
            ..SubscriptionAuthOptions::default()
        };
        let credential = login_subscription(&options, &CancelToken::new()).unwrap();
        assert_eq!(credential.access_token, "test-access");
        assert!(challenge.lock().unwrap().is_some());
        for handle in browser.lock().unwrap().drain(..) {
            assert_eq!(
                handle.join().unwrap(),
                (200, "Authorization received. You can return to the game.".to_string())
            );
        }
    }

    #[test]
    fn login_rejects_state_mismatch_without_tokens() {
        let browser: BrowserHandles = Arc::new(std::sync::Mutex::new(Vec::new()));
        let driver = Arc::clone(&browser);
        let options = SubscriptionAuthOptions {
            callback_port: Some(0),
            fetch: Some(token_fetcher()),
            open_browser: Some(Arc::new(move |url: &str, _: &CancelToken| {
                let target = callback_url(url).replace("state=", "state=wrong-secret&error=provider-secret&ignored=");
                driver.lock().unwrap().push(std::thread::spawn(move || get(&target)));
                Ok(())
            })),
            ..SubscriptionAuthOptions::default()
        };
        let error = login_subscription(&options, &CancelToken::new()).unwrap_err();
        assert_eq!(error.to_string(), "Subscription sign-in state mismatch.");
        for handle in browser.lock().unwrap().drain(..) {
            let (status, body) = handle.join().unwrap();
            assert_eq!(status, 400);
            assert!(!body.contains("secret"));
        }
    }

    #[test]
    fn login_without_browser_is_unavailable_and_cancel_wins() {
        let options = SubscriptionAuthOptions {
            callback_port: Some(0),
            fetch: Some(token_fetcher()),
            ..SubscriptionAuthOptions::default()
        };
        assert_eq!(
            login_subscription(&options, &CancelToken::new())
                .unwrap_err()
                .to_string(),
            "Sign-in browser is unavailable."
        );
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert_eq!(
            login_subscription(&options, &cancelled).unwrap_err(),
            super::cancelled()
        );
    }

    #[test]
    fn occupied_port_and_callback_timeout_settle_safely() {
        let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let options = SubscriptionAuthOptions {
            callback_port: Some(port),
            fetch: Some(token_fetcher()),
            open_browser: Some(Arc::new(|_: &str, _: &CancelToken| panic!("must not run"))),
            ..SubscriptionAuthOptions::default()
        };
        assert!(login_subscription(&options, &CancelToken::new())
            .unwrap_err()
            .to_string()
            .contains("local sign-in callback"));

        let stalled = SubscriptionAuthOptions {
            callback_port: Some(0),
            callback_timeout: Some(Duration::from_millis(50)),
            fetch: Some(token_fetcher()),
            open_browser: Some(Arc::new(|_: &str, _: &CancelToken| Ok(()))),
            ..SubscriptionAuthOptions::default()
        };
        assert_eq!(
            login_subscription(&stalled, &CancelToken::new())
                .unwrap_err()
                .to_string(),
            "Subscription sign-in timed out. Try again."
        );
    }
}
