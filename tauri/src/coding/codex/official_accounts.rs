use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, Mutex as AsyncMutex};

use super::adapter;
use super::commands::{
    apply_config_internal, get_codex_root_dir_from_db_async, get_codex_root_dir_without_db,
};
use super::constants::CODEX_LOCAL_PROVIDER_ID;
use super::types::{
    CodexCredits, CodexOfficialAccount, CodexOfficialAccountContent, CodexProvider,
};
use crate::coding::db_id::db_new_id;
use crate::db::helpers::{
    db_delete, db_get, db_list, db_patch_fields, db_put, db_query_by_field,
    db_update_applied_status,
};
use crate::db::schema::{DbTable, JsonFieldPath, OrderDirection, OrderField, OrderSpec};
use crate::db::SqliteDbState;
use tauri::{Emitter, Manager};

const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_OAUTH_DEFAULT_PORT: u16 = 1455;
// These ports are registered in the Codex OAuth redirect URI allow-list.
const CODEX_OAUTH_FALLBACK_PORT: u16 = 1457;
const CODEX_OAUTH_CALLBACK_PATH: &str = "/auth/callback";
const CODEX_DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const CODEX_DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const CODEX_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const LOCAL_OFFICIAL_ACCOUNT_ID: &str = CODEX_LOCAL_PROVIDER_ID;
const FIVE_HOUR_WINDOW_SECONDS: i64 = 18_000;
const WEEK_WINDOW_SECONDS: i64 = 604_800;
const MONTH_WINDOW_MIN_SECONDS: i64 = 28 * 24 * 60 * 60;
const MONTH_WINDOW_MAX_SECONDS: i64 = 31 * 24 * 60 * 60;
const AUTH_REFRESH_LEAD_SECONDS: i64 = 3 * 24 * 60 * 60;
static OAUTH_REFRESH_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));
static OAUTH_REFRESH_CACHE: LazyLock<
    AsyncMutex<HashMap<String, (std::time::Instant, OAuthTokenResponse)>>,
> = LazyLock::new(|| AsyncMutex::new(HashMap::new()));
struct DeviceAuthSession {
    cancel: watch::Sender<bool>,
    exchange_started: bool,
}

static DEVICE_AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, DeviceAuthSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static BROWSER_AUTH_SESSIONS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_USAGE_REQUEST: AtomicU64 = AtomicU64::new(1);
static USAGE_REQUESTS: LazyLock<Mutex<HashMap<String, UsageRequestState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct UsageRequestState {
    latest: u64,
    active: usize,
}

/// Tracks only in-flight requests. The newest started request owns the row's
/// usage fields; token rotations remain independently persisted before usage.
struct UsageRequestGuard {
    account_id: String,
    generation: u64,
}

impl UsageRequestGuard {
    fn begin(account_id: &str) -> Self {
        let mut requests = USAGE_REQUESTS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let generation = NEXT_USAGE_REQUEST.fetch_add(1, Ordering::Relaxed);
        let state = requests
            .entry(account_id.to_string())
            .or_insert(UsageRequestState {
                latest: generation,
                active: 0,
            });
        state.latest = generation;
        state.active += 1;
        Self {
            account_id: account_id.to_string(),
            generation,
        }
    }

    fn is_current(&self, requests: &HashMap<String, UsageRequestState>) -> bool {
        requests
            .get(&self.account_id)
            .is_some_and(|state| state.latest == self.generation)
    }
}

impl Drop for UsageRequestGuard {
    fn drop(&mut self) {
        let mut requests = USAGE_REQUESTS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(state) = requests.get_mut(&self.account_id) {
            state.active -= 1;
            if state.active == 0 {
                requests.remove(&self.account_id);
            }
        }
    }
}

/// Test-only endpoint override so the OAuth exchange and usage request can be
/// pointed at a local mock server.
#[cfg(test)]
static TEST_ENDPOINTS: LazyLock<Mutex<Option<(String, String)>>> =
    LazyLock::new(|| Mutex::new(None));

#[cfg(test)]
fn set_test_endpoints(usage_url: &str, token_url: &str) {
    *TEST_ENDPOINTS.lock().expect("test endpoint lock") =
        Some((usage_url.to_string(), token_url.to_string()));
}

#[cfg(test)]
fn clear_test_endpoints() {
    *TEST_ENDPOINTS.lock().expect("test endpoint lock") = None;
}

fn codex_usage_url() -> String {
    #[cfg(test)]
    if let Some((usage_url, _)) = TEST_ENDPOINTS.lock().expect("test endpoint lock").as_ref() {
        return usage_url.clone();
    }
    CODEX_USAGE_URL.to_string()
}

fn codex_oauth_token_url() -> String {
    #[cfg(test)]
    if let Some((_, token_url)) = TEST_ENDPOINTS.lock().expect("test endpoint lock").as_ref() {
        return token_url.clone();
    }
    CODEX_OAUTH_TOKEN_URL.to_string()
}

/// Machine-readable marker for "this account cannot be recovered; the user must
/// authorize again". The frontend matches this prefix and shows localized copy,
/// so the trailing English text is only a fallback for logs and older builds.
const CODEX_REAUTH_REQUIRED_MARKER: &str = "codex_reauth_required:";

/// Every unrecoverable credential problem ends up here so the UI can offer
/// "sign in again" instead of replaying a generic HTTP error.
fn codex_reauth_error(reason: &str) -> String {
    format!(
        "{CODEX_REAUTH_REQUIRED_MARKER} Codex official account authorization expired ({reason}). Sign in again with OAuth."
    )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOfficialAccountTokenCopyInput {
    pub provider_id: String,
    pub account_id: String,
    pub token_kind: String,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthTokenRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
    code_verifier: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthRefreshRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceAuthStartResult {
    pub session_id: String,
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at: i64,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexBrowserAuthStartResult {
    pub session_id: String,
    pub authorize_url: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexAuthStatusEvent {
    session_id: String,
    status: String,
    message: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexDeviceUserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Value,
}

#[derive(Debug, Deserialize)]
struct CodexDeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtAuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_account_is_fedramp: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct JwtClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<JwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<JwtAuthClaims>,
}

#[derive(Debug, Clone, Default)]
struct ParsedIdToken {
    email: Option<String>,
    plan_type: Option<String>,
    account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    chatgpt_account_is_fedramp: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UsageSnapshot {
    pub(crate) limit_short_label: Option<String>,
    pub(crate) limit_5h_text: Option<String>,
    pub(crate) limit_weekly_text: Option<String>,
    limit_monthly_text: Option<String>,
    pub(crate) limit_5h_reset_at: Option<i64>,
    pub(crate) limit_weekly_reset_at: Option<i64>,
    limit_monthly_reset_at: Option<i64>,
    reset_credits_available: Option<i64>,
    credits: Option<CodexCredits>,
}

fn oauth_scopes() -> &'static str {
    "openid profile email offline_access api.connectors.read api.connectors.invoke"
}

fn ensure_persisted_provider_id(provider_id: &str) -> Result<(), String> {
    if provider_id == CODEX_LOCAL_PROVIDER_ID {
        Err("The local Codex provider cannot manage official accounts".to_string())
    } else {
        Ok(())
    }
}

fn encode_url_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    encoded
}

fn generate_random_urlsafe(bytes_len: usize) -> String {
    let mut random_bytes = Vec::with_capacity(bytes_len);
    while random_bytes.len() < bytes_len {
        random_bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    random_bytes.truncate(bytes_len);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

fn generate_pkce_pair() -> (String, String) {
    let code_verifier = generate_random_urlsafe(32);
    let challenge_hash = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge_hash);
    (code_verifier, code_challenge)
}

fn build_oauth_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CODEX_OAUTH_CALLBACK_PATH}")
}

fn build_codex_authorize_url(redirect_uri: &str, state: &str, code_challenge: &str) -> String {
    let mut authorize_url = format!(
        "{CODEX_OAUTH_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        encode_url_component(CODEX_OAUTH_CLIENT_ID),
        encode_url_component(redirect_uri),
        encode_url_component(oauth_scopes()),
        encode_url_component(state),
        encode_url_component(code_challenge),
    );
    let extra_params = [
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli_rs"),
    ];
    for (key, value) in extra_params {
        authorize_url.push('&');
        authorize_url.push_str(&encode_url_component(key));
        authorize_url.push('=');
        authorize_url.push_str(&encode_url_component(value));
    }
    authorize_url
}

fn bind_oauth_listener() -> Result<(TcpListener, u16), String> {
    let listener = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_DEFAULT_PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", CODEX_OAUTH_FALLBACK_PORT)))
        .map_err(|error| format!(
            "Codex OAuth callback ports 1455 and 1457 are unavailable. Cancel another login or use device code login: {error}"
        ))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Failed to resolve Codex OAuth callback port: {error}"))?
        .port();
    Ok((listener, port))
}

async fn wait_for_oauth_callback_with_timeout(
    listener: TcpListener,
    state: &str,
    timeout: Duration,
    mut cancelled: watch::Receiver<bool>,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Failed to configure OAuth listener: {error}"))?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|error| format!("Failed to configure OAuth listener: {error}"))?;
    tokio::select! {
        result = tokio::time::timeout(timeout, receive_oauth_callback(&listener, state)) => {
            result.map_err(|_| "expired".to_string())?
        }
        _ = async {
            while !*cancelled.borrow() {
                if cancelled.changed().await.is_err() { break; }
            }
        } => Err("cancelled".to_string()),
    }
}

async fn receive_oauth_callback(
    listener: &tokio::net::TcpListener,
    state: &str,
) -> Result<String, String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("Failed to receive OAuth callback: {error}"))?;
        // Browsers may preconnect, fragment headers, or ask for a favicon. None
        // of those requests should consume the pending authorization session.
        let request = tokio::time::timeout(Duration::from_secs(2), async {
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 1024];
            while bytes.len() < 8192 {
                let size = stream.read(&mut buffer).await?;
                if size == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..size]);
                if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                    break;
                }
            }
            Ok::<_, std::io::Error>(String::from_utf8_lossy(&bytes).into_owned())
        })
        .await;
        let Ok(Ok(request)) = request else {
            continue;
        };
        let mut line = request
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace();
        let method = line.next().unwrap_or_default();
        let target = line.next().unwrap_or_default();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let params = parse_query_string(query);
        let valid_state = params.get("state").map(String::as_str) == Some(state);
        let code = params
            .get("code")
            .filter(|value| !value.trim().is_empty())
            .cloned();
        let valid = method == "GET" && path == CODEX_OAUTH_CALLBACK_PATH && valid_state;
        let (status, message) = if !valid {
            (
                "400 Bad Request",
                "Invalid OAuth callback. Continue the login in your browser.",
            )
        } else if params.contains_key("error") || code.is_none() {
            (
                "400 Bad Request",
                "OAuth authorization failed. Return to AI Toolbox to try again.",
            )
        } else {
            (
                "200 OK",
                "Authorization received. Return to AI Toolbox to finish signing in.",
            )
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{message}", message.len()
        );
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        if !valid {
            continue;
        }
        if params.contains_key("error") {
            return Err("OAuth authorization was declined. Try signing in again.".to_string());
        }
        return code.ok_or_else(|| "OAuth callback missing authorization code".to_string());
    }
}

fn parse_query_string(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((url_decode(key), url_decode(value)))
        })
        .collect()
}

fn url_decode(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.as_bytes().iter().copied().peekable();
    while let Some(byte) = chars.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = chars.next();
                let low = chars.next();
                if let (Some(high), Some(low)) = (high, low) {
                    if let Ok(decoded) =
                        u8::from_str_radix(&String::from_utf8_lossy(&[high, low]), 16)
                    {
                        bytes.push(decoded);
                    }
                }
            }
            _ => bytes.push(byte),
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

async fn exchange_authorization_code(
    db: &SqliteDbState,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = crate::http_client::account_client(db).await?;

    client
        .post(codex_oauth_token_url())
        .form(&OAuthTokenRequest {
            grant_type: "authorization_code",
            client_id: CODEX_OAUTH_CLIENT_ID,
            code,
            redirect_uri,
            code_verifier,
        })
        .send()
        .await
        .map_err(|error| {
            format!(
                "Failed to exchange authorization code: {}",
                crate::http_client::describe_error(&error)
            )
        })?
        .error_for_status()
        .map_err(|error| format!("OAuth token exchange failed: {error}"))?
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|error| format!("Failed to parse OAuth token response: {error}"))
}

async fn refresh_oauth_token(
    db: &SqliteDbState,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, String> {
    let client = crate::http_client::account_client(db).await?;
    let response = client
        .post(codex_oauth_token_url())
        .form(&OAuthRefreshRequest {
            grant_type: "refresh_token",
            client_id: CODEX_OAUTH_CLIENT_ID,
            refresh_token,
        })
        .send()
        .await
        .map_err(|error| {
            format!(
                "Failed to refresh OAuth token: {}",
                crate::http_client::describe_error(&error)
            )
        })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read OAuth refresh response: {error}"))?;
    if !status.is_success() {
        let error_code = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
            value
                .get("code")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        if matches!(
            error_code.as_deref(),
            Some("refresh_token_reused" | "invalid_grant")
        ) || body.contains("refresh_token_reused")
            || body.contains("invalid_grant")
        {
            return Err(codex_reauth_error(
                "the refresh token was rotated, revoked, or rejected",
            ));
        }
        return Err(format!("OAuth token refresh failed ({status}): {body}"));
    }
    serde_json::from_str::<OAuthTokenResponse>(&body)
        .map_err(|error| format!("Failed to parse refreshed OAuth token response: {error}"))
}

fn decode_jwt_payload(value: &str) -> Result<JwtClaims, String> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format".to_string());
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|error| format!("Failed to decode JWT payload: {error}"))?;
    serde_json::from_slice::<JwtClaims>(&payload)
        .map_err(|error| format!("Failed to parse JWT payload: {error}"))
}

fn parse_id_token(id_token: &str) -> ParsedIdToken {
    let claims = decode_jwt_payload(id_token).unwrap_or_default();
    let auth = claims.auth.unwrap_or_default();

    ParsedIdToken {
        email: claims
            .email
            .or_else(|| claims.profile.and_then(|profile| profile.email)),
        plan_type: auth.chatgpt_plan_type,
        account_id: auth.chatgpt_account_id,
        chatgpt_user_id: auth.chatgpt_user_id.or(auth.user_id),
        chatgpt_account_is_fedramp: auth.chatgpt_account_is_fedramp,
    }
}

fn build_auth_snapshot(
    token_response: &OAuthTokenResponse,
    existing_auth: Option<&Value>,
) -> Result<Value, String> {
    let existing_tokens = existing_auth.and_then(|value| value.get("tokens"));
    let id_token = token_response
        .id_token
        .as_deref()
        .or_else(|| {
            existing_tokens
                .and_then(|tokens| tokens.get("id_token"))
                .and_then(Value::as_str)
        })
        .ok_or_else(|| "OAuth response missing id_token".to_string())?;
    let parsed = parse_id_token(id_token);
    let account_id = parsed
        .account_id
        .clone()
        .ok_or_else(|| "OAuth response missing ChatGPT account id".to_string())?;

    let mut auth_object = existing_auth
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();

    auth_object.insert(
        "auth_mode".to_string(),
        Value::String("chatgpt".to_string()),
    );
    auth_object.insert(
        "tokens".to_string(),
        serde_json::json!({
            "id_token": id_token,
            "access_token": token_response.access_token,
            "refresh_token": token_response.refresh_token.clone().or_else(|| {
                existing_tokens
                    .and_then(|tokens| tokens.get("refresh_token"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }).unwrap_or_default(),
            "account_id": account_id,
        }),
    );
    auth_object.insert(
        "last_refresh".to_string(),
        Value::String(Local::now().to_rfc3339()),
    );
    auth_object.remove("OPENAI_API_KEY");
    auth_object.remove("agent_identity");
    let _ = parsed.chatgpt_user_id;
    let _ = parsed.chatgpt_account_is_fedramp;

    Ok(Value::Object(auth_object))
}

pub(super) fn auth_has_official_runtime(auth: &Value) -> bool {
    let auth_mode = auth
        .get("auth_mode")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let access_token = auth
        .pointer("/tokens/access_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    auth_mode.is_some() && access_token.is_some() && refresh_token.is_some()
}

fn token_expiration_unix(token: &str) -> Option<i64> {
    decode_jwt_payload(token).ok()?.exp
}

fn access_token_expiration(auth: &Value) -> Option<i64> {
    auth.pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .and_then(token_expiration_unix)
}

fn official_auth_needs_refresh(auth: &Value) -> bool {
    let now = chrono::Utc::now().timestamp();
    let expiration = access_token_expiration(auth).or_else(|| {
        auth.pointer("/tokens/id_token")
            .and_then(|value| value.as_str())
            .and_then(token_expiration_unix)
    });

    matches!(expiration, Some(expiration) if expiration <= now + AUTH_REFRESH_LEAD_SECONDS)
}

fn auth_refresh_token(auth: &Value) -> Option<&str> {
    auth.pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn auth_email(auth: &Value) -> Option<String> {
    auth.pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .and_then(|parsed| parsed.email)
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
}

/// Two credentials belong to the same ChatGPT login.
///
/// Refresh tokens are compared last on purpose: rotation legitimately changes
/// them, so an equal account id or email must win.
fn auth_identities_match(left: &Value, right: &Value) -> bool {
    if let (Some(left_id), Some(right_id)) = (
        usage_account_id_from_auth(left),
        usage_account_id_from_auth(right),
    ) {
        return left_id == right_id;
    }
    if let (Some(left_email), Some(right_email)) = (auth_email(left), auth_email(right)) {
        return left_email == right_email;
    }
    match (auth_refresh_token(left), auth_refresh_token(right)) {
        (Some(left_token), Some(right_token)) => left_token == right_token,
        _ => false,
    }
}

/// A runtime credential is adopted only when it provably outlives the stored
/// one. "Different" is not "newer": the stored value may be the fresh one.
fn runtime_credential_is_newer(runtime: &Value, stored: &Value) -> bool {
    match (
        access_token_expiration(runtime),
        access_token_expiration(stored),
    ) {
        (Some(runtime_expiration), Some(stored_expiration)) => {
            runtime_expiration > stored_expiration
        }
        _ => false,
    }
}

fn credential_changed_since_attempt(latest: &Value, attempted: &Value) -> bool {
    auth_identities_match(latest, attempted)
        && (access_token_from_auth(latest).ok() != access_token_from_auth(attempted).ok()
            || auth_refresh_token(latest) != auth_refresh_token(attempted))
}

/// Exchange the refresh token for a new credential. The caller must already
/// hold `OAUTH_REFRESH_LOCK`, and must have re-read the stored credential
/// inside the lock so a concurrently rotated refresh token is not consumed
/// twice. `force` skips the expiry check, which is what a 401 with an
/// unexpired token needs.
async fn exchange_official_auth_locked(
    db: &SqliteDbState,
    auth: &Value,
    force: bool,
) -> Result<Value, String> {
    if !force && !official_auth_needs_refresh(auth) {
        return Ok(auth.clone());
    }
    let refresh_token = auth_refresh_token(auth)
        .ok_or_else(|| codex_reauth_error("the account has no usable refresh token"))?;
    if !force {
        let cached_response = OAUTH_REFRESH_CACHE
            .lock()
            .await
            .get(refresh_token)
            .filter(|(created_at, _)| created_at.elapsed() <= Duration::from_secs(30))
            .map(|(_, response)| response.clone());
        if let Some(response) = cached_response {
            return build_auth_snapshot(&response, Some(auth));
        }
    }
    let response = refresh_oauth_token(db, refresh_token).await?;
    OAUTH_REFRESH_CACHE.lock().await.insert(
        refresh_token.to_string(),
        (std::time::Instant::now(), response.clone()),
    );
    build_auth_snapshot(&response, Some(auth))
}

async fn ensure_fresh_official_runtime_auth(
    db: &SqliteDbState,
    auth: &Value,
) -> Result<Value, String> {
    if !official_auth_needs_refresh(auth) {
        return Ok(auth.clone());
    }
    let _refresh_guard = OAUTH_REFRESH_LOCK.lock().await;
    exchange_official_auth_locked(db, auth, false).await
}

/// The runtime credential when it belongs to the same login and is newer than
/// `stored_auth`. Never borrows another account's login.
async fn newer_runtime_credential(
    db: &SqliteDbState,
    stored_auth: &Value,
) -> Result<Option<Value>, String> {
    let runtime_auth = read_auth_json_from_disk(Some(db)).await?;
    if !auth_has_official_runtime(&runtime_auth)
        || !auth_identities_match(stored_auth, &runtime_auth)
        || !runtime_credential_is_newer(&runtime_auth, stored_auth)
    {
        return Ok(None);
    }
    Ok(Some(runtime_auth))
}

/// Credential a usage request should use, plus the account row it belongs to.
struct PreparedOfficialCredential {
    account: CodexOfficialAccount,
    auth: Value,
    /// A rotated credential was written back to the runtime `auth.json`.
    runtime_written: bool,
}

/// Picks the newest credential this account may use, without exchanging.
///
/// Applied accounts may adopt the runtime `auth.json` credential, but only
/// when it is provably the same login *and* outlives the stored token. The
/// adopted credential is persisted immediately, so the database stops
/// lagging behind the runtime.
async fn newest_account_credential(
    db: &SqliteDbState,
    account: &CodexOfficialAccount,
) -> Result<(CodexOfficialAccount, Value), String> {
    let stored = stored_auth_from_account(account)?;
    if !account.is_applied {
        return Ok((account.clone(), stored));
    }
    let Some(runtime_auth) = newer_runtime_credential(db, &stored).await? else {
        return Ok((account.clone(), stored));
    };
    let updated = persist_refreshed_account_snapshot(db, account, &runtime_auth).await?;
    Ok((updated, runtime_auth))
}

/// Fresh credential for an account. Any rotation is persisted before this
/// returns, so a later usage failure can never discard a rotated refresh token.
///
/// A runtime-derived credential needs no exchange (it is the newest thing we
/// have); otherwise the stored snapshot is re-read under the OAuth lock before
/// the expiry check, so a concurrent refresh is not exchanged twice.
async fn prepare_persisted_account_credential(
    db: &SqliteDbState,
    account_id: &str,
) -> Result<PreparedOfficialCredential, String> {
    let (account, auth) =
        newest_account_credential(db, &load_official_account(db, account_id).await?).await?;
    if !official_auth_needs_refresh(&auth) {
        return Ok(PreparedOfficialCredential {
            account,
            auth,
            runtime_written: false,
        });
    }

    let _refresh_guard = OAUTH_REFRESH_LOCK.lock().await;
    // Re-read inside the lock: the account may have been refreshed by the
    // background pass or another click while we were waiting.
    let (latest, base) =
        newest_account_credential(db, &load_official_account(db, account_id).await?).await?;
    if !official_auth_needs_refresh(&base) {
        return Ok(PreparedOfficialCredential {
            account: latest,
            auth: base,
            runtime_written: false,
        });
    }
    let next = exchange_official_auth_locked(db, &base, false).await?;
    let account = persist_refreshed_account_snapshot(db, &latest, &next).await?;
    let runtime_written = write_runtime_credential_if_still_applied(db, account_id, &next).await?;
    Ok(PreparedOfficialCredential {
        account,
        auth: next,
        runtime_written,
    })
}

/// One controlled recovery after the usage endpoint rejected a credential:
/// prefer a newer same-account runtime credential, otherwise force exactly one
/// OAuth exchange. The exchange itself reports "sign in again" when the
/// refresh token is gone or rejected.
async fn recover_persisted_account_credential(
    db: &SqliteDbState,
    account_id: &str,
    attempted_auth: &Value,
) -> Result<PreparedOfficialCredential, String> {
    let account = load_official_account(db, account_id).await?;
    if account.is_applied {
        if let Some(runtime_auth) = newer_runtime_credential(db, attempted_auth).await? {
            let updated = persist_refreshed_account_snapshot(db, &account, &runtime_auth).await?;
            return Ok(PreparedOfficialCredential {
                account: updated,
                auth: runtime_auth,
                runtime_written: false,
            });
        }
    }

    let _refresh_guard = OAUTH_REFRESH_LOCK.lock().await;
    let (latest, base) =
        newest_account_credential(db, &load_official_account(db, account_id).await?).await?;
    if credential_changed_since_attempt(&base, attempted_auth) {
        return Ok(PreparedOfficialCredential {
            account: latest,
            auth: base,
            runtime_written: false,
        });
    }
    let next = exchange_official_auth_locked(db, &base, true).await?;
    let updated = persist_refreshed_account_snapshot(db, &latest, &next).await?;
    let runtime_written = write_runtime_credential_if_still_applied(db, account_id, &next).await?;
    Ok(PreparedOfficialCredential {
        account: updated,
        auth: next,
        runtime_written,
    })
}

fn stored_auth_from_account(account: &CodexOfficialAccount) -> Result<Value, String> {
    let snapshot = account
        .auth_snapshot
        .as_deref()
        .filter(|snapshot| !snapshot.trim().is_empty())
        .ok_or_else(|| "Official account snapshot is missing".to_string())?;
    auth_json_from_snapshot(snapshot)
}

/// Writes a rotated credential into the runtime `auth.json`, but only while
/// this account is still the applied one and the file still holds the same
/// login. A concurrent account switch therefore never gets overwritten.
async fn write_runtime_credential_if_still_applied(
    db: &SqliteDbState,
    account_id: &str,
    next_auth: &Value,
) -> Result<bool, String> {
    let current_account = load_official_account(db, account_id).await?;
    if !current_account.is_applied {
        return Ok(false);
    }
    let current_auth = read_auth_json_from_disk(Some(db)).await?;
    if !auth_has_official_runtime(&current_auth) || !auth_identities_match(&current_auth, next_auth)
    {
        return Ok(false);
    }
    let merged = merge_official_runtime_auth(&current_auth, next_auth);
    write_auth_json_to_disk(db, &merged).await?;
    Ok(true)
}

/// Same guard for the `__local__` virtual account, whose runtime file *is* the
/// credential. `previous_auth` identifies the login the read started from.
async fn write_local_runtime_credential(
    db: &SqliteDbState,
    previous_auth: &Value,
    next_auth: &Value,
) -> Result<bool, String> {
    let current_auth = read_auth_json_from_disk(Some(db)).await?;
    if !auth_has_official_runtime(&current_auth)
        || !auth_identities_match(previous_auth, &current_auth)
    {
        return Ok(false);
    }
    let merged = merge_official_runtime_auth(&current_auth, next_auth);
    write_auth_json_to_disk(db, &merged).await?;
    Ok(true)
}

fn auth_json_from_snapshot(snapshot: &str) -> Result<Value, String> {
    serde_json::from_str(snapshot)
        .map_err(|error| format!("Failed to parse account snapshot: {error}"))
}

fn parse_imported_auth_json(content: &str) -> Result<Value, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("Codex auth.json content is empty".to_string());
    }
    let auth = serde_json::from_str::<Value>(content)
        .map_err(|error| format!("Failed to parse auth.json: {error}"))?;
    if !auth.is_object() {
        return Err("Codex auth.json must be a JSON object".to_string());
    }
    if !auth_has_official_runtime(&auth) {
        return Err("auth.json does not contain an official Codex login".to_string());
    }
    Ok(auth)
}

/// Test-only root override so credential flows can run against a temp dir
/// without touching the global runtime-location cache.
#[cfg(test)]
static TEST_AUTH_ROOT: LazyLock<Mutex<Option<std::path::PathBuf>>> =
    LazyLock::new(|| Mutex::new(None));

async fn codex_auth_root(
    db: Option<&crate::db::SqliteDbState>,
) -> Result<std::path::PathBuf, String> {
    #[cfg(test)]
    if let Some(root) = TEST_AUTH_ROOT.lock().expect("test auth root lock").clone() {
        return Ok(root);
    }
    match db {
        Some(db) => get_codex_root_dir_from_db_async(db).await,
        None => get_codex_root_dir_without_db(),
    }
}

async fn read_auth_json_from_disk(db: Option<&crate::db::SqliteDbState>) -> Result<Value, String> {
    let root_dir = codex_auth_root(db).await?;
    let auth_path = root_dir.join("auth.json");
    if !auth_path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = fs::read_to_string(&auth_path)
        .map_err(|error| format!("Failed to read auth.json: {error}"))?;
    serde_json::from_str(&content).map_err(|error| format!("Failed to parse auth.json: {error}"))
}

async fn write_auth_json_to_disk(
    db: &crate::db::SqliteDbState,
    auth: &Value,
) -> Result<(), String> {
    let root_dir = codex_auth_root(Some(db)).await?;
    if !root_dir.exists() {
        fs::create_dir_all(&root_dir)
            .map_err(|error| format!("Failed to create Codex root directory: {error}"))?;
    }
    let auth_path = root_dir.join("auth.json");
    write_auth_json_atomic(&auth_path, auth)
}

fn write_auth_json_atomic(auth_path: &std::path::Path, auth: &Value) -> Result<(), String> {
    let root_dir = auth_path
        .parent()
        .ok_or_else(|| "Codex auth.json path has no parent directory".to_string())?;
    let mut temporary = NamedTempFile::new_in(&root_dir)
        .map_err(|error| format!("Failed to create temporary auth.json: {error}"))?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), auth)
        .map_err(|error| format!("Failed to serialize auth.json: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("Failed to finalize auth.json: {error}"))?;
    temporary
        .persist(&auth_path)
        .map_err(|error| format!("Failed to replace auth.json: {}", error.error))?;
    set_codex_auth_permissions(&auth_path)
}

#[cfg(unix)]
fn set_codex_auth_permissions(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Failed to set auth.json permissions: {error}"))
}

#[cfg(not(unix))]
fn set_codex_auth_permissions(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

async fn query_provider(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<CodexProvider, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CodexProvider, provider_id))?
        .map(adapter::from_db_value_provider)
        .ok_or_else(|| format!("Codex provider '{}' not found", provider_id))
}

async fn list_persisted_official_accounts(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<Vec<CodexOfficialAccount>, String> {
    let provider_id_value = Value::String(provider_id.to_string());
    let provider_id_path = JsonFieldPath::new("provider_id")?;
    let order = OrderSpec::new(vec![
        OrderField::json_integer("sort_index", OrderDirection::Asc)?,
        OrderField::json_text("created_at", OrderDirection::Asc)?,
    ]);
    db.with_conn(|conn| {
        Ok(db_query_by_field(
            conn,
            DbTable::CodexOfficialAccount,
            &provider_id_path,
            &provider_id_value,
            Some(&order),
            None,
        )?
        .into_iter()
        .map(adapter::from_db_value_official_account)
        .collect())
    })
}

fn build_virtual_local_account(auth: &Value) -> CodexOfficialAccount {
    let parsed = auth
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .unwrap_or_default();
    let now = Local::now().to_rfc3339();

    CodexOfficialAccount {
        id: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        provider_id: String::new(),
        name: LOCAL_OFFICIAL_ACCOUNT_ID.to_string(),
        kind: "local".to_string(),
        email: parsed.email.clone(),
        auth_snapshot: Some(auth.to_string()),
        auth_mode: auth
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        account_id: auth
            .pointer("/tokens/account_id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or(parsed.account_id.clone()),
        plan_type: parsed.plan_type.clone(),
        last_refresh: auth
            .get("last_refresh")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        token_expires_at: auth
            .pointer("/tokens/access_token")
            .and_then(|value| value.as_str())
            .and_then(token_expiration_unix)
            .or_else(|| {
                auth.pointer("/tokens/id_token")
                    .and_then(|value| value.as_str())
                    .and_then(token_expiration_unix)
            }),
        access_token_preview: auth
            .pointer("/tokens/access_token")
            .and_then(|value| value.as_str())
            .and_then(|value| {
                let trimmed = value.trim();
                let char_count = trimmed.chars().count();
                if trimmed.is_empty() {
                    None
                } else if char_count <= 12 {
                    Some(trimmed.to_string())
                } else {
                    let head: String = trimmed.chars().take(6).collect();
                    let tail: String = trimmed
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    Some(format!("{head}...{tail}"))
                }
            }),
        refresh_token_preview: auth
            .pointer("/tokens/refresh_token")
            .and_then(|value| value.as_str())
            .and_then(|value| {
                let trimmed = value.trim();
                let char_count = trimmed.chars().count();
                if trimmed.is_empty() {
                    None
                } else if char_count <= 12 {
                    Some(trimmed.to_string())
                } else {
                    let head: String = trimmed.chars().take(6).collect();
                    let tail: String = trimmed
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    Some(format!("{head}...{tail}"))
                }
            }),
        limit_short_label: None,
        limit_5h_text: None,
        limit_weekly_text: None,
        limit_monthly_text: None,
        limit_5h_reset_at: None,
        limit_weekly_reset_at: None,
        limit_monthly_reset_at: None,
        reset_credits_available: None,
        credits: None,
        last_limits_fetched_at: None,
        last_error: None,
        sort_index: None,
        is_applied: false,
        is_virtual: true,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn official_account_identity_matches_auth(account: &CodexOfficialAccount, auth: &Value) -> bool {
    let local_refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let local_account_id = auth
        .pointer("/tokens/account_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            auth.pointer("/tokens/id_token")
                .and_then(|value| value.as_str())
                .map(parse_id_token)
                .and_then(|parsed| parsed.account_id)
        });
    let local_email = auth
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .and_then(|parsed| parsed.email)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    let account_refresh_token = account
        .auth_snapshot
        .as_deref()
        .and_then(|snapshot| serde_json::from_str::<Value>(snapshot).ok())
        .and_then(|snapshot| {
            snapshot
                .pointer("/tokens/refresh_token")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    if let (Some(local_refresh_token), Some(account_refresh_token)) =
        (local_refresh_token, account_refresh_token.as_deref())
    {
        if local_refresh_token == account_refresh_token {
            return true;
        }
    }

    if let (Some(local_account_id), Some(account_account_id)) =
        (local_account_id.as_deref(), account.account_id.as_deref())
    {
        if local_account_id == account_account_id {
            return true;
        }
    }

    if let (Some(local_email), Some(account_email)) =
        (local_email.as_deref(), account.email.as_deref())
    {
        if local_email == account_email.trim().to_ascii_lowercase() {
            return true;
        }
    }

    false
}

fn should_show_virtual_local_account(
    persisted_accounts: &[CodexOfficialAccount],
    local_auth: &Value,
) -> bool {
    if !auth_has_official_runtime(local_auth) {
        return false;
    }

    !persisted_accounts
        .iter()
        .any(|account| official_account_identity_matches_auth(account, local_auth))
}

fn parse_remaining_percent_from_window(window: &Value) -> Option<f64> {
    if !window.is_object() {
        return None;
    }
    if let Some(used_percent) = window
        .get("used_percent")
        .and_then(Value::as_f64)
        .or_else(|| window.get("usedPercent").and_then(Value::as_f64))
    {
        return Some((100.0 - used_percent).clamp(0.0, 100.0));
    }

    let remaining = window
        .get("remaining_count")
        .and_then(Value::as_f64)
        .or_else(|| window.get("remainingCount").and_then(Value::as_f64));
    let total = window
        .get("total_count")
        .and_then(Value::as_f64)
        .or_else(|| window.get("totalCount").and_then(Value::as_f64));
    match (remaining, total) {
        (Some(remaining), Some(total)) if total > 0.0 => {
            Some((remaining / total * 100.0).clamp(0.0, 100.0))
        }
        _ => None,
    }
}

fn parse_remaining_percent_from_rate_window(window: RateWindowRef<'_>) -> Option<f64> {
    parse_remaining_percent_from_window(window.value).or_else(|| {
        let exhausted = window.limit_reached.unwrap_or(false) || window.allowed == Some(false);
        if exhausted && extract_reset_timestamp(window.value).is_some() {
            Some(0.0)
        } else {
            None
        }
    })
}

fn format_percent_label(value: f64) -> String {
    format!("{:.0}%", value.clamp(0.0, 100.0))
}

#[derive(Debug, Default)]
struct ResolvedRateWindows<'a> {
    primary: Option<RateWindowRef<'a>>,
    secondary: Option<RateWindowRef<'a>>,
    monthly: Option<RateWindowRef<'a>>,
    all: Vec<RateWindowRef<'a>>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitSource<'a> {
    value: &'a Value,
    limit_reached: Option<bool>,
    allowed: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
struct RateWindowRef<'a> {
    value: &'a Value,
    limit_reached: Option<bool>,
    allowed: Option<bool>,
}

fn looks_like_rate_window(value: &Value) -> bool {
    if !value.is_object() {
        return false;
    }

    extract_limit_window_seconds(value).is_some()
        || value.get("used_percent").is_some()
        || value.get("usedPercent").is_some()
        || value.get("remaining_count").is_some()
        || value.get("remainingCount").is_some()
        || value.get("total_count").is_some()
        || value.get("totalCount").is_some()
        || value.get("reset_at").is_some()
        || value.get("resetAt").is_some()
        || value.get("resets_at").is_some()
        || value.get("resetsAt").is_some()
        || value.get("reset_after_seconds").is_some()
        || value.get("resetAfterSeconds").is_some()
        || value.get("resets_in_seconds").is_some()
        || value.get("resetsInSeconds").is_some()
}

fn json_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn rate_limit_source_from_value(value: &Value) -> RateLimitSource<'_> {
    RateLimitSource {
        value,
        limit_reached: json_bool(
            value
                .get("limit_reached")
                .or_else(|| value.get("limitReached")),
        ),
        allowed: json_bool(value.get("allowed")),
    }
}

fn push_unique_rate_limit_source<'a>(
    sources: &mut Vec<RateLimitSource<'a>>,
    source: Option<&'a Value>,
) {
    if let Some(source) = source.filter(|value| value.is_object()) {
        if !sources
            .iter()
            .any(|existing| std::ptr::eq(existing.value, source))
        {
            sources.push(rate_limit_source_from_value(source));
        }
    }
}

fn first_rate_window_with_aliases<'a>(
    source: RateLimitSource<'a>,
    aliases: &[&str],
) -> Option<RateWindowRef<'a>> {
    aliases
        .iter()
        .find_map(|key| source.value.get(*key))
        .filter(|value| looks_like_rate_window(value))
        .map(|value| RateWindowRef {
            value,
            limit_reached: source.limit_reached,
            allowed: source.allowed,
        })
}

fn push_unique_rate_window<'a>(
    windows: &mut Vec<RateWindowRef<'a>>,
    window: Option<RateWindowRef<'a>>,
) {
    if let Some(window) = window.filter(|window| looks_like_rate_window(window.value)) {
        if !windows
            .iter()
            .any(|existing| std::ptr::eq(existing.value, window.value))
        {
            windows.push(window);
        }
    }
}

fn resolve_rate_windows(body: &Value) -> ResolvedRateWindows<'_> {
    let rate_limit = body
        .get("rate_limit")
        .or_else(|| body.get("rateLimit"))
        .unwrap_or(body);
    let mut sources = Vec::new();
    push_unique_rate_limit_source(&mut sources, Some(rate_limit));
    if !std::ptr::eq(rate_limit, body) {
        push_unique_rate_limit_source(&mut sources, Some(body));
    }
    if let Some(additional_rate_limits) = body
        .get("additional_rate_limits")
        .or_else(|| body.get("additionalRateLimits"))
        .and_then(Value::as_array)
    {
        for entry in additional_rate_limits {
            if let Some(entry_rate_limit) = entry
                .get("rate_limit")
                .or_else(|| entry.get("rateLimit"))
                .filter(|value| value.is_object())
            {
                push_unique_rate_limit_source(&mut sources, Some(entry_rate_limit));
            } else {
                push_unique_rate_limit_source(&mut sources, Some(entry));
            }
        }
    }

    let primary_aliases = [
        "primary_window",
        "primaryWindow",
        "five_hour",
        "5_hour_window",
        "fiveHourWindow",
    ];
    let secondary_aliases = [
        "secondary_window",
        "secondaryWindow",
        "seven_day",
        "weekly_window",
        "weeklyWindow",
    ];
    let monthly_aliases = [
        "monthly_window",
        "monthlyWindow",
        "month_window",
        "monthWindow",
        "thirty_day",
        "thirtyDay",
        "thirty_day_window",
        "thirtyDayWindow",
    ];
    let all_aliases = [
        "primary_window",
        "primaryWindow",
        "secondary_window",
        "secondaryWindow",
        "five_hour",
        "5_hour_window",
        "fiveHourWindow",
        "seven_day",
        "weekly_window",
        "weeklyWindow",
        "monthly_window",
        "monthlyWindow",
        "month_window",
        "monthWindow",
        "thirty_day",
        "thirtyDay",
        "thirty_day_window",
        "thirtyDayWindow",
    ];

    let primary = sources
        .iter()
        .find_map(|source| first_rate_window_with_aliases(*source, &primary_aliases));
    let secondary = sources
        .iter()
        .find_map(|source| first_rate_window_with_aliases(*source, &secondary_aliases));
    let monthly = sources
        .iter()
        .find_map(|source| first_rate_window_with_aliases(*source, &monthly_aliases));

    let mut all = Vec::new();
    push_unique_rate_window(&mut all, primary);
    push_unique_rate_window(&mut all, secondary);
    push_unique_rate_window(&mut all, monthly);
    for source in sources {
        for key in all_aliases {
            push_unique_rate_window(
                &mut all,
                source.value.get(key).map(|value| RateWindowRef {
                    value,
                    limit_reached: source.limit_reached,
                    allowed: source.allowed,
                }),
            );
        }
        if let Some(object) = source.value.as_object() {
            for value in object.values() {
                push_unique_rate_window(
                    &mut all,
                    Some(RateWindowRef {
                        value,
                        limit_reached: source.limit_reached,
                        allowed: source.allowed,
                    }),
                );
            }
        }
    }

    ResolvedRateWindows {
        primary,
        secondary,
        monthly,
        all,
    }
}

fn extract_limit_window_seconds(window: &Value) -> Option<i64> {
    window
        .get("limit_window_seconds")
        .or_else(|| window.get("limitWindowSeconds"))
        .and_then(Value::as_i64)
}

fn is_month_window_seconds(seconds: i64) -> bool {
    (MONTH_WINDOW_MIN_SECONDS..=MONTH_WINDOW_MAX_SECONDS).contains(&seconds)
}

fn is_selected_window(candidate: RateWindowRef<'_>, selected: Option<RateWindowRef<'_>>) -> bool {
    match selected {
        Some(selected) => std::ptr::eq(candidate.value, selected.value),
        None => false,
    }
}

fn classify_rate_windows(
    body: &Value,
) -> (
    Option<RateWindowRef<'_>>,
    Option<RateWindowRef<'_>>,
    Option<RateWindowRef<'_>>,
) {
    let resolved_windows = resolve_rate_windows(body);
    let mut short_window = None;
    let mut weekly_window = None;
    let mut monthly_window = None;

    for window in resolved_windows.all.iter().copied() {
        match extract_limit_window_seconds(window.value) {
            Some(FIVE_HOUR_WINDOW_SECONDS) if short_window.is_none() => {
                short_window = Some(window);
            }
            Some(WEEK_WINDOW_SECONDS) if weekly_window.is_none() => {
                weekly_window = Some(window);
            }
            Some(seconds) if is_month_window_seconds(seconds) && monthly_window.is_none() => {
                monthly_window = Some(window);
            }
            _ => {}
        }
    }

    if monthly_window.is_none() {
        monthly_window = resolved_windows.monthly;
    }
    if short_window.is_none() {
        short_window = resolved_windows.primary.filter(|window| {
            !is_selected_window(*window, weekly_window)
                && !is_selected_window(*window, monthly_window)
        });
    }
    if weekly_window.is_none() {
        weekly_window = resolved_windows.secondary.filter(|window| {
            !is_selected_window(*window, short_window)
                && !is_selected_window(*window, monthly_window)
        });
    }

    (short_window, weekly_window, monthly_window)
}

fn extract_reset_timestamp_at(window: &Value, now_timestamp: i64) -> Option<i64> {
    if let Some(reset_at) = window
        .get("reset_at")
        .or_else(|| window.get("resetAt"))
        .or_else(|| window.get("resets_at"))
        .or_else(|| window.get("resetsAt"))
        .and_then(Value::as_i64)
    {
        return Some(reset_at);
    }

    window
        .get("reset_after_seconds")
        .or_else(|| window.get("resetAfterSeconds"))
        .or_else(|| window.get("resets_in_seconds"))
        .or_else(|| window.get("resetsInSeconds"))
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| now_timestamp.checked_add(seconds))
}

fn extract_reset_timestamp(window: &Value) -> Option<i64> {
    extract_reset_timestamp_at(window, Local::now().timestamp())
}

fn plan_type_has_short_window(plan_type: Option<&str>) -> bool {
    !matches!(
        plan_type
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("free" | "pro" | "prolite" | "pro_lite")
    )
}

fn usage_plan_type_from_auth(auth_snapshot: &Value) -> Option<String> {
    auth_snapshot
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .and_then(|parsed| parsed.plan_type)
}

fn usage_account_id_from_auth(auth_snapshot: &Value) -> Option<String> {
    auth_snapshot
        .pointer("/tokens/account_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            auth_snapshot
                .pointer("/tokens/id_token")
                .and_then(|value| value.as_str())
                .map(parse_id_token)
                .and_then(|parsed| parsed.account_id)
        })
}

fn extract_reset_credits_available(body: &Value) -> Option<i64> {
    let container = body
        .get("rate_limit_reset_credits")
        .or_else(|| body.get("rateLimitResetCredits"))
        .or_else(|| {
            body.get("data").and_then(|data| {
                data.get("rate_limit_reset_credits")
                    .or_else(|| data.get("rateLimitResetCredits"))
            })
        })?;

    container
        .get("available_count")
        .or_else(|| container.get("availableCount"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        })
        .filter(|count| *count >= 0)
}

fn extract_codex_credits(body: &Value) -> Option<CodexCredits> {
    // Same tolerance as extract_reset_credits_available: the field can arrive
    // top-level or under a `data` wrapper, in either naming style. Narrowness
    // here is not harmless — a successful read that parses to None clears the
    // stored balance.
    let credits = body
        .get("credits")
        .or_else(|| body.get("data").and_then(|data| data.get("credits")))?
        .as_object()?;
    let has_credits = credits
        .get("has_credits")
        .or_else(|| credits.get("hasCredits"))
        .and_then(Value::as_bool);
    let unlimited = credits.get("unlimited").and_then(Value::as_bool);
    let balance = credits
        .get("balance")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
        })
        .filter(|balance| balance.is_finite());
    // Do not turn an absent/malformed response into zero. Negative balances are
    // valid: concurrent Codex tasks can temporarily spend beyond the balance.
    (has_credits.is_some() || unlimited.is_some() || balance.is_some()).then_some(CodexCredits {
        has_credits,
        unlimited,
        balance,
    })
}

fn parse_usage_snapshot(body: &Value, plan_type: Option<&str>) -> UsageSnapshot {
    let (short_window, weekly_window, monthly_window) = classify_rate_windows(body);
    // Usage reports the current tier; native credential metadata can be stale.
    let plan_type = body
        .get("plan_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .or(plan_type);
    let has_short_window = plan_type_has_short_window(plan_type);
    let effective_weekly_window = if has_short_window {
        weekly_window
    } else {
        // Legacy primary windows without durations are weekly for no-5h tiers.
        // Never relabel an explicitly timed 5h window as weekly.
        weekly_window
            .or(short_window.filter(|window| extract_limit_window_seconds(window.value).is_none()))
    };

    UsageSnapshot {
        limit_short_label: has_short_window.then(|| "5h".to_string()),
        limit_5h_text: if has_short_window {
            short_window
                .and_then(parse_remaining_percent_from_rate_window)
                .map(format_percent_label)
        } else {
            None
        },
        limit_weekly_text: effective_weekly_window
            .and_then(parse_remaining_percent_from_rate_window)
            .map(format_percent_label),
        limit_monthly_text: monthly_window
            .and_then(parse_remaining_percent_from_rate_window)
            .map(format_percent_label),
        limit_5h_reset_at: if has_short_window {
            short_window.and_then(|window| extract_reset_timestamp(window.value))
        } else {
            None
        },
        limit_weekly_reset_at: effective_weekly_window
            .and_then(|window| extract_reset_timestamp(window.value)),
        limit_monthly_reset_at: monthly_window
            .and_then(|window| extract_reset_timestamp(window.value)),
        reset_credits_available: extract_reset_credits_available(body),
        credits: extract_codex_credits(body),
    }
}

/// Outcome of one usage request. The HTTP status is kept so the caller can
/// distinguish "the token was rejected" from timeouts, rate limits and 5xx.
struct UsageRequestError {
    status: Option<u16>,
    message: String,
}

impl UsageRequestError {
    fn is_unauthorized(&self) -> bool {
        self.status == Some(reqwest::StatusCode::UNAUTHORIZED.as_u16())
    }
}

/// One attempt at the usage endpoint, with the transport failure left unflattened
/// so the caller can decide whether it is worth repeating.
async fn send_usage_request(
    db: &crate::db::SqliteDbState,
    access_token: &str,
    account_id: Option<&str>,
    plan_type: Option<&str>,
) -> Result<UsageSnapshot, UsageAttemptError> {
    let client = crate::http_client::account_client(db)
        .await
        .map_err(|message| {
            UsageAttemptError::Fatal(UsageRequestError {
                status: None,
                message,
            })
        })?;

    let mut request = client
        .get(codex_usage_url())
        .header("Authorization", format!("Bearer {access_token}"))
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AI-Toolbox Codex OAuth",
        )
        .header("Content-Type", "application/json");
    if let Some(account_id) = account_id
        .map(str::trim)
        .filter(|account_id| !account_id.is_empty())
    {
        request = request.header("Chatgpt-Account-Id", account_id);
    }

    let response = request
        .send()
        .await
        .map_err(|error| UsageAttemptError::Send {
            retryable: crate::http_client::is_transient_transport_error(&error),
            error: UsageRequestError {
                status: None,
                // reqwest's Display drops the cause; keep the whole chain so the
                // stored failure says *why* (proxy refused the tunnel, DNS,
                // timeout, TLS) instead of just naming the URL.
                message: format!(
                    "Failed to fetch usage: {}",
                    crate::http_client::describe_error(&error)
                ),
            },
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(UsageAttemptError::Fatal(UsageRequestError {
            status: Some(status.as_u16()),
            message: format!("Codex usage request failed: HTTP status {status}"),
        }));
    }

    let body = response.json::<Value>().await.map_err(|error| {
        UsageAttemptError::Fatal(UsageRequestError {
            status: Some(status.as_u16()),
            message: format!("Failed to parse usage response: {error}"),
        })
    })?;
    Ok(parse_usage_snapshot(&body, plan_type))
}

/// A failed attempt that may still be repeated verbatim.
enum UsageAttemptError {
    /// Never reached the upstream (connect/tunnel/timeout). Repeating the same
    /// request cannot double-consume a quota window, so one retry is safe.
    Send {
        retryable: bool,
        error: UsageRequestError,
    },
    /// The upstream answered, or the request could not be built. Keep as-is.
    Fatal(UsageRequestError),
}

impl UsageAttemptError {
    fn into_request_error(self) -> UsageRequestError {
        match self {
            Self::Send { error, .. } | Self::Fatal(error) => error,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Send {
                retryable: true,
                ..
            }
        )
    }
}

/// Transport failures through a local proxy (the common Chinese-network setup)
/// are frequently one-off: the tunnel is refused while the proxy picks a node,
/// or the node drops the first connection. Without a retry the very first
/// hiccup becomes the persisted `last_error` the user sees until they refresh
/// again, which is exactly how a single blip reads as "usage detection is
/// broken". One retry only — never a loop, and never for HTTP status errors.
const USAGE_TRANSPORT_RETRY_DELAY: Duration = Duration::from_millis(400);

async fn request_usage_snapshot(
    db: &crate::db::SqliteDbState,
    access_token: &str,
    account_id: Option<&str>,
    plan_type: Option<&str>,
) -> Result<UsageSnapshot, UsageRequestError> {
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(20), async {
    match send_usage_request(db, access_token, account_id, plan_type).await {
        Ok(snapshot) => Ok(snapshot),
        Err(error) if error.is_retryable() => {
            log::debug!(
                "Codex usage request failed on transport, retrying once: {}",
                error_message(&error)
            );
            tokio::time::sleep(USAGE_TRANSPORT_RETRY_DELAY).await;
            send_usage_request(db, access_token, account_id, plan_type)
                .await
                .map_err(UsageAttemptError::into_request_error)
        }
        Err(error) => Err(error.into_request_error()),
    }
    }).await.unwrap_or_else(|_| Err(UsageRequestError {
        status: None,
        message: "Codex usage request timed out after 20 seconds. Check the configured proxy and retry.".to_string(),
    }));
    match &result {
        Ok(_) => log::debug!(
            "Codex usage request completed in {} ms",
            started.elapsed().as_millis()
        ),
        Err(error) => log::warn!(
            "Codex usage request failed after {} ms: {}",
            started.elapsed().as_millis(),
            error.message
        ),
    }
    result
}

fn error_message(error: &UsageAttemptError) -> &str {
    match error {
        UsageAttemptError::Send { error, .. } | UsageAttemptError::Fatal(error) => &error.message,
    }
}

pub(crate) async fn fetch_usage_snapshot(
    db: &crate::db::SqliteDbState,
    access_token: &str,
    account_id: Option<&str>,
    plan_type: Option<&str>,
) -> Result<UsageSnapshot, String> {
    request_usage_snapshot(db, access_token, account_id, plan_type)
        .await
        .map_err(|error| error.message)
}

fn build_account_content_from_auth_snapshot(
    provider_id: &str,
    auth_snapshot: &Value,
    usage_snapshot: Option<&UsageSnapshot>,
    name_override: Option<&str>,
) -> Result<CodexOfficialAccountContent, String> {
    let parsed = auth_snapshot
        .pointer("/tokens/id_token")
        .and_then(|value| value.as_str())
        .map(parse_id_token)
        .unwrap_or_default();
    let now = Local::now().to_rfc3339();
    let account_id = auth_snapshot
        .pointer("/tokens/account_id")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or(parsed.account_id.clone());
    let email = parsed.email.clone();
    let display_name = name_override
        .map(|value| value.to_string())
        .or_else(|| email.clone())
        .or_else(|| account_id.clone())
        .unwrap_or_else(|| "official-account".to_string());

    Ok(CodexOfficialAccountContent {
        provider_id: provider_id.to_string(),
        name: display_name,
        kind: "oauth".to_string(),
        email,
        auth_snapshot: serde_json::to_string(auth_snapshot)
            .map_err(|error| format!("Failed to serialize account auth snapshot: {error}"))?,
        auth_mode: auth_snapshot
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        account_id,
        plan_type: parsed.plan_type,
        last_refresh: auth_snapshot
            .get("last_refresh")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        limit_short_label: usage_snapshot.and_then(|snapshot| snapshot.limit_short_label.clone()),
        limit_5h_text: usage_snapshot.and_then(|snapshot| snapshot.limit_5h_text.clone()),
        limit_weekly_text: usage_snapshot.and_then(|snapshot| snapshot.limit_weekly_text.clone()),
        limit_monthly_text: usage_snapshot.and_then(|snapshot| snapshot.limit_monthly_text.clone()),
        limit_5h_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_5h_reset_at),
        limit_weekly_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_weekly_reset_at),
        limit_monthly_reset_at: usage_snapshot.and_then(|snapshot| snapshot.limit_monthly_reset_at),
        reset_credits_available: usage_snapshot
            .and_then(|snapshot| snapshot.reset_credits_available),
        credits: usage_snapshot.and_then(|snapshot| snapshot.credits.clone()),
        last_limits_fetched_at: usage_snapshot.map(|_| now.clone()),
        last_error: None,
        sort_index: None,
        is_applied: false,
        created_at: now.clone(),
        updated_at: now,
    })
}

async fn save_official_account(
    db: &crate::db::SqliteDbState,
    content: &CodexOfficialAccountContent,
) -> Result<CodexOfficialAccount, String> {
    let account_id = db_new_id();
    let payload = adapter::to_db_value_official_account(content);
    db.with_conn(|conn| db_put(conn, DbTable::CodexOfficialAccount, &account_id, &payload))?;
    load_official_account(db, &account_id).await
}

async fn find_matching_official_account(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
    auth: &Value,
) -> Result<Option<CodexOfficialAccount>, String> {
    let accounts = list_persisted_official_accounts(db, provider_id).await?;
    Ok(accounts
        .into_iter()
        .find(|account| official_account_identity_matches_auth(account, auth)))
}

async fn update_official_account_apply_status(
    db: &crate::db::SqliteDbState,
    account_id: Option<&str>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodexOfficialAccount, account_id, &now)
    })?;

    Ok(())
}

async fn load_official_account(
    db: &crate::db::SqliteDbState,
    account_id: &str,
) -> Result<CodexOfficialAccount, String> {
    db.with_conn(|conn| db_get(conn, DbTable::CodexOfficialAccount, account_id))?
        .map(adapter::from_db_value_official_account)
        .ok_or_else(|| format!("Official account '{}' not found", account_id))
}

async fn persist_usage_snapshot(
    db: &crate::db::SqliteDbState,
    account_id: &str,
    usage_snapshot: &UsageSnapshot,
    last_error: Option<&str>,
    request: Option<&UsageRequestGuard>,
) -> Result<CodexOfficialAccount, String> {
    let now = Local::now().to_rfc3339();
    db.with_conn(|conn| {
        // The generation check and JSONB patch are one synchronous critical
        // section. A newer request cannot start between them.
        let requests = request.map(|_| {
            USAGE_REQUESTS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        });
        if request
            .zip(requests.as_ref())
            .is_some_and(|(request, requests)| !request.is_current(requests))
        {
            return Ok(());
        }
        db_patch_fields(
            conn,
            DbTable::CodexOfficialAccount,
            account_id,
            &[
                (
                    "limit_short_label",
                    usage_snapshot
                        .limit_short_label
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_5h_text",
                    usage_snapshot
                        .limit_5h_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_weekly_text",
                    usage_snapshot
                        .limit_weekly_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_monthly_text",
                    usage_snapshot
                        .limit_monthly_text
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_5h_reset_at",
                    usage_snapshot
                        .limit_5h_reset_at
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_weekly_reset_at",
                    usage_snapshot
                        .limit_weekly_reset_at
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                (
                    "limit_monthly_reset_at",
                    usage_snapshot
                        .limit_monthly_reset_at
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                (
                    "reset_credits_available",
                    usage_snapshot
                        .reset_credits_available
                        .map(|value| serde_json::json!(value))
                        .unwrap_or(Value::Null),
                ),
                (
                    "credits",
                    serde_json::to_value(&usage_snapshot.credits)
                        .map_err(|error| format!("Failed to serialize Codex credits: {error}"))?,
                ),
                ("last_limits_fetched_at", Value::String(now.clone())),
                (
                    "last_error",
                    last_error
                        .map(|value| Value::String(value.to_string()))
                        .unwrap_or(Value::Null),
                ),
                ("updated_at", Value::String(now)),
            ],
        )
        .map(|_| ())
    })?;

    load_official_account(db, account_id).await
}

async fn persist_refreshed_account_snapshot(
    db: &crate::db::SqliteDbState,
    account: &CodexOfficialAccount,
    refreshed_snapshot: &Value,
) -> Result<CodexOfficialAccount, String> {
    let parsed_content = build_account_content_from_auth_snapshot(
        &account.provider_id,
        refreshed_snapshot,
        None,
        Some(&account.name),
    )?;
    let payload = adapter::to_db_value_official_account(&parsed_content);
    db.with_conn(|conn| {
        let fields = [
            "email",
            "auth_snapshot",
            "auth_mode",
            "account_id",
            "plan_type",
            "last_refresh",
            "updated_at",
        ]
        .map(|key| (key, payload.get(key).cloned().unwrap_or(Value::Null)));
        db_patch_fields(conn, DbTable::CodexOfficialAccount, &account.id, &fields).map(|_| ())
    })?;

    load_official_account(db, &account.id).await
}

#[cfg(test)]
async fn persist_usage_result(
    db: &SqliteDbState,
    account_id: &str,
    result: Result<UsageSnapshot, String>,
) -> Result<CodexOfficialAccount, String> {
    persist_usage_result_with_request(db, account_id, result, None).await
}

async fn persist_usage_result_with_request(
    db: &SqliteDbState,
    account_id: &str,
    result: Result<UsageSnapshot, String>,
    request: Option<&UsageRequestGuard>,
) -> Result<CodexOfficialAccount, String> {
    match result {
        Ok(snapshot) => persist_usage_snapshot(db, account_id, &snapshot, None, request).await,
        Err(error) => {
            // A failed read is neither a zero balance nor a successful refresh.
            // Keep the old success timestamp, but do not display cached credits.
            let persisted = db
                .with_conn(|conn| {
                    let requests = request.map(|_| {
                        USAGE_REQUESTS
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                    });
                    if request
                        .zip(requests.as_ref())
                        .is_some_and(|(request, requests)| !request.is_current(requests))
                    {
                        return Ok(false);
                    }
                    db_patch_fields(
                        conn,
                        DbTable::CodexOfficialAccount,
                        account_id,
                        &[
                            ("credits", Value::Null),
                            ("last_error", Value::String(error.clone())),
                            ("updated_at", Value::String(Local::now().to_rfc3339())),
                        ],
                    )
                    .map(|_| true)
                })
                .map_err(|storage_error| {
                    format!("{error}; failed to save usage error: {storage_error}")
                })?;
            if !persisted {
                return load_official_account(db, account_id).await;
            }
            Err(error)
        }
    }
}

fn merge_official_runtime_auth(existing_auth: &Value, next_auth: &Value) -> Value {
    let mut merged = existing_auth.as_object().cloned().unwrap_or_default();
    merged.remove("OPENAI_API_KEY");

    for key in ["auth_mode", "last_refresh", "agent_identity"] {
        if let Some(value) = next_auth.get(key) {
            merged.insert(key.to_string(), value.clone());
        }
    }

    if let Some(next_tokens) = next_auth.get("tokens").and_then(Value::as_object) {
        let mut tokens = existing_auth
            .get("tokens")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in next_tokens {
            tokens.insert(key.clone(), value.clone());
        }
        merged.insert("tokens".to_string(), Value::Object(tokens));
    }

    Value::Object(merged)
}

fn extract_token_from_auth_snapshot(auth: &Value, token_kind: &str) -> Result<String, String> {
    let token_pointer = match token_kind {
        "access" => "/tokens/access_token",
        "refresh" => "/tokens/refresh_token",
        _ => return Err("Unsupported official account token kind".to_string()),
    };
    auth.pointer(token_pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Official account {} token is missing", token_kind))
}

fn copy_text_to_clipboard(value: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Failed to access system clipboard: {error}"))?;
    clipboard
        .set_text(value.to_string())
        .map_err(|error| format!("Failed to copy token to clipboard: {error}"))
}

fn assign_provider_id(
    mut account: CodexOfficialAccount,
    provider_id: &str,
) -> CodexOfficialAccount {
    account.provider_id = provider_id.to_string();
    account
}

pub async fn list_codex_official_accounts_for_provider(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<Vec<CodexOfficialAccount>, String> {
    ensure_persisted_provider_id(provider_id)?;
    let provider = query_provider(db, provider_id).await?;
    let mut accounts = list_persisted_official_accounts(db, provider_id).await?;
    let local_auth = read_auth_json_from_disk(Some(db)).await?;
    if provider.category == "official" && should_show_virtual_local_account(&accounts, &local_auth)
    {
        accounts.push(assign_provider_id(
            build_virtual_local_account(&local_auth),
            provider_id,
        ));
    }
    Ok(accounts)
}

#[tauri::command]
pub async fn list_codex_official_accounts(
    state: tauri::State<'_, SqliteDbState>,
    provider_id: String,
) -> Result<Vec<CodexOfficialAccount>, String> {
    let db = state.db();
    list_codex_official_accounts_for_provider(&db, &provider_id).await
}

#[tauri::command]
pub async fn start_codex_official_account_device_auth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexDeviceAuthStartResult, String> {
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(state.db(), &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can add official accounts".to_string());
    }
    if !DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?
        .is_empty()
    {
        return Err("A Codex device authorization session is already active".to_string());
    }

    let client = crate::http_client::account_client(state.db()).await?;
    let response = client
        .post(CODEX_DEVICE_USER_CODE_URL)
        .json(&serde_json::json!({ "client_id": CODEX_OAUTH_CLIENT_ID }))
        .send()
        .await
        .map_err(|error| format!("Failed to request Codex device code: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Codex device code response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Codex device code request failed ({status}): {body}"
        ));
    }
    let device: CodexDeviceUserCodeResponse = serde_json::from_str(&body)
        .map_err(|error| format!("Failed to parse Codex device code response: {error}"))?;
    if device.device_auth_id.trim().is_empty() || device.user_code.trim().is_empty() {
        return Err("Codex device code response is incomplete".to_string());
    }
    let poll_interval_seconds = match &device.interval {
        Value::Number(value) => value.as_u64().unwrap_or(5),
        Value::String(value) => value.parse::<u64>().unwrap_or(5),
        _ => 5,
    }
    .max(1);
    let session_id = db_new_id();
    let expires_at = chrono::Utc::now().timestamp() + 15 * 60;
    let (cancel_sender, cancel_receiver) = watch::channel(false);
    let mut sessions = DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?;
    // The first check happened before the network request. Recheck while
    // registering so simultaneous starts cannot both become live sessions.
    if !sessions.is_empty() {
        return Err("A Codex device authorization session is already active".to_string());
    }
    sessions.insert(
        session_id.clone(),
        DeviceAuthSession {
            cancel: cancel_sender,
            exchange_started: false,
        },
    );
    drop(sessions);

    emit_codex_auth_status(&app, &session_id, "waiting_for_user", None, None);
    let poll_session_id = session_id.clone();
    let poll_user_code = device.user_code.clone();
    tauri::async_runtime::spawn(async move {
        poll_codex_device_auth(
            app,
            poll_session_id,
            provider_id,
            device.device_auth_id,
            poll_user_code,
            poll_interval_seconds,
            expires_at,
            cancel_receiver,
        )
        .await;
    });

    Ok(CodexDeviceAuthStartResult {
        session_id,
        verification_uri: CODEX_DEVICE_VERIFICATION_URL.to_string(),
        user_code: device.user_code,
        expires_at,
        poll_interval_seconds,
    })
}

#[tauri::command]
pub fn cancel_codex_official_account_device_auth(session_id: String) -> Result<(), String> {
    if let Some(session) = DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?
        .get(&session_id)
    {
        // Once exchange starts, the one-shot code must be saved even if the
        // dialog closes. The worker removes the session at completion.
        if !session.exchange_started {
            let _ = session.cancel.send(true);
        }
    }
    Ok(())
}

fn begin_device_auth_exchange(
    session_id: &str,
    cancelled: &watch::Receiver<bool>,
) -> Result<(), String> {
    let mut sessions = DEVICE_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex device auth session lock is poisoned".to_string())?;
    let Some(session) = sessions.get_mut(session_id) else {
        return Err("cancelled".to_string());
    };
    if *cancelled.borrow() {
        return Err("cancelled".to_string());
    }
    session.exchange_started = true;
    Ok(())
}

async fn poll_codex_device_auth(
    app: tauri::AppHandle,
    session_id: String,
    provider_id: String,
    device_auth_id: String,
    user_code: String,
    poll_interval_seconds: u64,
    expires_at: i64,
    mut cancel_receiver: watch::Receiver<bool>,
) {
    let result = async {
        let db_state = app.state::<SqliteDbState>();
        let db = db_state.db();
        let client = crate::http_client::account_client(db).await?;
        loop {
            if *cancel_receiver.borrow() {
                return Err("cancelled".to_string());
            }
            if chrono::Utc::now().timestamp() >= expires_at {
                return Err("expired".to_string());
            }
            let response = tokio::select! {
                response = client
                    .post(CODEX_DEVICE_TOKEN_URL)
                    .json(&serde_json::json!({
                        "device_auth_id": device_auth_id,
                        "user_code": user_code,
                    }))
                    .send() => response
                    .map_err(|error| format!("Failed to poll Codex device authorization: {error}"))?,
                _ = cancel_receiver.changed() => return Err("cancelled".to_string()),
            };
            let status = response.status();
            let body = tokio::select! {
                body = response.text() => body
                    .map_err(|error| format!("Failed to read Codex device authorization: {error}"))?,
                _ = cancel_receiver.changed() => return Err("cancelled".to_string()),
            };
            if status.is_success() {
                let token: CodexDeviceTokenResponse = serde_json::from_str(&body)
                    .map_err(|error| format!("Invalid Codex device token response: {error}"))?;
                if token.authorization_code.trim().is_empty()
                    || token.code_verifier.trim().is_empty()
                {
                    return Err("Codex device token response is incomplete".to_string());
                }
                begin_device_auth_exchange(&session_id, &cancel_receiver)?;
                emit_codex_auth_status(&app, &session_id, "exchanging", None, None);
                let account = exchange_and_save_official_account(
                    db,
                    &provider_id,
                    &token.authorization_code,
                    CODEX_DEVICE_REDIRECT_URI,
                    &token.code_verifier,
                )
                .await?;
                return Ok(account.id);
            }
            if status.as_u16() != 403 && status.as_u16() != 404 {
                return Err(format!(
                    "Codex device authorization failed ({status}): {body}"
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(poll_interval_seconds)) => {}
                changed = cancel_receiver.changed() => {
                    if changed.is_ok() && *cancel_receiver.borrow() {
                        return Err("cancelled".to_string());
                    }
                }
            }
        }
    }
    .await;

    DEVICE_AUTH_SESSIONS
        .lock()
        .ok()
        .and_then(|mut sessions| sessions.remove(&session_id));
    match result {
        Ok(account_id) => {
            emit_codex_auth_status(&app, &session_id, "authorized", None, Some(account_id));
            let _ = app.emit("codex-official-accounts-changed", &provider_id);
            let _ = app.emit("config-changed", "window");
        }
        Err(error) if error == "cancelled" || error == "expired" => {
            emit_codex_auth_status(&app, &session_id, &error, None, None);
        }
        Err(error) => emit_codex_auth_status(&app, &session_id, "error", Some(error), None),
    }
}

fn emit_codex_auth_status(
    app: &tauri::AppHandle,
    session_id: &str,
    status: &str,
    message: Option<String>,
    account_id: Option<String>,
) {
    let _ = app.emit(
        "codex-auth-status",
        CodexAuthStatusEvent {
            session_id: session_id.to_string(),
            status: status.to_string(),
            message,
            account_id,
        },
    );
}

#[tauri::command]
pub async fn start_codex_official_account_oauth(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexBrowserAuthStartResult, String> {
    let db = state.db().clone();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can add official accounts".to_string());
    }

    let oauth_state = generate_random_urlsafe(32);
    let (code_verifier, code_challenge) = generate_pkce_pair();
    let session_id = db_new_id();
    let (sender, receiver) = watch::channel(false);
    let (oauth_listener, callback_port) = {
        let mut sessions = BROWSER_AUTH_SESSIONS
            .lock()
            .map_err(|_| "Codex browser auth session lock is poisoned".to_string())?;
        if !sessions.is_empty() {
            return Err(
                "A Codex browser login is already active. Cancel it before starting again."
                    .to_string(),
            );
        }
        let listener = bind_oauth_listener()?;
        sessions.insert(session_id.clone(), sender);
        listener
    };
    let redirect_uri = build_oauth_redirect_uri(callback_port);
    let authorize_url = build_codex_authorize_url(&redirect_uri, &oauth_state, &code_challenge);
    let expires_at = chrono::Utc::now().timestamp() + 300;
    let task_session_id = session_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let code = wait_for_oauth_callback_with_timeout(
                oauth_listener,
                &oauth_state,
                Duration::from_secs(300),
                receiver,
            )
            .await?;
            emit_codex_auth_status(&app, &task_session_id, "exchanging", None, None);
            // Once exchange starts it must finish and persist, even if the UI
            // closes: authorization codes are one-shot and refresh tokens rotate.
            exchange_and_save_official_account(
                &db,
                &provider_id,
                &code,
                &redirect_uri,
                &code_verifier,
            )
            .await
        }
        .await;
        if let Ok(mut sessions) = BROWSER_AUTH_SESSIONS.lock() {
            sessions.remove(&task_session_id);
        }
        match result {
            Ok(account) => {
                emit_codex_auth_status(
                    &app,
                    &task_session_id,
                    "authorized",
                    None,
                    Some(account.id.clone()),
                );
                let _ = app.emit("config-changed", "window");
                // Usage is ancillary. Save the login first; a slow quota endpoint
                // must neither delay success nor discard the only fresh token.
                if let Err(error) =
                    refresh_persisted_account_usage(&db, &account.id, Some(&app)).await
                {
                    log::debug!("Codex post-login usage refresh failed: {error}");
                }
                let _ = app.emit("codex-official-accounts-changed", &provider_id);
                let _ = app.emit("config-changed", "window");
            }
            Err(error) if error == "cancelled" || error == "expired" => {
                emit_codex_auth_status(&app, &task_session_id, &error, None, None);
            }
            Err(error) => {
                log::warn!("Codex browser OAuth failed: {error}");
                emit_codex_auth_status(&app, &task_session_id, "error", Some(error), None);
            }
        }
    });
    Ok(CodexBrowserAuthStartResult {
        session_id,
        authorize_url,
        expires_at,
    })
}

async fn exchange_and_save_official_account(
    db: &SqliteDbState,
    provider_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<CodexOfficialAccount, String> {
    let token = exchange_authorization_code(db, code, redirect_uri, verifier).await?;
    let auth = build_auth_snapshot(&token, None)?;
    let content = build_account_content_from_auth_snapshot(provider_id, &auth, None, None)?;
    save_official_account(db, &content).await
}

#[tauri::command]
pub fn cancel_codex_official_account_oauth(session_id: String) -> Result<(), String> {
    if let Some(sender) = BROWSER_AUTH_SESSIONS
        .lock()
        .map_err(|_| "Codex browser auth session lock is poisoned".to_string())?
        .get(&session_id)
    {
        // The worker removes the session after dropping its callback listener.
        let _ = sender.send(true);
    }
    Ok(())
}

#[tauri::command]
pub async fn save_codex_official_local_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can save local official accounts".to_string());
    }

    let local_auth = read_auth_json_from_disk(Some(&db)).await?;
    if !auth_has_official_runtime(&local_auth) {
        return Err("Current local auth.json does not contain an official Codex login".to_string());
    }

    if let Some(existing_account) =
        find_matching_official_account(&db, &provider_id, &local_auth).await?
    {
        if provider.is_applied {
            update_official_account_apply_status(&db, Some(&existing_account.id)).await?;
        }
        let _ = app.emit("config-changed", "window");
        return load_official_account(&db, &existing_account.id).await;
    }

    let usage_plan_type = usage_plan_type_from_auth(&local_auth);
    let usage_account_id = usage_account_id_from_auth(&local_auth);
    let usage_snapshot = local_auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|access_token| async {
            fetch_usage_snapshot(
                &db,
                access_token,
                usage_account_id.as_deref(),
                usage_plan_type.as_deref(),
            )
            .await
        });

    let usage_snapshot = match usage_snapshot {
        Some(request) => request.await.ok(),
        None => None,
    };

    let mut content = build_account_content_from_auth_snapshot(
        &provider_id,
        &local_auth,
        usage_snapshot.as_ref(),
        None,
    )?;
    content.is_applied = provider.is_applied;

    let account = save_official_account(&db, &content).await?;
    if provider.is_applied {
        update_official_account_apply_status(&db, Some(&account.id)).await?;
    }

    let _ = app.emit("config-changed", "window");
    load_official_account(&db, &account.id).await
}

#[tauri::command]
pub async fn import_codex_official_account_auth_json(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    auth_json: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can import official accounts".to_string());
    }

    let auth = parse_imported_auth_json(&auth_json)?;
    if let Some(existing_account) = find_matching_official_account(&db, &provider_id, &auth).await?
    {
        let _ = app.emit("config-changed", "window");
        return load_official_account(&db, &existing_account.id).await;
    }

    let usage_plan_type = usage_plan_type_from_auth(&auth);
    let usage_account_id = usage_account_id_from_auth(&auth);
    let usage_snapshot = auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|access_token| async {
            fetch_usage_snapshot(
                &db,
                access_token,
                usage_account_id.as_deref(),
                usage_plan_type.as_deref(),
            )
            .await
        });
    let usage_snapshot = match usage_snapshot {
        Some(request) => request.await.ok(),
        None => None,
    };

    let content = build_account_content_from_auth_snapshot(
        &provider_id,
        &auth,
        usage_snapshot.as_ref(),
        None,
    )?;
    let account = save_official_account(&db, &content).await?;
    let _ = app.emit("config-changed", "window");
    Ok(account)
}

#[tauri::command]
pub async fn apply_codex_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can apply official accounts".to_string());
    }
    if provider.is_disabled {
        return Err(format!(
            "Provider '{}' is disabled and cannot be applied",
            provider_id
        ));
    }

    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_auth_json_from_disk(Some(&db)).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current local auth.json does not contain an official Codex login".to_string(),
            );
        }
        let refreshed_auth = ensure_fresh_official_runtime_auth(&db, &local_auth).await?;
        let merged_auth = merge_official_runtime_auth(&local_auth, &refreshed_auth);
        apply_config_internal(&db, &app, &provider_id, false).await?;
        write_auth_json_to_disk(&db, &merged_auth).await?;
        update_official_account_apply_status(&db, None).await?;
    } else {
        let account = load_official_account(&db, &account_id).await?;
        if account.provider_id != provider_id {
            return Err("Official account does not belong to the selected provider".to_string());
        }
        let current_auth = read_auth_json_from_disk(Some(&db)).await?;
        let snapshot = account
            .auth_snapshot
            .as_deref()
            .ok_or_else(|| "Official account snapshot is missing".to_string())?;
        let snapshot_auth = auth_json_from_snapshot(snapshot)?;
        let refreshed_snapshot = ensure_fresh_official_runtime_auth(&db, &snapshot_auth).await?;
        if refreshed_snapshot != snapshot_auth {
            let _ = persist_refreshed_account_snapshot(&db, &account, &refreshed_snapshot).await?;
        }
        let merged_auth = merge_official_runtime_auth(&current_auth, &refreshed_snapshot);
        apply_config_internal(&db, &app, &provider_id, false).await?;
        write_auth_json_to_disk(&db, &merged_auth).await?;
        update_official_account_apply_status(&db, Some(&account_id)).await?;
    }

    let _ = app.emit("config-changed", "window");
    Ok(())
}

#[tauri::command]
pub async fn delete_codex_official_account(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        return Err("The local official account cannot be deleted".to_string());
    }

    let account = load_official_account(&db, &account_id).await?;
    if account.provider_id != provider_id {
        return Err("Official account does not belong to the selected provider".to_string());
    }
    if account.is_applied {
        return Err("The applied official account cannot be deleted".to_string());
    }

    db.with_conn(|conn| db_delete(conn, DbTable::CodexOfficialAccount, &account_id).map(|_| ()))?;

    let _ = app.emit("config-changed", "window");
    Ok(())
}

/// Background / startup pass: ensure_fresh for every persisted Codex official account.
///
/// Non-applied accounts still refresh OAuth into SQLite so login-only tokens do not die.
/// Live `auth.json` is rewritten only for applied accounts.
pub async fn refresh_applied_codex_accounts_if_needed(
    db: &SqliteDbState,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let accounts = db.with_conn(|conn| {
        Ok(db_list(conn, DbTable::CodexOfficialAccount, None)?
            .into_iter()
            .map(adapter::from_db_value_official_account)
            .filter(|account| account.id != LOCAL_OFFICIAL_ACCOUNT_ID)
            .collect::<Vec<_>>())
    })?;

    let mut wrote_runtime = false;
    for account in accounts {
        // The same preparation path a manual refresh uses: adopt a newer
        // same-account runtime credential first, then exchange only when the
        // chosen credential is inside the refresh window. Without this the pass
        // would spend an older (possibly already rotated) refresh token and
        // leave the database permanently behind the runtime.
        match prepare_persisted_account_credential(db, &account.id).await {
            Ok(prepared) => wrote_runtime |= prepared.runtime_written,
            Err(error) => {
                log::debug!(
                    "Codex official account '{}' refresh failed: {error}",
                    account.id
                );
            }
        }
    }

    if wrote_runtime {
        let _ = app.emit("config-changed", "window");
        // Align with apply_config_internal: host auth.json change should sync to WSL.
        #[cfg(target_os = "windows")]
        let _ = app.emit("wsl-sync-request-codex", ());
    }
    Ok(())
}

#[tauri::command]
pub async fn refresh_codex_official_account_limits(
    state: tauri::State<'_, SqliteDbState>,
    app: tauri::AppHandle,
    provider_id: String,
    account_id: String,
) -> Result<CodexOfficialAccount, String> {
    let db = state.db();
    ensure_persisted_provider_id(&provider_id)?;
    let provider = query_provider(&db, &provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can refresh official account usage".to_string());
    }

    if account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let prepared = refresh_local_account_usage(&db, Some(&app)).await?;
        // Built from the credential that answered the request: re-reading the
        // file could pick up a login the user switched to mid-refresh.
        let mut account = assign_provider_id(
            build_virtual_local_account(&prepared.credential),
            &provider_id,
        );
        apply_usage_snapshot_to_account(&mut account, &prepared.usage);
        account.last_limits_fetched_at = Some(Local::now().to_rfc3339());
        return Ok(account);
    }

    let account = load_official_account(&db, &account_id).await?;
    if account.provider_id != provider_id {
        return Err("Official account does not belong to the selected provider".to_string());
    }
    let (refreshed, _) = refresh_persisted_account_usage(&db, &account.id, Some(&app)).await?;
    Ok(refreshed)
}

/// The live `auth.json` now holds a rotated credential, so the same consumers
/// as apply need to be told (tray + WSL sync target).
fn emit_runtime_credential_changed(app: &tauri::AppHandle) {
    let _ = app.emit("config-changed", "window");
    #[cfg(target_os = "windows")]
    let _ = app.emit("wsl-sync-request-codex", ());
}

fn apply_usage_snapshot_to_account(account: &mut CodexOfficialAccount, usage: &UsageSnapshot) {
    account.limit_short_label = usage.limit_short_label.clone();
    account.limit_5h_text = usage.limit_5h_text.clone();
    account.limit_weekly_text = usage.limit_weekly_text.clone();
    account.limit_monthly_text = usage.limit_monthly_text.clone();
    account.limit_5h_reset_at = usage.limit_5h_reset_at;
    account.limit_weekly_reset_at = usage.limit_weekly_reset_at;
    account.limit_monthly_reset_at = usage.limit_monthly_reset_at;
    account.reset_credits_available = usage.reset_credits_available;
    account.credits = usage.credits.clone();
}

/// What a `__local__` refresh produced: the credential that answered and its usage.
struct LocalUsageResult {
    credential: Value,
    usage: UsageSnapshot,
}

/// Usage for the `__local__` virtual account. The runtime credential is rotated
/// and written back like any applied account: it *is* the login.
async fn refresh_local_account_usage(
    db: &SqliteDbState,
    app: Option<&tauri::AppHandle>,
) -> Result<LocalUsageResult, String> {
    let auth = read_auth_json_from_disk(Some(db)).await?;
    if !auth_has_official_runtime(&auth) {
        return Err("Current local auth.json does not contain an official Codex login".to_string());
    }
    let (mut credential, runtime_written) = prepare_local_credential(db, &auth, false).await?;
    if runtime_written {
        if let Some(app) = app {
            emit_runtime_credential_changed(app);
        }
    }
    match request_usage_from_local(db, &credential).await {
        Ok(usage) => Ok(LocalUsageResult { credential, usage }),
        // A 401 on an unexpired token still deserves one forced exchange: the
        // server may have revoked it early. Any other status keeps its meaning.
        Err(error) if error.is_unauthorized() => {
            let (next, forced_written) = prepare_local_credential(db, &credential, true).await?;
            if forced_written {
                if let Some(app) = app {
                    emit_runtime_credential_changed(app);
                }
            }
            credential = next;
            request_usage_from_local(db, &credential)
                .await
                .map(|usage| LocalUsageResult { credential, usage })
                .map_err(|error| {
                    if error.is_unauthorized() {
                        codex_reauth_error("the usage endpoint keeps rejecting the credential")
                    } else {
                        error.message
                    }
                })
        }
        Err(error) => Err(error.message),
    }
}

async fn request_usage_from_local(
    db: &SqliteDbState,
    credential: &Value,
) -> Result<UsageSnapshot, UsageRequestError> {
    let access_token = access_token_from_auth(credential).map_err(|message| UsageRequestError {
        status: None,
        message,
    })?;
    request_usage_snapshot(
        db,
        access_token,
        usage_account_id_from_auth(credential).as_deref(),
        usage_plan_type_from_auth(credential).as_deref(),
    )
    .await
}

/// Local runtime credential, refreshed when due (or forced after a 401) and
/// written back before any usage request. Returns it with the write flag.
async fn prepare_local_credential(
    db: &SqliteDbState,
    auth: &Value,
    force: bool,
) -> Result<(Value, bool), String> {
    if !force && !official_auth_needs_refresh(auth) {
        return Ok((auth.clone(), false));
    }
    let _refresh_guard = OAUTH_REFRESH_LOCK.lock().await;
    let latest = read_auth_json_from_disk(Some(db)).await?;
    // Re-read under the lock so a concurrent rotation is not consumed twice.
    if !auth_has_official_runtime(&latest) || !auth_identities_match(auth, &latest) {
        // The user switched the runtime login while we were working. Rotating
        // the old account's refresh token now would invalidate it for nothing
        // and the answer would belong to a row that is no longer on screen.
        return Err("The local Codex login changed during the refresh; retry".to_string());
    }
    if force && credential_changed_since_attempt(&latest, auth) {
        return Ok((latest, false));
    }
    let next = exchange_official_auth_locked(db, &latest, force).await?;
    let written = if next != latest {
        write_local_runtime_credential(db, &latest, &next).await?
    } else {
        false
    };
    Ok((next, written))
}

fn access_token_from_auth(auth: &Value) -> Result<&str, String> {
    auth.pointer("/tokens/access_token")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Official account is missing access token".to_string())
}

/// One manual refresh for a stored account.
///
/// Credential preparation runs first, so the request never carries the stale
/// SQLite token while the runtime already holds a newer one. A 401 gets exactly
/// one recovery attempt (adopt a newer same-account credential, otherwise force
/// one OAuth exchange) and then one retry — never a loop.
/// Returns the refreshed row and whether the runtime `auth.json` was rewritten
/// (the caller then emits the same sync events as apply).
async fn refresh_persisted_account_usage(
    db: &SqliteDbState,
    account_id: &str,
    app: Option<&tauri::AppHandle>,
) -> Result<(CodexOfficialAccount, bool), String> {
    let request = UsageRequestGuard::begin(account_id);
    // Preparation errors invalidate cached credits just like HTTP errors do.
    let (result, runtime_written) = refresh_account_usage_inner(db, account_id).await;
    if runtime_written {
        if let Some(app) = app {
            emit_runtime_credential_changed(app);
        }
    }
    let account = persist_usage_result_with_request(db, account_id, result, Some(&request)).await?;
    Ok((account, runtime_written))
}

async fn refresh_account_usage_inner(
    db: &SqliteDbState,
    account_id: &str,
) -> (Result<UsageSnapshot, String>, bool) {
    let prepared = match prepare_persisted_account_credential(db, account_id).await {
        Ok(prepared) => prepared,
        Err(error) => return (Err(error), false),
    };
    let mut runtime_written = prepared.runtime_written;
    let result = match request_usage_from_prepared(db, &prepared).await {
        Ok(snapshot) => Ok(snapshot),
        Err(error) if error.is_unauthorized() => {
            match recover_persisted_account_credential(db, account_id, &prepared.auth).await {
                Ok(recovered) => {
                    runtime_written |= recovered.runtime_written;
                    request_usage_from_prepared(db, &recovered)
                        .await
                        .map_err(|retry| {
                            if retry.is_unauthorized() {
                                codex_reauth_error(
                                    "the usage endpoint keeps rejecting the credential",
                                )
                            } else {
                                retry.message
                            }
                        })
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error.message),
    };
    (result, runtime_written)
}

async fn request_usage_from_prepared(
    db: &SqliteDbState,
    prepared: &PreparedOfficialCredential,
) -> Result<UsageSnapshot, UsageRequestError> {
    // Account id and plan type come from the credential actually used, so a
    // recovered token never travels with the previous login's context.
    let usage_account_id =
        usage_account_id_from_auth(&prepared.auth).or_else(|| prepared.account.account_id.clone());
    let plan_type =
        usage_plan_type_from_auth(&prepared.auth).or_else(|| prepared.account.plan_type.clone());
    let access_token =
        access_token_from_auth(&prepared.auth).map_err(|message| UsageRequestError {
            status: None,
            message,
        })?;
    request_usage_snapshot(
        db,
        access_token,
        usage_account_id.as_deref(),
        plan_type.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn copy_codex_official_account_token(
    state: tauri::State<'_, SqliteDbState>,
    input: CodexOfficialAccountTokenCopyInput,
) -> Result<(), String> {
    let db = state.db();
    ensure_persisted_provider_id(&input.provider_id)?;
    let provider = query_provider(&db, &input.provider_id).await?;
    if provider.category != "official" {
        return Err("Only official Codex providers can copy official account tokens".to_string());
    }

    let auth = if input.account_id == LOCAL_OFFICIAL_ACCOUNT_ID {
        let local_auth = read_auth_json_from_disk(Some(&db)).await?;
        if !auth_has_official_runtime(&local_auth) {
            return Err(
                "Current local auth.json does not contain an official Codex login".to_string(),
            );
        }
        local_auth
    } else {
        let account = load_official_account(&db, &input.account_id).await?;
        if account.provider_id != input.provider_id {
            return Err("Official account does not belong to the selected provider".to_string());
        }
        let snapshot = account
            .auth_snapshot
            .as_deref()
            .ok_or_else(|| "Official account snapshot is missing".to_string())?;
        auth_json_from_snapshot(snapshot)?
    };

    let token = extract_token_from_auth_snapshot(&auth, input.token_kind.trim())?;
    copy_text_to_clipboard(&token)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_credits_parse_strings_numbers_and_preserve_subscription_windows() {
        for balance in [serde_json::json!("62500.25"), serde_json::json!(62500.25)] {
            let snapshot = parse_usage_snapshot(
                &serde_json::json!({
                    "rate_limit": {"primary_window": {
                        "used_percent": 25, "limit_window_seconds": WEEK_WINDOW_SECONDS
                    }},
                    "rate_limit_reset_credits": {"available_count": 2},
                    "credits": {"has_credits": true, "unlimited": false, "balance": balance}
                }),
                Some("pro"),
            );
            assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
            assert_eq!(snapshot.reset_credits_available, Some(2));
            assert_eq!(
                snapshot.credits,
                Some(CodexCredits {
                    has_credits: Some(true),
                    unlimited: Some(false),
                    balance: Some(62500.25)
                })
            );
        }
    }

    #[test]
    fn codex_credits_preserve_zero_negative_and_conflicting_availability() {
        for balance in ["0", "-0.25", " 12.5 "] {
            for has_credits in [false, true] {
                let snapshot = parse_usage_snapshot(
                    &serde_json::json!({
                        "credits": {"has_credits": has_credits, "unlimited": true, "balance": balance}
                    }),
                    None,
                );
                let credits = snapshot.credits.expect("credits without rate windows");
                assert_eq!(
                    credits.balance,
                    Some(balance.trim().parse::<f64>().unwrap())
                );
                assert_eq!(credits.has_credits, Some(has_credits));
                assert_eq!(credits.unlimited, Some(true));
            }
        }
    }

    #[test]
    fn codex_credits_malformed_fields_do_not_fail_or_invent_a_balance() {
        for body in [
            serde_json::json!({}),
            serde_json::json!({"credits": null}),
            serde_json::json!({"credits": []}),
            serde_json::json!({"credits": "bad"}),
            serde_json::json!({"credits": {"has_credits": "true", "unlimited": 1}}),
        ] {
            assert_eq!(parse_usage_snapshot(&body, None).credits, None);
        }
        for balance in [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!({}),
            serde_json::json!(""),
            serde_json::json!("NaN"),
            serde_json::json!("Infinity"),
            serde_json::json!("1e999"),
            serde_json::json!("not-a-number"),
        ] {
            let snapshot = parse_usage_snapshot(
                &serde_json::json!({
                    "credits": {"has_credits": true, "balance": balance},
                    "rate_limit": {"primary_window": {
                        "used_percent": 25, "limit_window_seconds": WEEK_WINDOW_SECONDS
                    }}
                }),
                Some("pro"),
            );
            assert_eq!(snapshot.credits.unwrap().balance, None);
            assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
        }
    }

    #[tokio::test]
    async fn codex_credits_round_trip_refresh_clear_and_failure_are_account_scoped() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let auth = serde_json::json!({"tokens": {"account_id": "synthetic-account"}});
        let snapshot = parse_usage_snapshot(
            &serde_json::json!({
                "credits": {"has_credits": true, "unlimited": false, "balance": "42.5"},
                "rate_limit": {"primary_window": {
                    "used_percent": 25, "limit_window_seconds": WEEK_WINDOW_SECONDS
                }}
            }),
            Some("pro"),
        );
        let content =
            build_account_content_from_auth_snapshot("provider-a", &auth, Some(&snapshot), None)
                .unwrap();
        let first = save_official_account(&db, &content).await.unwrap();
        let second = save_official_account(&db, &content).await.unwrap();
        assert_eq!(first.credits.as_ref().unwrap().balance, Some(42.5));
        let wire = serde_json::to_value(&first).unwrap();
        assert_eq!(wire["credits"]["hasCredits"], true);
        assert_eq!(wire["credits"]["balance"], 42.5);
        assert!(wire.get("authSnapshot").is_none());

        // The row keeps snake_case keys; the wire keeps camelCase.
        let stored = db
            .with_conn(|conn| db_get(conn, DbTable::CodexOfficialAccount, &first.id))
            .unwrap()
            .unwrap();
        assert_eq!(stored["credits"]["has_credits"], true);
        assert!(stored["credits"].get("hasCredits").is_none());

        let refreshed = persist_refreshed_account_snapshot(&db, &first, &auth)
            .await
            .unwrap();
        assert_eq!(refreshed.credits, first.credits);
        assert_eq!(
            refreshed.last_limits_fetched_at,
            first.last_limits_fetched_at
        );

        let error =
            persist_usage_result(&db, &first.id, Err("synthetic usage failure".to_string())).await;
        assert_eq!(error.unwrap_err(), "synthetic usage failure");
        let failed = load_official_account(&db, &first.id).await.unwrap();
        assert_eq!(failed.credits, None);
        assert_eq!(failed.last_limits_fetched_at, first.last_limits_fetched_at);
        assert_eq!(failed.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(
            failed.last_error.as_deref(),
            Some("synthetic usage failure")
        );
        assert_eq!(
            load_official_account(&db, &second.id)
                .await
                .unwrap()
                .credits,
            second.credits
        );

        let recovered = persist_usage_result(&db, &first.id, Ok(snapshot))
            .await
            .unwrap();
        assert_eq!(recovered.credits, first.credits);
        assert_eq!(recovered.last_error, None);
        let omitted = parse_usage_snapshot(&serde_json::json!({}), None);
        let cleared = persist_usage_result(&db, &first.id, Ok(omitted))
            .await
            .unwrap();
        assert_eq!(cleared.credits, None);
        assert_eq!(
            load_official_account(&db, &first.id).await.unwrap().credits,
            None
        );
    }

    #[test]
    fn codex_credits_old_records_and_virtual_logins_have_no_invented_balance() {
        let old = adapter::from_db_value_official_account(serde_json::json!({
            "id": "legacy-account", "auth_snapshot": "{}", "limit_weekly_text": "75%"
        }));
        assert_eq!(old.credits, None);
        assert_eq!(old.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(
            build_virtual_local_account(&serde_json::json!({})).credits,
            None
        );
    }

    #[test]
    fn codex_credits_accept_the_same_wrappers_as_reset_credits() {
        let expected = Some(CodexCredits {
            has_credits: Some(true),
            unlimited: Some(false),
            balance: Some(42.5),
        });
        for body in [
            serde_json::json!({"credits": {"has_credits": true, "unlimited": false, "balance": 42.5}}),
            serde_json::json!({"credits": {"hasCredits": true, "unlimited": false, "balance": 42.5}}),
            serde_json::json!({"data": {"credits": {"has_credits": true, "unlimited": false, "balance": 42.5}}}),
        ] {
            assert_eq!(parse_usage_snapshot(&body, None).credits, expected);
        }
    }

    #[test]
    fn codex_credits_read_both_stored_spellings() {
        let read = |credits: serde_json::Value| {
            adapter::from_db_value_official_account(serde_json::json!({
                "id": "account", "auth_snapshot": "{}", "credits": credits
            }))
            .credits
        };
        // Rows written by the current build use snake_case.
        assert_eq!(
            read(serde_json::json!({
                "has_credits": true, "unlimited": false, "balance": 42.5
            })),
            Some(CodexCredits {
                has_credits: Some(true),
                unlimited: Some(false),
                balance: Some(42.5)
            })
        );
        // Rows written by the first build of this field stay readable.
        assert_eq!(
            read(serde_json::json!({
                "hasCredits": true, "unlimited": false, "balance": 42.5
            })),
            read(serde_json::json!({
                "has_credits": true, "unlimited": false, "balance": 42.5
            }))
        );
        assert_eq!(read(serde_json::json!(null)), None);
        assert_eq!(read(serde_json::json!({"balance": "nope"})), None);
    }

    #[tokio::test]
    async fn codex_credits_preparation_failure_clears_persisted_balance() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let auth = serde_json::json!({"tokens": {"account_id": "synthetic-account"}});
        let snapshot = parse_usage_snapshot(
            &serde_json::json!({
                "credits": {"has_credits": true, "balance": "42"}
            }),
            None,
        );
        for auth_snapshot in [
            None,
            Some("not-json"),
            Some("{}"),
            Some(r#"{"tokens":{"access_token":" "}}"#),
        ] {
            let content = build_account_content_from_auth_snapshot(
                "provider-a",
                &auth,
                Some(&snapshot),
                None,
            )
            .unwrap();
            let account = save_official_account(&db, &content).await.unwrap();
            // The refresh path re-reads the row, so the bad snapshot must be stored.
            db.with_conn(|conn| {
                db_patch_fields(
                    conn,
                    DbTable::CodexOfficialAccount,
                    &account.id,
                    &[(
                        "auth_snapshot",
                        auth_snapshot
                            .map(|value| Value::String(value.to_string()))
                            .unwrap_or(Value::Null),
                    )],
                )
                .map(|_| ())
            })
            .unwrap();
            assert!(refresh_persisted_account_usage(&db, &account.id, None)
                .await
                .is_err());
            let stored = load_official_account(&db, &account.id).await.unwrap();
            assert_eq!(stored.credits, None);
            assert!(stored.last_error.is_some());
            assert_eq!(
                stored.last_limits_fetched_at,
                account.last_limits_fetched_at
            );
        }
    }

    #[tokio::test]
    async fn codex_credits_oauth_refresh_does_not_restore_an_older_usage_snapshot() {
        let db = SqliteDbState::in_memory_for_test().unwrap();
        let auth = serde_json::json!({"tokens": {"account_id": "synthetic-account"}});
        let old_usage = parse_usage_snapshot(
            &serde_json::json!({
                "credits": {"has_credits": true, "balance": "42"}
            }),
            None,
        );
        let content =
            build_account_content_from_auth_snapshot("provider-a", &auth, Some(&old_usage), None)
                .unwrap();
        let old_account = save_official_account(&db, &content).await.unwrap();
        let new_usage = parse_usage_snapshot(
            &serde_json::json!({
                "credits": {"has_credits": false, "balance": "-1.5"}
            }),
            None,
        );
        let latest = persist_usage_result(&db, &old_account.id, Ok(new_usage))
            .await
            .unwrap();
        let refreshed = persist_refreshed_account_snapshot(&db, &old_account, &auth)
            .await
            .unwrap();
        assert_eq!(refreshed.credits, latest.credits);
        assert_eq!(
            refreshed.last_limits_fetched_at,
            latest.last_limits_fetched_at
        );
        assert_eq!(refreshed.last_error, latest.last_error);
    }

    fn test_id_token(account_id: &str) -> String {
        let payload = serde_json::json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id
            }
        });
        format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).expect("serialize JWT payload"))
        )
    }

    #[test]
    fn auth_has_official_runtime_requires_auth_mode_and_both_tokens() {
        assert!(auth_has_official_runtime(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token"
            }
        })));

        assert!(!auth_has_official_runtime(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token"
            }
        })));
        assert!(!auth_has_official_runtime(&serde_json::json!({
            "OPENAI_API_KEY": "sk-test",
            "auth_mode": "apikey"
        })));
        assert!(!auth_has_official_runtime(&serde_json::json!({})));
    }

    #[test]
    fn imported_auth_json_requires_a_valid_official_login_object() {
        let valid = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token",
                "refresh_token": "refresh-token"
            }
        })
        .to_string();

        let parsed = parse_imported_auth_json(&format!("\n {valid} \n"))
            .expect("valid auth.json should parse");
        assert_eq!(parsed["auth_mode"], "chatgpt");
        assert!(parse_imported_auth_json("").is_err());
        assert!(parse_imported_auth_json("[]").is_err());
        assert!(parse_imported_auth_json("not-json").is_err());
        assert!(parse_imported_auth_json(
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"only"}}"#
        )
        .is_err());
    }

    #[test]
    fn refreshed_auth_preserves_rotated_fields_omitted_by_server() {
        let existing_id_token = test_id_token("account-1");
        let existing = serde_json::json!({
            "auth_mode": "chatgpt",
            "runtime_owned": true,
            "tokens": {
                "id_token": existing_id_token,
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "account_id": "account-1"
            }
        });
        let response = OAuthTokenResponse {
            access_token: "new-access".to_string(),
            refresh_token: None,
            id_token: None,
        };

        let merged = build_auth_snapshot(&response, Some(&existing)).expect("build auth snapshot");

        assert_eq!(
            merged
                .pointer("/tokens/access_token")
                .and_then(Value::as_str),
            Some("new-access")
        );
        assert_eq!(
            merged
                .pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("old-refresh")
        );
        assert_eq!(
            merged.pointer("/tokens/id_token").and_then(Value::as_str),
            Some(existing_id_token.as_str())
        );
        assert_eq!(
            merged.get("runtime_owned").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn browser_oauth_authorize_url_uses_actual_callback_port_and_official_scopes() {
        let redirect_uri = build_oauth_redirect_uri(CODEX_OAUTH_FALLBACK_PORT);
        let authorize_url = build_codex_authorize_url(&redirect_uri, "state-value", "challenge");

        assert_eq!(redirect_uri, "http://localhost:1457/auth/callback");
        assert!(
            authorize_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1457%2Fauth%2Fcallback")
        );
        assert!(authorize_url.contains(
            "scope=openid%20profile%20email%20offline_access%20api.connectors.read%20api.connectors.invoke"
        ));
        assert!(authorize_url.contains("code_challenge_method=S256"));
        assert!(authorize_url.contains("codex_cli_simplified_flow=true"));
        assert!(authorize_url.contains("originator=codex_cli_rs"));
    }

    #[test]
    fn browser_oauth_falls_back_when_default_port_is_occupied() {
        let occupied_listener = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_DEFAULT_PORT)).ok();
        if let Ok((_callback_listener, callback_port)) = bind_oauth_listener() {
            // Even when an external login owns the preferred port, only the
            // registered fallback may be sent to OpenAI. Never use port zero.
            assert_eq!(callback_port, CODEX_OAUTH_FALLBACK_PORT);
        }
        let _fallback = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_FALLBACK_PORT)).ok();
        assert!(bind_oauth_listener().unwrap_err().contains("1455 and 1457"));
        drop(occupied_listener);
    }

    #[tokio::test]
    async fn browser_oauth_callback_ignores_stray_requests_and_invalid_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind callback listener");
        let address = listener.local_addr().expect("callback address");
        let (_cancel_sender, cancel_receiver) = watch::channel(false);
        let task = tokio::spawn(wait_for_oauth_callback_with_timeout(
            listener,
            "expected-state",
            Duration::from_secs(5),
            cancel_receiver,
        ));
        for path in [
            "/favicon.ico",
            "/unrelated?code=bad&state=expected-state",
            "/auth/callback?code=bad&state=wrong-state",
        ] {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.starts_with("HTTP/1.1 400"));
            assert!(!task.is_finished());
        }
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"GET /auth/callback?code=auth-code&state=expected-state HTTP/1.1\r\n")
            .await
            .unwrap();
        tokio::task::yield_now().await;
        stream.write_all(b"Host: localhost\r\n\r\n").await.unwrap();
        assert_eq!(task.await.unwrap().unwrap(), "auth-code");
    }

    #[tokio::test]
    async fn browser_oauth_callback_times_out_without_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind callback listener");
        let (_sender, receiver) = watch::channel(false);
        let error = wait_for_oauth_callback_with_timeout(
            listener,
            "expected-state",
            Duration::from_millis(10),
            receiver,
        )
        .await
        .expect_err("callback must time out");
        assert_eq!(error, "expired");
    }

    #[tokio::test]
    async fn browser_oauth_cancel_releases_listener_even_with_an_idle_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = watch::channel(false);
        let session = format!("test-browser-{}", uuid::Uuid::new_v4());
        BROWSER_AUTH_SESSIONS
            .lock()
            .unwrap()
            .insert(session.clone(), sender);
        let task = tokio::spawn(wait_for_oauth_callback_with_timeout(
            listener,
            "state",
            Duration::from_secs(300),
            receiver,
        ));
        let _idle = tokio::net::TcpStream::connect(address).await.unwrap();
        cancel_codex_official_account_oauth(session.clone()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "cancelled");
        BROWSER_AUTH_SESSIONS.lock().unwrap().remove(&session);
        assert!(
            TcpListener::bind(address).is_ok(),
            "cancel must release the callback port"
        );
    }

    #[test]
    fn device_auth_cancel_signals_without_releasing_active_slot() {
        let session_id = format!("test-device-session-{}", uuid::Uuid::new_v4());
        let (cancel_sender, cancel_receiver) = watch::channel(false);
        DEVICE_AUTH_SESSIONS
            .lock()
            .expect("device session lock")
            .insert(
                session_id.clone(),
                DeviceAuthSession {
                    cancel: cancel_sender,
                    exchange_started: false,
                },
            );

        cancel_codex_official_account_device_auth(session_id.clone()).expect("cancel device auth");

        assert!(*cancel_receiver.borrow());
        assert!(DEVICE_AUTH_SESSIONS
            .lock()
            .expect("device session lock")
            .contains_key(&session_id));
        DEVICE_AUTH_SESSIONS.lock().unwrap().remove(&session_id);
    }

    #[test]
    fn device_auth_cancel_before_exchange_cannot_consume_poll_code() {
        let session_id = format!("test-device-session-{}", uuid::Uuid::new_v4());
        let (cancel_sender, cancel_receiver) = watch::channel(false);
        DEVICE_AUTH_SESSIONS.lock().unwrap().insert(
            session_id.clone(),
            DeviceAuthSession {
                cancel: cancel_sender,
                exchange_started: false,
            },
        );
        cancel_codex_official_account_device_auth(session_id.clone()).unwrap();
        assert_eq!(
            begin_device_auth_exchange(&session_id, &cancel_receiver),
            Err("cancelled".to_string())
        );
        DEVICE_AUTH_SESSIONS.lock().unwrap().remove(&session_id);

        let (cancel_sender, cancel_receiver) = watch::channel(false);
        DEVICE_AUTH_SESSIONS.lock().unwrap().insert(
            session_id.clone(),
            DeviceAuthSession {
                cancel: cancel_sender,
                exchange_started: false,
            },
        );
        begin_device_auth_exchange(&session_id, &cancel_receiver).unwrap();
        cancel_codex_official_account_device_auth(session_id.clone()).unwrap();
        assert!(
            !*cancel_receiver.borrow(),
            "closing after exchange starts keeps the one-shot code"
        );
        DEVICE_AUTH_SESSIONS.lock().unwrap().remove(&session_id);
    }

    #[test]
    fn official_runtime_merge_preserves_unknown_runtime_fields() {
        let existing = serde_json::json!({
            "OPENAI_API_KEY": "remove-me",
            "auth_mode": "chatgpt",
            "last_refresh": "runtime-refresh",
            "runtime_root": { "keep": true },
            "tokens": {
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "runtime_token_metadata": { "keep": true }
            }
        });
        let next = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "id_token": "new-id"
            }
        });

        let merged = merge_official_runtime_auth(&existing, &next);
        assert!(merged.get("OPENAI_API_KEY").is_none());
        assert_eq!(merged["runtime_root"]["keep"], true);
        assert_eq!(merged["last_refresh"], "runtime-refresh");
        assert_eq!(merged["tokens"]["access_token"], "new-access");
        assert_eq!(merged["tokens"]["runtime_token_metadata"]["keep"], true);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_auth_writer_replaces_content_and_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let auth_path = temp_dir.path().join("auth.json");
        fs::write(&auth_path, "{\"old\":true}\n").expect("write old auth");

        write_auth_json_atomic(
            &auth_path,
            &serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": { "access_token": "new-access" },
                "runtime_owned": true
            }),
        )
        .expect("write auth atomically");

        let written: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).expect("read written auth"))
                .expect("parse written auth");
        assert_eq!(written["runtime_owned"], true);
        assert!(written.get("old").is_none());
        assert_eq!(
            fs::metadata(&auth_path)
                .expect("auth metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn parse_usage_snapshot_free_treats_primary_week_window_as_weekly_limit() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 25.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 12345
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("free"));

        assert_eq!(snapshot.limit_short_label, None);
        assert_eq!(snapshot.limit_5h_text, None);
        assert_eq!(snapshot.limit_5h_reset_at, None);
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(12345));
        assert_eq!(snapshot.limit_monthly_text, None);
        assert_eq!(snapshot.limit_monthly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_free_falls_back_to_primary_window_as_weekly_limit() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "remaining_count": 3.0,
                    "total_count": 4.0,
                    "reset_at": 67890
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("free"));

        assert_eq!(snapshot.limit_short_label, None);
        assert_eq!(snapshot.limit_5h_text, None);
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(67890));
        assert_eq!(snapshot.limit_monthly_text, None);
        assert_eq!(snapshot.limit_monthly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_pro_family_has_no_five_hour_limit() {
        let mut body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 25, "limit_window_seconds": WEEK_WINDOW_SECONDS, "reset_at": 12345
                },
                "secondary_window": {
                    "used_percent": 40, "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS, "reset_at": 23456
                }
            }
        });
        // Native usage source recognizes both spellings of Pro Lite.
        for plan in ["pro", "prolite", "pro_lite", " PRO "] {
            let snapshot = parse_usage_snapshot(&body, Some(plan));
            assert_eq!(snapshot.limit_short_label, None);
            assert_eq!(snapshot.limit_5h_text, None);
            assert_eq!(snapshot.limit_5h_reset_at, None);
            assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("75%"));
            assert_eq!(snapshot.limit_weekly_reset_at, Some(12345));
        }
        body["plan_type"] = serde_json::json!("prolite");
        assert_eq!(
            parse_usage_snapshot(&body, Some("plus")).limit_5h_text,
            None
        );
        body["plan_type"] = serde_json::json!("plus");
        let snapshot = parse_usage_snapshot(&body, Some("pro"));
        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(23456));
    }

    #[test]
    fn parse_usage_snapshot_pro_does_not_relabel_timed_five_hour_window_as_weekly() {
        let body = serde_json::json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10, "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS, "reset_at": 12345
                }
            }
        });
        let snapshot = parse_usage_snapshot(&body, None);
        assert_eq!(snapshot.limit_short_label, None);
        assert_eq!(snapshot.limit_5h_text, None);
        assert_eq!(snapshot.limit_weekly_text, None);
        assert_eq!(snapshot.limit_weekly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_non_free_uses_window_duration_before_order() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 222
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_short_label.as_deref(), Some("5h"));
        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(222));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("90%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(111));
        assert_eq!(snapshot.limit_monthly_text, None);
        assert_eq!(snapshot.limit_monthly_reset_at, None);
    }

    #[test]
    fn parse_usage_snapshot_detects_monthly_window() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 222
                },
                "monthly_window": {
                    "used_percent": 70.0,
                    "limit_window_seconds": 30 * 24 * 60 * 60,
                    "reset_at": 333
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_short_label.as_deref(), Some("5h"));
        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(222));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("90%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(111));
        assert_eq!(snapshot.limit_monthly_text.as_deref(), Some("30%"));
        assert_eq!(snapshot.limit_monthly_reset_at, Some(333));
    }

    #[test]
    fn parse_usage_snapshot_detects_monthly_window_from_additional_rate_limits() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 222
                }
            },
            "additional_rate_limits": [
                {
                    "limit_name": "monthly",
                    "rate_limit": {
                        "primary_window": {
                            "used_percent": 70.0,
                            "limit_window_seconds": 30 * 24 * 60 * 60,
                            "reset_at": 333
                        }
                    }
                }
            ]
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("90%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(111));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(222));
        assert_eq!(snapshot.limit_monthly_text.as_deref(), Some("30%"));
        assert_eq!(snapshot.limit_monthly_reset_at, Some(333));
    }

    #[test]
    fn extract_reset_timestamp_uses_relative_seconds_when_absolute_timestamp_is_missing() {
        let window = serde_json::json!({
            "reset_after_seconds": 90
        });

        assert_eq!(extract_reset_timestamp_at(&window, 1_000), Some(1_090));
    }

    #[test]
    fn parse_usage_snapshot_reads_reset_credit_count() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 111
                }
            },
            "rate_limit_reset_credits": { "available_count": 2 }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.reset_credits_available, Some(2));
    }

    #[test]
    fn parse_usage_snapshot_keeps_zero_reset_credit_count_and_camel_case_alias() {
        let body = serde_json::json!({
            "rateLimitResetCredits": { "availableCount": 0 }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.reset_credits_available, Some(0));
    }

    #[test]
    fn parse_usage_snapshot_leaves_reset_credit_count_absent_when_missing() {
        let body = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 111
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.reset_credits_available, None);
    }

    #[test]
    fn parse_usage_snapshot_uses_limit_reached_hint_when_percent_is_missing() {
        let body = serde_json::json!({
            "rate_limit": {
                "limitReached": "true",
                "primary_window": {
                    "limit_window_seconds": FIVE_HOUR_WINDOW_SECONDS,
                    "reset_at": 111
                },
                "secondary_window": {
                    "used_percent": 40.0,
                    "limit_window_seconds": WEEK_WINDOW_SECONDS,
                    "reset_at": 222
                }
            }
        });

        let snapshot = parse_usage_snapshot(&body, Some("plus"));

        assert_eq!(snapshot.limit_5h_text.as_deref(), Some("0%"));
        assert_eq!(snapshot.limit_5h_reset_at, Some(111));
        assert_eq!(snapshot.limit_weekly_text.as_deref(), Some("60%"));
        assert_eq!(snapshot.limit_weekly_reset_at, Some(222));
    }

    #[test]
    fn should_hide_virtual_local_account_when_refresh_token_matches_saved_account() {
        let local_auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "local-access",
                "refresh_token": "same-refresh",
                "account_id": "acc-1",
                "id_token": "header.payload.sig"
            }
        });
        let persisted_account = CodexOfficialAccount {
            id: "saved-1".to_string(),
            provider_id: "provider-1".to_string(),
            name: "saved".to_string(),
            kind: "oauth".to_string(),
            email: Some("saved@example.com".to_string()),
            auth_snapshot: Some(
                serde_json::json!({
                    "auth_mode": "chatgpt",
                    "tokens": {
                        "access_token": "saved-access",
                        "refresh_token": "same-refresh",
                        "account_id": "acc-1",
                    }
                })
                .to_string(),
            ),
            auth_mode: Some("chatgpt".to_string()),
            account_id: Some("acc-1".to_string()),
            plan_type: None,
            last_refresh: None,
            token_expires_at: None,
            access_token_preview: None,
            refresh_token_preview: None,
            limit_short_label: None,
            limit_5h_text: None,
            limit_weekly_text: None,
            limit_monthly_text: None,
            limit_5h_reset_at: None,
            limit_weekly_reset_at: None,
            limit_monthly_reset_at: None,
            reset_credits_available: None,
            credits: None,
            last_limits_fetched_at: None,
            last_error: None,
            sort_index: None,
            is_applied: false,
            is_virtual: false,
            created_at: String::new(),
            updated_at: String::new(),
        };

        assert!(!should_show_virtual_local_account(
            &[persisted_account],
            &local_auth
        ));
    }

    #[test]
    fn should_show_virtual_local_account_when_local_auth_is_not_saved() {
        let local_auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "local-access",
                "refresh_token": "local-refresh",
                "account_id": "acc-local"
            }
        });
        let persisted_account = CodexOfficialAccount {
            id: "saved-1".to_string(),
            provider_id: "provider-1".to_string(),
            name: "saved".to_string(),
            kind: "oauth".to_string(),
            email: Some("saved@example.com".to_string()),
            auth_snapshot: Some(
                serde_json::json!({
                    "auth_mode": "chatgpt",
                    "tokens": {
                        "access_token": "saved-access",
                        "refresh_token": "saved-refresh",
                        "account_id": "acc-saved"
                    }
                })
                .to_string(),
            ),
            auth_mode: Some("chatgpt".to_string()),
            account_id: Some("acc-saved".to_string()),
            plan_type: None,
            last_refresh: None,
            token_expires_at: None,
            access_token_preview: None,
            refresh_token_preview: None,
            limit_short_label: None,
            limit_5h_text: None,
            limit_weekly_text: None,
            limit_monthly_text: None,
            limit_5h_reset_at: None,
            limit_weekly_reset_at: None,
            limit_monthly_reset_at: None,
            reset_credits_available: None,
            credits: None,
            last_limits_fetched_at: None,
            last_error: None,
            sort_index: None,
            is_applied: false,
            is_virtual: false,
            created_at: String::new(),
            updated_at: String::new(),
        };

        assert!(should_show_virtual_local_account(
            &[persisted_account],
            &local_auth
        ));
    }
}

pub async fn ensure_codex_provider_has_no_official_accounts(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    if codex_provider_has_official_accounts(db, provider_id).await? {
        return Err(
            "Please delete all official accounts under this provider before deleting the provider"
                .to_string(),
        );
    }
    Ok(())
}

pub async fn codex_provider_has_official_accounts(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<bool, String> {
    Ok(!list_persisted_official_accounts(db, provider_id)
        .await?
        .is_empty())
}

pub async fn clear_all_codex_official_account_apply_status(
    db: &crate::db::SqliteDbState,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    db.with_conn_mut(|conn| {
        db_update_applied_status(conn, DbTable::CodexOfficialAccount, None, &now)
    })?;
    Ok(())
}

pub async fn sync_codex_official_account_apply_status(
    db: &crate::db::SqliteDbState,
    provider_id: &str,
) -> Result<(), String> {
    let local_auth = read_auth_json_from_disk(Some(db)).await?;
    if !auth_has_official_runtime(&local_auth) {
        return update_official_account_apply_status(db, None).await;
    }

    let accounts = list_persisted_official_accounts(db, provider_id).await?;
    let matched_account_id = accounts
        .iter()
        .find(|account| official_account_identity_matches_auth(account, &local_auth))
        .map(|account| account.id.clone());

    update_official_account_apply_status(db, matched_account_id.as_deref()).await
}

/// Regression suite for issue #440: credential selection, rotation persistence
/// and the bounded 401 recovery of Codex usage refreshes.
///
/// Everything runs against a local mock server plus a temp Codex root, so no
/// real account or network is involved. `TEST_GUARD` serializes the tests
/// because `TEST_ENDPOINTS` and `TEST_AUTH_ROOT` are process-wide.
#[cfg(test)]
mod usage_credential_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    static TEST_GUARD: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    /// One captured HTTP request against the mock server.
    #[derive(Debug, Clone)]
    struct MockRequest {
        path: String,
        headers: String,
        body: String,
    }

    impl MockRequest {
        fn header(&self, name: &str) -> Option<String> {
            let needle = format!("{}:", name.to_ascii_lowercase());
            self.headers.lines().find_map(|line| {
                line.to_ascii_lowercase().starts_with(&needle).then(|| {
                    line.split_once(':')
                        .map(|(_, value)| value.trim().to_string())
                        .unwrap_or_default()
                })
            })
        }
    }

    struct MockReply {
        status: u16,
        body: String,
        wait_for: Option<Arc<std::sync::atomic::AtomicBool>>,
    }

    /// Distinct email per account id keeps the identity checks meaningful: with
    /// a shared address every synthetic account would "match" every other one.
    fn test_id_token(account_id: &str) -> String {
        let payload = serde_json::json!({
            "email": format!("{account_id}@example.com"),
            "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
        });
        format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).expect("serialize JWT payload"))
        )
    }

    fn usage_body(used_percent: f64) -> String {
        serde_json::json!({
            "rate_limit": {"primary_window": {
                "used_percent": used_percent, "limit_window_seconds": WEEK_WINDOW_SECONDS
            }},
            "credits": {"has_credits": true, "balance": "12"}
        })
        .to_string()
    }

    fn token_body(access: &str, refresh: &str) -> String {
        serde_json::json!({
            "access_token": access,
            "refresh_token": refresh,
            "id_token": test_id_token("account-a")
        })
        .to_string()
    }

    /// Serves `replies.len()` requests in order on a background thread.
    struct MockServer {
        usage_url: String,
        token_url: String,
        requests: Arc<std::sync::Mutex<Vec<MockRequest>>>,
        usage_count: Arc<AtomicUsize>,
        token_count: Arc<AtomicUsize>,
        stopped: Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn start(usage_replies: Vec<MockReply>, token_replies: Vec<MockReply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let address = listener.local_addr().expect("mock address");
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let usage_count = Arc::new(AtomicUsize::new(0));
            let token_count = Arc::new(AtomicUsize::new(0));
            let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let expected = usage_replies.len() + token_replies.len();
            let requests_thread = requests.clone();
            let usage_thread = usage_count.clone();
            let token_thread = token_count.clone();
            let stopped_thread = stopped.clone();
            let handle = std::thread::spawn(move || {
                listener
                    .set_nonblocking(true)
                    .expect("mock listener nonblocking");
                // A client making fewer calls than budgeted must fail its own
                // assertion instead of hanging teardown, so the wait is bounded
                // and teardown can stop the loop early.
                let deadline = std::time::Instant::now() + Duration::from_secs(20);
                let mut served = 0;
                let mut deferred = Vec::new();
                while served < expected
                    && !stopped_thread.load(Ordering::SeqCst)
                    && std::time::Instant::now() < deadline
                {
                    let Ok((mut socket, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    // Accepted sockets inherit the listener's nonblocking mode on
                    // Windows; reads must block or the request looks empty.
                    socket.set_nonblocking(false).expect("mock socket blocking");
                    socket
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .expect("mock read timeout");
                    let Some(request) = read_http_request(&mut socket) else {
                        continue;
                    };
                    let reply = if request.path.contains("/oauth/token") {
                        let index = token_thread.fetch_add(1, Ordering::SeqCst);
                        &token_replies[index]
                    } else {
                        let index = usage_thread.fetch_add(1, Ordering::SeqCst);
                        &usage_replies[index]
                    };
                    let status = reply.status;
                    let body = reply.body.clone();
                    let wait_for = reply.wait_for.clone();
                    requests_thread.lock().expect("requests lock").push(request);
                    served += 1;
                    // Status zero models a transport reset before HTTP headers.
                    if status == 0 {
                        continue;
                    }
                    let mut respond = move || {
                        let _ = write!(
                            socket,
                            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    };
                    if let Some(gate) = wait_for {
                        deferred.push(std::thread::spawn(move || {
                            let deadline = std::time::Instant::now() + Duration::from_secs(5);
                            while !gate.load(Ordering::SeqCst)
                                && std::time::Instant::now() < deadline
                            {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            respond();
                        }));
                    } else {
                        respond();
                    }
                }
                for handle in deferred {
                    let _ = handle.join();
                }
            });
            Self {
                usage_url: format!("http://{address}/backend-api/wham/usage"),
                token_url: format!("http://{address}/oauth/token"),
                requests,
                usage_count,
                token_count,
                stopped,
                handle: Some(handle),
            }
        }

        fn requests(&self) -> Vec<MockRequest> {
            self.requests.lock().expect("requests lock").clone()
        }

        fn usage_calls(&self) -> usize {
            self.usage_count.load(Ordering::SeqCst)
        }

        fn token_calls(&self) -> usize {
            self.token_count.load(Ordering::SeqCst)
        }

        fn authorization_headers(&self) -> Vec<String> {
            self.requests()
                .into_iter()
                .filter(|request| !request.path.contains("/oauth/token"))
                .filter_map(|request| request.header("Authorization"))
                .collect()
        }

        /// Form bodies of the OAuth exchange requests, in order.
        fn token_request_bodies(&self) -> Vec<String> {
            self.requests()
                .into_iter()
                .filter(|request| request.path.contains("/oauth/token"))
                .map(|request| request.body)
                .collect()
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                // The listener lives inside the thread, so the signal plus the
                // short sleep in the accept loop is what releases it.
                let _ = handle.join();
            }
        }
    }

    fn read_http_request(socket: &mut std::net::TcpStream) -> Option<MockRequest> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let count = socket.read(&mut buffer).ok()?;
            if count == 0 {
                return None;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        while bytes.len() < header_end + length {
            let count = socket.read(&mut buffer).ok()?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let path = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_string();
        Some(MockRequest {
            path,
            headers,
            body: String::from_utf8_lossy(&bytes[header_end..]).to_string(),
        })
    }

    fn reply(status: u16, body: impl Into<String>) -> MockReply {
        MockReply {
            status,
            body: body.into(),
            wait_for: None,
        }
    }

    fn deferred_reply(
        status: u16,
        body: impl Into<String>,
        gate: Arc<std::sync::atomic::AtomicBool>,
    ) -> MockReply {
        MockReply {
            status,
            body: body.into(),
            wait_for: Some(gate),
        }
    }

    fn ok_usage(used_percent: f64) -> MockReply {
        reply(200, usage_body(used_percent))
    }

    /// JWT-shaped token whose `exp` is `offset_seconds` from now.
    fn token(suffix: &str, offset_seconds: i64) -> String {
        let expiration = chrono::Utc::now().timestamp() + offset_seconds;
        let payload = serde_json::json!({ "exp": expiration, "sub": suffix });
        format!(
            "e30.{}.{suffix}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    fn auth_with(access_token: &str, refresh_token: &str, account_id: &str) -> serde_json::Value {
        serde_json::json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-01-01T00:00:00Z",
            "tokens": {
                "access_token": access_token,
                "refresh_token": refresh_token,
                "id_token": test_id_token(account_id),
                "account_id": account_id,
            }
        })
    }

    struct TestEnv {
        _root: tempfile::TempDir,
        db: SqliteDbState,
        provider_id: String,
    }

    impl TestEnv {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temp codex root");
            *TEST_AUTH_ROOT.lock().expect("test auth root lock") = Some(root.path().to_path_buf());
            let db = SqliteDbState::in_memory_for_test().expect("in-memory db");
            // A direct proxy keeps reqwest from picking up env proxies for the
            // loopback mock server.
            db.with_conn(|conn| {
                db_put(
                    conn,
                    DbTable::Settings,
                    "app",
                    &serde_json::json!({"proxy_mode": "direct"}),
                )
            })
            .expect("settings");
            let provider_id = "provider-a".to_string();
            db.with_conn(|conn| {
                db_put(
                    conn,
                    DbTable::CodexProvider,
                    &provider_id,
                    &serde_json::json!({"category": "official", "name": "Official"}),
                )
            })
            .expect("provider");
            Self {
                _root: root,
                db,
                provider_id,
            }
        }

        fn write_runtime_auth(&self, auth: &serde_json::Value) {
            std::fs::write(
                self._root.path().join("auth.json"),
                serde_json::to_string_pretty(auth).unwrap(),
            )
            .expect("write runtime auth");
        }

        fn read_runtime_auth(&self) -> serde_json::Value {
            std::fs::read_to_string(self._root.path().join("auth.json"))
                .ok()
                .and_then(|content| serde_json::from_str(&content).ok())
                .unwrap_or_else(|| serde_json::json!({}))
        }

        async fn save_account(
            &self,
            auth: &serde_json::Value,
            applied: bool,
        ) -> CodexOfficialAccount {
            let mut content =
                build_account_content_from_auth_snapshot(&self.provider_id, auth, None, None)
                    .expect("account content");
            content.is_applied = applied;
            save_official_account(&self.db, &content)
                .await
                .expect("save account")
        }

        async fn refresh(&self, account_id: &str) -> Result<CodexOfficialAccount, String> {
            refresh_persisted_account_usage(&self.db, account_id, None)
                .await
                .map(|(account, _)| account)
        }

        async fn stored_snapshot(&self, account_id: &str) -> serde_json::Value {
            let account = load_official_account(&self.db, account_id)
                .await
                .expect("load account");
            auth_json_from_snapshot(account.auth_snapshot.as_deref().unwrap()).expect("snapshot")
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            *TEST_AUTH_ROOT.lock().expect("test auth root lock") = None;
            clear_test_endpoints();
            // The 30s refresh cache is keyed by refresh token and process-wide;
            // a leftover entry would let a later test skip its OAuth exchange.
            if let Ok(mut cache) = OAUTH_REFRESH_CACHE.try_lock() {
                cache.clear();
            }
        }
    }

    async fn wait_for_usage_calls(server: &MockServer, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while server.usage_calls() < count {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("mock usage request did not arrive");
    }

    #[tokio::test]
    async fn newer_usage_success_survives_older_failure() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let release_old = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = MockServer::start(
            vec![
                deferred_reply(500, "{}", release_old.clone()),
                ok_usage(25.0),
            ],
            Vec::new(),
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        let account = env
            .save_account(
                &auth_with(
                    &token("access-a", 30 * 24 * 60 * 60),
                    "refresh-a",
                    "account-a",
                ),
                false,
            )
            .await;

        let db = env.db.clone();
        let account_id = account.id.clone();
        let old =
            tokio::spawn(
                async move { refresh_persisted_account_usage(&db, &account_id, None).await },
            );
        wait_for_usage_calls(&server, 1).await;
        let latest = env
            .refresh(&account.id)
            .await
            .expect("newer usage succeeds");
        assert_eq!(latest.limit_weekly_text.as_deref(), Some("75%"));
        release_old.store(true, Ordering::SeqCst);
        old.await
            .expect("old task")
            .expect("stale failure is ignored");

        let stored = load_official_account(&env.db, &account.id).await.unwrap();
        assert_eq!(stored.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(
            stored.credits.as_ref().and_then(|credits| credits.balance),
            Some(12.0)
        );
        assert!(stored.last_error.is_none());
        assert!(USAGE_REQUESTS.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn newer_usage_failure_survives_older_success() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let release_old = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = MockServer::start(
            vec![
                deferred_reply(200, usage_body(10.0), release_old.clone()),
                reply(500, "{}"),
            ],
            Vec::new(),
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        let account = env
            .save_account(
                &auth_with(
                    &token("access-a", 30 * 24 * 60 * 60),
                    "refresh-a",
                    "account-a",
                ),
                false,
            )
            .await;

        let db = env.db.clone();
        let account_id = account.id.clone();
        let old =
            tokio::spawn(
                async move { refresh_persisted_account_usage(&db, &account_id, None).await },
            );
        wait_for_usage_calls(&server, 1).await;
        assert!(env.refresh(&account.id).await.unwrap_err().contains("500"));
        release_old.store(true, Ordering::SeqCst);
        old.await
            .expect("old task")
            .expect("stale success is ignored");

        let stored = load_official_account(&env.db, &account.id).await.unwrap();
        assert!(stored
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("500")));
        assert!(stored.credits.is_none());
        assert!(stored.last_limits_fetched_at.is_none());
        assert!(USAGE_REQUESTS.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn queued_persisted_401_recovery_reuses_first_rotation() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let release_exchange = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rotated_access = token("access-rotated", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![
                reply(401, "{}"),
                reply(401, "{}"),
                ok_usage(10.0),
                ok_usage(10.0),
            ],
            vec![deferred_reply(
                200,
                token_body(&rotated_access, "refresh-rotated"),
                release_exchange.clone(),
            )],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        let account = env
            .save_account(
                &auth_with(
                    &token("access-old", 30 * 24 * 60 * 60),
                    "refresh-old",
                    "account-a",
                ),
                false,
            )
            .await;
        let first_db = env.db.clone();
        let first_id = account.id.clone();
        let first = tokio::spawn(async move {
            refresh_persisted_account_usage(&first_db, &first_id, None).await
        });
        wait_for_usage_calls(&server, 1).await;
        let second_db = env.db.clone();
        let second_id = account.id.clone();
        let second = tokio::spawn(async move {
            refresh_persisted_account_usage(&second_db, &second_id, None).await
        });
        wait_for_usage_calls(&server, 2).await;
        release_exchange.store(true, Ordering::SeqCst);
        first.await.expect("first task").expect("first recovery");
        second.await.expect("second task").expect("second recovery");

        assert_eq!(
            server.token_calls(),
            1,
            "a queued 401 must reuse the rotated token"
        );
        assert_eq!(server.usage_calls(), 4);
        assert_eq!(
            env.stored_snapshot(&account.id).await["tokens"]["refresh_token"],
            "refresh-rotated"
        );
    }

    #[tokio::test]
    async fn queued_local_401_recovery_reuses_first_rotation() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let release_exchange = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rotated_access = token("access-rotated", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![
                reply(401, "{}"),
                reply(401, "{}"),
                ok_usage(10.0),
                ok_usage(10.0),
            ],
            vec![deferred_reply(
                200,
                token_body(&rotated_access, "refresh-rotated"),
                release_exchange.clone(),
            )],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        env.write_runtime_auth(&auth_with(
            &token("access-old", 30 * 24 * 60 * 60),
            "refresh-old",
            "account-a",
        ));
        let first_db = env.db.clone();
        let first = tokio::spawn(async move { refresh_local_account_usage(&first_db, None).await });
        wait_for_usage_calls(&server, 1).await;
        let second_db = env.db.clone();
        let second =
            tokio::spawn(async move { refresh_local_account_usage(&second_db, None).await });
        wait_for_usage_calls(&server, 2).await;
        release_exchange.store(true, Ordering::SeqCst);
        first.await.expect("first task").expect("first recovery");
        second.await.expect("second task").expect("second recovery");

        assert_eq!(
            server.token_calls(),
            1,
            "a queued local 401 must reuse the rotated token"
        );
        assert_eq!(server.usage_calls(), 4);
        assert_eq!(
            env.read_runtime_auth()["tokens"]["refresh_token"],
            "refresh-rotated"
        );
    }

    #[tokio::test]
    async fn official_oauth_saves_new_tokens_without_waiting_for_usage_or_live_auth() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let access = token("browser-access", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            Vec::new(),
            vec![reply(200, token_body(&access, "browser-refresh"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        std::fs::write(env._root.path().join("auth.json"), "{malformed")
            .expect("write unrelated broken runtime auth");
        let account = exchange_and_save_official_account(
            &env.db,
            "provider-a",
            "one-shot-code",
            "http://localhost:1457/auth/callback",
            "pkce-verifier",
        )
        .await
        .unwrap();
        let stored = env.stored_snapshot(&account.id).await;
        assert_eq!(stored["tokens"]["refresh_token"], "browser-refresh");
        assert_eq!(
            server.usage_calls(),
            0,
            "login success must not await quota"
        );
        assert_eq!(
            std::fs::read_to_string(env._root.path().join("auth.json")).unwrap(),
            "{malformed",
            "login must not read or replace unrelated live auth"
        );
        let bodies = server.token_request_bodies();
        assert!(bodies[0].contains("redirect_uri=http%3A%2F%2Flocalhost%3A1457%2Fauth%2Fcallback"));
        assert!(bodies[0].contains("code_verifier=pkce-verifier"));
    }

    #[tokio::test]
    async fn official_oauth_missing_id_token_never_borrows_live_identity() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        env.write_runtime_auth(&auth_with(
            &token("old-access", 30 * 24 * 60 * 60),
            "old-refresh",
            "old-account",
        ));
        let server = MockServer::start(
            Vec::new(),
            vec![reply(
                200,
                serde_json::json!({
                    "access_token": "new-access",
                    "refresh_token": "new-refresh"
                })
                .to_string(),
            )],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);

        let error = exchange_and_save_official_account(
            &env.db,
            "provider-a",
            "one-shot-code",
            CODEX_DEVICE_REDIRECT_URI,
            "pkce-verifier",
        )
        .await
        .unwrap_err();
        assert!(error.contains("missing id_token"));
        assert!(list_persisted_official_accounts(&env.db, "provider-a")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            env.read_runtime_auth()["tokens"]["refresh_token"],
            "old-refresh"
        );
    }

    #[tokio::test]
    async fn transient_usage_transport_failure_retries_once_without_rotating_tokens() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(vec![reply(0, ""), ok_usage(25.0)], Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);
        let access = token("access-a", 30 * 24 * 60 * 60);
        let account = env
            .save_account(&auth_with(&access, "refresh-a", "account-a"), false)
            .await;
        env.refresh(&account.id).await.unwrap();
        assert_eq!(server.usage_calls(), 2);
        assert_eq!(server.token_calls(), 0);
        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {access}"), format!("Bearer {access}")]
        );
    }

    /// Scenario 1: usable stored credential, outside the refresh window.
    #[tokio::test]
    async fn usable_stored_credential_skips_exchange_and_persists_usage() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(vec![ok_usage(25.0)], Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);

        let access = token("access-a", 30 * 24 * 60 * 60);
        let account = env
            .save_account(&auth_with(&access, "refresh-a", "account-a"), false)
            .await;
        let refreshed = env.refresh(&account.id).await.expect("refresh");

        assert_eq!(server.usage_calls(), 1);
        assert_eq!(
            server.token_calls(),
            0,
            "no OAuth exchange when the token is fresh"
        );
        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {access}")]
        );
        assert_eq!(refreshed.limit_weekly_text.as_deref(), Some("75%"));
        assert_eq!(refreshed.last_error, None);
        assert!(refreshed.last_limits_fetched_at.is_some());
    }

    /// Scenario 2: stored token expired; usage must wait for the rotation.
    #[tokio::test]
    async fn expired_stored_credential_is_rotated_before_usage() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let fresh_access = token("access-new", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![ok_usage(10.0)],
            vec![reply(200, token_body(&fresh_access, "refresh-new"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);

        let expired = token("access-old", -3600);
        let account = env
            .save_account(&auth_with(&expired, "refresh-old", "account-a"), false)
            .await;
        env.refresh(&account.id).await.expect("refresh");

        assert_eq!(server.token_calls(), 1);
        // The exchange must consume the stored refresh token, not a stale one.
        assert!(server.token_request_bodies()[0].contains("refresh-old"));
        assert_eq!(server.usage_calls(), 1);
        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {fresh_access}")]
        );
        let stored = env.stored_snapshot(&account.id).await;
        assert_eq!(stored["tokens"]["access_token"], fresh_access);
        assert_eq!(stored["tokens"]["refresh_token"], "refresh-new");
        // Not applied: the runtime file must stay untouched.
        assert_eq!(env.read_runtime_auth(), serde_json::json!({}));
    }

    /// Scenario 3: stored token expired, same-account runtime token still valid.
    #[tokio::test]
    async fn applied_account_adopts_newer_same_account_runtime_credential() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(vec![ok_usage(20.0)], Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);

        let runtime_access = token("access-runtime", 10 * 24 * 60 * 60);
        let runtime_auth = auth_with(&runtime_access, "refresh-runtime", "account-a");
        let account = env
            .save_account(
                &auth_with(
                    &token("access-stored", -3600),
                    "refresh-stored",
                    "account-a",
                ),
                true,
            )
            .await;
        env.write_runtime_auth(&runtime_auth);

        env.refresh(&account.id).await.expect("refresh");

        assert_eq!(
            server.token_calls(),
            0,
            "the runtime credential is used as-is"
        );
        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {runtime_access}")]
        );
        let stored = env.stored_snapshot(&account.id).await;
        assert_eq!(stored["tokens"]["access_token"], runtime_access);
        assert_eq!(stored["tokens"]["refresh_token"], "refresh-runtime");
    }

    /// Scenario 4: the runtime file belongs to a different login.
    #[tokio::test]
    async fn runtime_credential_of_another_account_is_never_borrowed() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(vec![ok_usage(30.0)], Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);

        let other_access = token("access-other", 20 * 24 * 60 * 60);
        let other_auth = auth_with(&other_access, "refresh-other", "account-b");
        let target_access = token("access-target", 20 * 24 * 60 * 60);
        let account = env
            .save_account(
                &auth_with(&target_access, "refresh-target", "account-a"),
                true,
            )
            .await;
        env.write_runtime_auth(&other_auth);

        env.refresh(&account.id).await.expect("refresh");

        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {target_access}")]
        );
        assert_eq!(server.token_calls(), 0);
        assert_eq!(
            env.read_runtime_auth(),
            other_auth,
            "another account's runtime login must be left alone"
        );
        assert_eq!(
            env.stored_snapshot(&account.id).await["tokens"]["access_token"],
            target_access
        );
    }

    /// Scenario 5: refreshing an unapplied account must not touch live state.
    #[tokio::test]
    async fn refreshing_unapplied_account_leaves_runtime_login_untouched() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(vec![ok_usage(40.0)], Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);

        let live_auth = auth_with(
            &token("access-live", 20 * 24 * 60 * 60),
            "refresh-live",
            "account-a",
        );
        env.write_runtime_auth(&live_auth);
        // Build the stored token once: `token()` stamps a fresh `exp` from
        // `Utc::now()`, so calling it again for the assertion yields a
        // different string whenever a whole second elapses in between.
        let stored_access = token("access-stored", 20 * 24 * 60 * 60);
        let account = env
            .save_account(
                &auth_with(&stored_access, "refresh-stored", "account-a"),
                false,
            )
            .await;

        env.refresh(&account.id).await.expect("refresh");

        assert_eq!(server.token_calls(), 0);
        assert_eq!(env.read_runtime_auth(), live_auth);
        assert_eq!(
            env.stored_snapshot(&account.id).await["tokens"]["access_token"],
            stored_access
        );
    }

    /// Scenario 6: unexpired token rejected once — force exactly one exchange.
    #[tokio::test]
    async fn unexpired_token_rejected_with_401_triggers_one_forced_recovery() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let rotated_access = token("access-rotated", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![reply(401, "{}"), ok_usage(15.0)],
            vec![reply(200, token_body(&rotated_access, "refresh-rotated"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);

        let original_access = token("access-original", 20 * 24 * 60 * 60);
        let account = env
            .save_account(
                &auth_with(&original_access, "refresh-original", "account-a"),
                false,
            )
            .await;

        env.refresh(&account.id)
            .await
            .expect("refresh after recovery");

        assert_eq!(server.usage_calls(), 2, "one retry, not a loop");
        assert_eq!(server.token_calls(), 1);
        assert_eq!(
            server.authorization_headers(),
            vec![
                format!("Bearer {original_access}"),
                format!("Bearer {rotated_access}")
            ]
        );
        let stored = env.stored_snapshot(&account.id).await;
        assert_eq!(stored["tokens"]["access_token"], rotated_access);
        assert_eq!(stored["tokens"]["refresh_token"], "refresh-rotated");
    }

    /// Scenario 7: recovery is bounded; the second 401 asks for reauth.
    #[tokio::test]
    async fn second_401_after_recovery_reports_reauth_without_looping() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let rotated_access = token("access-rotated", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![reply(401, "{}"), reply(401, "{}")],
            vec![reply(200, token_body(&rotated_access, "refresh-rotated"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);

        let account = env
            .save_account(
                &auth_with(
                    &token("access-original", 20 * 24 * 60 * 60),
                    "refresh-original",
                    "account-a",
                ),
                false,
            )
            .await;

        let error = env
            .refresh(&account.id)
            .await
            .expect_err("still unauthorized");

        assert!(error.contains(CODEX_REAUTH_REQUIRED_MARKER), "{error}");
        assert_eq!(server.usage_calls(), 2);
        assert_eq!(server.token_calls(), 1);
        let stored = load_official_account(&env.db, &account.id).await.unwrap();
        assert_eq!(stored.last_error.as_deref(), Some(error.as_str()));
        // Neither a success nor a new balance: the usage windows stay as they were.
        assert_eq!(stored.credits, None);
        assert_eq!(stored.limit_weekly_text, None);
    }

    /// Scenario 8: the refresh token itself is rejected.
    #[tokio::test]
    async fn rejected_refresh_token_reports_reauth_and_keeps_the_account() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(
            vec![reply(401, "{}")],
            vec![reply(400, r#"{"error":"invalid_grant"}"#)],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);

        let original = auth_with(
            &token("access-original", 20 * 24 * 60 * 60),
            "refresh-original",
            "account-a",
        );
        let account = env.save_account(&original, false).await;

        let error = env.refresh(&account.id).await.expect_err("reauth required");

        assert!(error.contains(CODEX_REAUTH_REQUIRED_MARKER), "{error}");
        assert_eq!(server.usage_calls(), 1);
        assert_eq!(server.token_calls(), 1);
        let stored = load_official_account(&env.db, &account.id).await.unwrap();
        assert_eq!(
            stored.auth_snapshot.as_deref().map(auth_json_from_snapshot),
            Some(Ok(original)),
            "the account record survives a reauth error"
        );
        assert!(stored.last_error.is_some());
    }

    /// Scenario 8b: an account stored without a refresh token.
    #[tokio::test]
    async fn missing_refresh_token_reports_reauth_without_any_request() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(Vec::new(), Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);
        let mut auth = auth_with(&token("access-a", -3600), "", "account-a");
        auth["tokens"]
            .as_object_mut()
            .unwrap()
            .remove("refresh_token");

        let account = env.save_account(&auth, false).await;
        let error = env
            .refresh(&account.id)
            .await
            .expect_err("no refresh token");

        assert!(error.contains(CODEX_REAUTH_REQUIRED_MARKER), "{error}");
        assert_eq!(server.usage_calls(), 0);
        assert_eq!(server.token_calls(), 0);
    }

    /// Scenario 8c: an unapplied account is refreshed by the background pass too.
    #[tokio::test]
    async fn background_pass_persists_a_non_applied_account_rotation() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let fresh_access = token("access-new", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            Vec::new(),
            vec![reply(200, token_body(&fresh_access, "refresh-new"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        let account = env
            .save_account(
                &auth_with(&token("access-old", -3600), "refresh-old", "account-a"),
                false,
            )
            .await;

        let prepared = prepare_persisted_account_credential(&env.db, &account.id)
            .await
            .expect("background preparation");

        assert_eq!(server.token_calls(), 1);
        assert_eq!(prepared.auth["tokens"]["refresh_token"], "refresh-new");
        assert!(!prepared.runtime_written);
        assert_eq!(
            env.stored_snapshot(&account.id).await["tokens"]["refresh_token"],
            "refresh-new"
        );
    }

    /// Scenario 9: rotation persists even when the usage call afterwards fails.
    #[tokio::test]
    async fn rotated_credential_is_persisted_before_a_failing_usage_call() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let fresh_access = token("access-new", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![reply(500, "{}")],
            vec![reply(200, token_body(&fresh_access, "refresh-new"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);

        let account = env
            .save_account(
                &auth_with(&token("access-old", -3600), "refresh-old", "account-a"),
                false,
            )
            .await;

        let error = env.refresh(&account.id).await.expect_err("usage failed");
        assert!(!error.contains(CODEX_REAUTH_REQUIRED_MARKER), "{error}");
        assert!(error.contains("500"), "{error}");

        let stored = env.stored_snapshot(&account.id).await;
        assert_eq!(stored["tokens"]["refresh_token"], "refresh-new");
        assert_eq!(stored["tokens"]["access_token"], fresh_access);
        let row = load_official_account(&env.db, &account.id).await.unwrap();
        assert_eq!(row.credits, None);
        assert_eq!(row.last_limits_fetched_at, None);
    }

    /// Scenario 10: transient errors keep their meaning and never rotate.
    #[tokio::test]
    async fn transient_usage_errors_do_not_trigger_a_token_exchange() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        for (status, expected) in [(429u16, "429"), (500, "500"), (503, "503")] {
            let server = MockServer::start(vec![reply(status, "{}")], Vec::new());
            set_test_endpoints(&server.usage_url, &server.token_url);
            let account = env
                .save_account(
                    &auth_with(
                        &token("access-a", 20 * 24 * 60 * 60),
                        "refresh-a",
                        "account-a",
                    ),
                    false,
                )
                .await;

            let error = env.refresh(&account.id).await.expect_err("usage failed");

            assert!(error.contains(expected), "{error}");
            assert!(!error.contains(CODEX_REAUTH_REQUIRED_MARKER), "{error}");
            assert_eq!(server.usage_calls(), 1);
            assert_eq!(server.token_calls(), 0);
        }
    }

    /// Scenario 11: the `__local__` virtual account expires and refreshes in place.
    #[tokio::test]
    async fn local_virtual_account_refreshes_its_expired_runtime_credential() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let fresh_access = token("access-new", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![ok_usage(12.0)],
            vec![reply(200, token_body(&fresh_access, "refresh-new"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        env.write_runtime_auth(&auth_with(
            &token("access-old", -3600),
            "refresh-old",
            "account-a",
        ));

        let result = refresh_local_account_usage(&env.db, None)
            .await
            .expect("local usage");

        assert_eq!(result.usage.limit_weekly_text.as_deref(), Some("88%"));
        // The returned credential is the one that answered, not a later read.
        assert_eq!(result.credential["tokens"]["access_token"], fresh_access);
        assert_eq!(server.token_calls(), 1);
        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {fresh_access}")]
        );
        let runtime = env.read_runtime_auth();
        assert_eq!(runtime["tokens"]["access_token"], fresh_access);
        assert_eq!(runtime["tokens"]["refresh_token"], "refresh-new");
        // No persisted account was created as a side effect.
        assert!(list_persisted_official_accounts(&env.db, &env.provider_id)
            .await
            .unwrap()
            .is_empty());
    }

    /// Scenario 12: a concurrent rotation must not consume the old token twice.
    #[tokio::test]
    async fn credential_is_reread_after_waiting_for_the_oauth_lock() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(vec![ok_usage(18.0)], Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);
        let account = env
            .save_account(
                &auth_with(
                    &token("access-a", 20 * 24 * 60 * 60),
                    "refresh-a",
                    "account-a",
                ),
                false,
            )
            .await;

        // Simulate another writer rotating the row while this refresh waits.
        let rotated_access = token("access-rotated", 30 * 24 * 60 * 60);
        let rotated = auth_with(&rotated_access, "refresh-rotated", "account-a");
        let row = load_official_account(&env.db, &account.id).await.unwrap();
        persist_refreshed_account_snapshot(&env.db, &row, &rotated)
            .await
            .expect("simulated concurrent rotation");

        env.refresh(&account.id).await.expect("refresh");

        assert_eq!(
            server.token_calls(),
            0,
            "the rotated credential is still fresh"
        );
        assert_eq!(
            server.authorization_headers(),
            vec![format!("Bearer {rotated_access}")]
        );
    }

    /// Scenario 11b: the `__local__` account also recovers from a 401.
    #[tokio::test]
    async fn local_virtual_account_recovers_from_401_with_a_forced_exchange() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let rotated_access = token("access-rotated", 30 * 24 * 60 * 60);
        let server = MockServer::start(
            vec![reply(401, "{}"), ok_usage(8.0)],
            vec![reply(200, token_body(&rotated_access, "refresh-rotated"))],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        let original_access = token("access-original", 20 * 24 * 60 * 60);
        let mut initial = auth_with(&original_access, "refresh-original", "account-a");
        // A field Codex itself owns and AI Toolbox must keep verbatim.
        initial["runtime_owned"] = serde_json::json!({ "keep": true });
        env.write_runtime_auth(&initial);

        let result = refresh_local_account_usage(&env.db, None)
            .await
            .expect("local usage after recovery");

        assert_eq!(server.usage_calls(), 2);
        assert_eq!(server.token_calls(), 1);
        assert_eq!(result.usage.limit_weekly_text.as_deref(), Some("92%"));
        let runtime = env.read_runtime_auth();
        assert_eq!(runtime["tokens"]["access_token"], rotated_access);
        assert_eq!(runtime["tokens"]["refresh_token"], "refresh-rotated");
        assert_eq!(runtime["runtime_owned"]["keep"], true);
    }

    /// Scenario 13: a late response cannot overwrite the newly applied account.
    #[tokio::test]
    async fn runtime_write_is_skipped_when_the_account_was_switched_away() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let switched_auth = auth_with(
            &token("access-switched", 25 * 24 * 60 * 60),
            "refresh-switched",
            "account-b",
        );
        env.write_runtime_auth(&switched_auth);
        let rotated = auth_with(
            &token("access-rotated", 30 * 24 * 60 * 60),
            "refresh-rotated",
            "account-a",
        );

        // The row was applied when the request started but is no longer applied.
        let account = env
            .save_account(
                &auth_with(
                    &token("access-a", 20 * 24 * 60 * 60),
                    "refresh-a",
                    "account-a",
                ),
                false,
            )
            .await;
        let wrote = write_runtime_credential_if_still_applied(&env.db, &account.id, &rotated)
            .await
            .expect("write check");

        assert!(!wrote);
        assert_eq!(env.read_runtime_auth(), switched_auth);
    }

    /// Scenario 14: a missing or broken snapshot never sends a usage request.
    #[tokio::test]
    async fn missing_or_broken_snapshot_sends_no_usage_request() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(Vec::new(), Vec::new());
        set_test_endpoints(&server.usage_url, &server.token_url);

        for snapshot in [
            serde_json::Value::Null,
            serde_json::Value::String("not-json".to_string()),
            serde_json::json!(r#"{"tokens":{"access_token":" "}}"#),
        ] {
            let account = env
                .save_account(
                    &auth_with(
                        &token("access-a", 20 * 24 * 60 * 60),
                        "refresh-a",
                        "account-a",
                    ),
                    false,
                )
                .await;
            env.db
                .with_conn(|conn| {
                    db_patch_fields(
                        conn,
                        DbTable::CodexOfficialAccount,
                        &account.id,
                        &[("auth_snapshot", snapshot.clone())],
                    )
                    .map(|_| ())
                })
                .unwrap();

            let error = env
                .refresh(&account.id)
                .await
                .expect_err("unusable snapshot");
            assert!(!error.is_empty());
        }

        assert_eq!(server.usage_calls(), 0);
        assert_eq!(server.token_calls(), 0);
    }

    /// Scenario 15: header account id follows the credential actually used.
    #[tokio::test]
    async fn account_header_comes_from_the_recovered_credential() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let server = MockServer::start(
            vec![reply(401, "{}"), ok_usage(22.0)],
            vec![reply(
                200,
                serde_json::json!({
                    "access_token": token("access-a-rotated", 30 * 24 * 60 * 60),
                    "refresh_token": "refresh-a-rotated",
                    "id_token": test_id_token("account-a"),
                })
                .to_string(),
            )],
        );
        set_test_endpoints(&server.usage_url, &server.token_url);
        let account = env
            .save_account(
                &auth_with(
                    &token("access-a", 20 * 24 * 60 * 60),
                    "refresh-a",
                    "account-a",
                ),
                false,
            )
            .await;

        env.refresh(&account.id).await.expect("refresh");

        let headers: Vec<String> = server
            .requests()
            .into_iter()
            .filter(|request| !request.path.contains("/oauth/token"))
            .filter_map(|request| request.header("Chatgpt-Account-Id"))
            .collect();
        assert_eq!(
            headers,
            vec!["account-a".to_string(), "account-a".to_string()]
        );
    }

    /// Scenario 16: a failed refresh stays a failure on the stored row.
    #[tokio::test]
    async fn failed_refresh_keeps_previous_usage_and_records_the_error() {
        let _guard = TEST_GUARD.lock().await;
        let env = TestEnv::new();
        let success = MockServer::start(vec![ok_usage(35.0)], Vec::new());
        set_test_endpoints(&success.usage_url, &success.token_url);
        let account = env
            .save_account(
                &auth_with(
                    &token("access-a", 20 * 24 * 60 * 60),
                    "refresh-a",
                    "account-a",
                ),
                false,
            )
            .await;
        let first = env.refresh(&account.id).await.expect("first refresh");
        assert_eq!(first.limit_weekly_text.as_deref(), Some("65%"));
        let first_fetched_at = first.last_limits_fetched_at.clone();

        let failing = MockServer::start(vec![reply(503, "{}")], Vec::new());
        set_test_endpoints(&failing.usage_url, &failing.token_url);
        let error = env
            .refresh(&account.id)
            .await
            .expect_err("second refresh fails");

        let stored = load_official_account(&env.db, &account.id).await.unwrap();
        assert_eq!(stored.last_error.as_deref(), Some(error.as_str()));
        // Windows and the last success time are not rewritten by a failure.
        assert_eq!(stored.limit_weekly_text.as_deref(), Some("65%"));
        assert_eq!(stored.last_limits_fetched_at, first_fetched_at);
        // Credits are the exception: the cached balance is invalidated.
        assert_eq!(stored.credits, None);
    }
}
