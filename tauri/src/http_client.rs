//! Unified HTTP Client Module
//!
//! Provides HTTP client creation with automatic proxy configuration.
//! All HTTP requests in the application should use this module to ensure
//! they respect the user's proxy settings.
//!
//! # Usage
//!
//! ```rust,no_run
//! use ai_toolbox_lib::{http_client, SqliteDbState};
//!
//! # async fn demo(state: &SqliteDbState) -> Result<(), String> {
//! // Create client with automatic proxy configuration (30s timeout)
//! let client = http_client::client(state).await?;
//!
//! // Create client with custom timeout
//! let client = http_client::client_with_timeout(state, 60).await?;
//!
//! // Bypass proxy (special cases only)
//! let direct_client = http_client::create_client_no_proxy(30)?;
//! # let _ = (client, direct_client);
//! # Ok(())
//! # }
//! ```

use reqwest::{Client, Proxy};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::db::SqliteDbState;
use crate::settings::store::load_settings_from_sqlite_state;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    Direct,
    Custom,
    System,
}

impl ProxyMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "direct" => Self::Direct,
            "custom" => Self::Custom,
            _ => Self::System,
        }
    }
}

const ACCOUNT_CLIENT_TTL: Duration = Duration::from_secs(60);

struct AccountClientCache {
    proxy_mode: ProxyMode,
    proxy_url: String,
    created_at: Instant,
    client: Client,
}

static ACCOUNT_CLIENT: OnceLock<Mutex<Option<AccountClientCache>>> = OnceLock::new();

/// A bounded, pooled client for account OAuth and usage requests. A settings
/// change replaces the single cached client immediately; the short lifetime
/// also refreshes reqwest's snapshot of system and environment proxy settings.
pub async fn account_client(db_state: &SqliteDbState) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    account_client_for_proxy(proxy_mode, proxy_url).await
}

async fn account_client_for_proxy(
    proxy_mode: ProxyMode,
    proxy_url: String,
) -> Result<Client, String> {
    let cache = ACCOUNT_CLIENT.get_or_init(|| Mutex::new(None));
    account_client_for_proxy_with_cache(cache, proxy_mode, proxy_url).await
}

async fn account_client_for_proxy_with_cache(
    cache: &Mutex<Option<AccountClientCache>>,
    proxy_mode: ProxyMode,
    proxy_url: String,
) -> Result<Client, String> {
    {
        let guard = cache
            .lock()
            .map_err(|_| "Account HTTP client cache unavailable")?;
        if let Some(entry) = guard.as_ref() {
            if entry.proxy_mode == proxy_mode
                && entry.proxy_url == proxy_url
                && entry.created_at.elapsed() < ACCOUNT_CLIENT_TTL
            {
                return Ok(entry.client.clone());
            }
        }
    }

    // Client construction can load native roots and inspect system proxy state.
    // Keep that work off the async executor used by OAuth and usage refreshes.
    let build_proxy_url = proxy_url.clone();
    let client = tokio::task::spawn_blocking(move || {
        let mut builder = Client::builder()
            .use_rustls_tls()
            .http1_title_case_headers()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10));
        match proxy_mode {
            ProxyMode::Direct => builder = builder.no_proxy(),
            ProxyMode::Custom => {
                if build_proxy_url.is_empty() {
                    return Err("Custom proxy mode requires a proxy URL".to_string());
                }
                let proxy = build_proxy(&build_proxy_url)
                    .map_err(|_| "Invalid account proxy configuration".to_string())?
                    .ok_or_else(|| "Invalid account proxy configuration".to_string())?;
                builder = builder.proxy(proxy);
            }
            ProxyMode::System => {}
        }
        builder
            .build()
            .map_err(|_| "Failed to create account HTTP client".to_string())
    })
    .await
    .map_err(|_| "Account HTTP client setup task failed".to_string())??;

    let mut guard = cache
        .lock()
        .map_err(|_| "Account HTTP client cache unavailable")?;
    *guard = Some(AccountClientCache {
        proxy_mode,
        proxy_url,
        created_at: Instant::now(),
        client: client.clone(),
    });
    Ok(client)
}

/// Create an HTTP client with automatic proxy configuration.
///
/// This is the primary function for making HTTP requests.
/// Proxy settings are automatically applied from user settings.
///
/// # Arguments
/// * `db_state` - Database state to read proxy settings from
///
/// # Returns
/// A configured reqwest::Client with 30 second timeout
///
/// # Example
/// ```rust,no_run
/// use ai_toolbox_lib::{http_client, SqliteDbState};
///
/// # async fn demo(state: &SqliteDbState) -> Result<(), String> {
/// let client = http_client::client(state).await?;
/// let response = client
///     .get("https://api.example.com")
///     .send()
///     .await
///     .map_err(|error| error.to_string())?;
/// # let _ = response;
/// # Ok(())
/// # }
/// ```
pub async fn client(db_state: &SqliteDbState) -> Result<Client, String> {
    client_with_timeout(db_state, 30).await
}

/// Create an HTTP client with custom timeout.
///
/// # Arguments
/// * `db_state` - Database state to read proxy settings from
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A configured reqwest::Client
///
/// # Example
/// ```rust,no_run
/// use ai_toolbox_lib::{http_client, SqliteDbState};
///
/// # async fn demo(state: &SqliteDbState) -> Result<(), String> {
/// let client = http_client::client_with_timeout(state, 60).await?;
/// # let _ = client;
/// # Ok(())
/// # }
/// ```
pub async fn client_with_timeout(
    db_state: &SqliteDbState,
    timeout_secs: u64,
) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    build_client(proxy_mode, &proxy_url, Some(timeout_secs), false)
}

/// Create an HTTP client with custom timeout and disabled automatic response decompression.
///
/// Use this for compatibility workarounds when an upstream gateway mishandles
/// compressed responses for specific clients or platforms.
pub async fn client_with_timeout_no_compression(
    db_state: &SqliteDbState,
    timeout_secs: u64,
) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    build_client(proxy_mode, &proxy_url, Some(timeout_secs), true)
}

/// Streaming-oriented HTTP client: no overall request timeout so long-lived SSE
/// bodies are not cut off after a fixed wall-clock budget. Connect timeout still
/// applies; first-byte and per-chunk idle timeouts are enforced by callers.
pub async fn client_streaming_no_compression(db_state: &SqliteDbState) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    build_client(proxy_mode, &proxy_url, None, true)
}

/// WebSocket upgrades use the same proxy/TLS policy, without redirects or a
/// total request timeout that could cut off the upgraded connection.
pub async fn client_websocket_handshake(db_state: &SqliteDbState) -> Result<Client, String> {
    let (proxy_mode, proxy_url) = get_proxy_from_settings(db_state).await?;
    build_client_for_transport(proxy_mode, &proxy_url, None, true, true)
}

/// Build an HTTP client with explicit proxy URL.
///
/// This is an internal function. Business code should use `client()` or `client_with_timeout()`.
///
/// # Arguments
/// * `proxy_mode` - Proxy mode selected by user
/// * `proxy_url` - Proxy URL (e.g., "http://proxy.com:8080" or "socks5://proxy.com:1080")
///                 Only used when proxy_mode is custom
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A configured reqwest::Client
///
/// # Proxy Priority
/// 1. direct: explicitly disable all proxies (including system proxy)
/// 2. custom: use user-configured proxy
/// 3. system: use system proxy (Windows/macOS) or env vars (Linux)
fn build_client(
    proxy_mode: ProxyMode,
    proxy_url: &str,
    timeout_secs: Option<u64>,
    disable_content_decoding: bool,
) -> Result<Client, String> {
    build_client_for_transport(
        proxy_mode,
        proxy_url,
        timeout_secs,
        disable_content_decoding,
        false,
    )
}

fn build_client_for_transport(
    proxy_mode: ProxyMode,
    proxy_url: &str,
    timeout_secs: Option<u64>,
    disable_content_decoding: bool,
    websocket_upgrade: bool,
) -> Result<Client, String> {
    let mut builder = Client::builder()
        .use_rustls_tls()
        .http1_title_case_headers();
    if websocket_upgrade {
        builder = builder
            .http1_only()
            .redirect(reqwest::redirect::Policy::none());
    }
    if let Some(timeout_secs) = timeout_secs {
        builder = builder.timeout(Duration::from_secs(timeout_secs));
    } else {
        // Streaming clients: keep connect bounded, leave body open for idle/first-byte control.
        builder = builder.connect_timeout(Duration::from_secs(30));
    }

    if disable_content_decoding {
        builder = builder.no_gzip().no_brotli().no_zstd().no_deflate();
    }

    match proxy_mode {
        ProxyMode::Direct => {
            // User explicitly chose direct connection - bypass all proxies including system proxy
            builder = builder.no_proxy();
        }
        ProxyMode::Custom => {
            if proxy_url.is_empty() {
                return Err("Custom proxy mode requires a proxy URL".to_string());
            }
            if let Some(proxy) = build_proxy(proxy_url)? {
                builder = builder.proxy(proxy);
            }
        }
        ProxyMode::System => {}
    }
    // In system mode, reqwest automatically detects system proxy or environment proxies

    builder
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Create an HTTP client without proxy (bypass proxy settings).
///
/// Use this only when you explicitly need to bypass proxy settings.
///
/// # Arguments
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A reqwest::Client without proxy
pub fn create_client_no_proxy(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .use_rustls_tls()
        .http1_title_case_headers()
        .timeout(Duration::from_secs(timeout_secs))
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .no_proxy()
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Create an HTTP client that honors system / environment proxy without
/// requiring a `SqliteDbState`.
///
/// Unlike `create_client_no_proxy`, this does **not** call `.no_proxy()`, so
/// reqwest reads `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` from the
/// environment. This is the recovery-mode client used when the database is not
/// available (e.g. startup "schema too new" path) and the in-app proxy config
/// cannot be read. `use_rustls_tls()` is retained per the global TLS rule.
///
/// # Arguments
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// A reqwest::Client that respects environment proxy settings
pub fn create_client_with_env_proxy(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .use_rustls_tls()
        .http1_title_case_headers()
        .timeout(Duration::from_secs(timeout_secs))
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Flatten a `reqwest::Error` into one diagnosable line.
///
/// `reqwest::Error`'s own `Display` is deliberately terse: a failed send renders
/// as `error sending request for url (<url>)` and nothing else, while the
/// actionable part — `tcp connect error`, `tunnel error`, `operation timed out`,
/// the OS socket message, a TLS failure — only exists in `source()`.
/// Surface failures are persisted and shown to the user verbatim (Codex usage
/// limits are the canonical example: `Failed to fetch usage: error sending
/// request for url (...)` told nobody whether the proxy, DNS or the network was
/// at fault). Walk the chain instead of dropping it.
pub fn describe_error(error: &reqwest::Error) -> String {
    let mut description = error.to_string();
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        let text = error.to_string();
        if !text.is_empty() && !description.contains(&text) {
            description.push_str(": ");
            description.push_str(&text);
        }
        cause = error.source();
    }
    if let Some(url) = error.url() {
        let mut safe_url = url.clone();
        let _ = safe_url.set_username("");
        let _ = safe_url.set_password(None);
        safe_url.set_query(None);
        safe_url.set_fragment(None);
        description = description.replace(url.as_str(), safe_url.as_str());
    }
    description
}

/// Whether a failed send is worth retrying unchanged.
///
/// Only transport-level failures qualify. A missing response does not prove
/// that the server never received the request: callers must also ensure the
/// operation is safe to replay (such as the read-only usage GET). Never apply
/// this helper blindly to a one-shot OAuth code or rotating refresh token.
/// HTTP statuses keep their own semantics, including the bounded 401 recovery.
pub fn is_transient_transport_error(error: &reqwest::Error) -> bool {
    !error.is_status()
        && !error.is_builder()
        && !error.is_decode()
        && !error.is_body()
        && (error.is_connect() || error.is_timeout() || error.is_request())
}

/// Test proxy connectivity by making a request to a test URL.
///
/// This function is used by the settings page to validate proxy configuration.
///
/// # Arguments
/// * `proxy_url` - Proxy URL to test
///
/// # Returns
/// Ok(()) if connection successful, Err with message otherwise
pub async fn test_proxy(proxy_url: &str) -> Result<(), String> {
    if proxy_url.is_empty() {
        return Err("Proxy URL is empty".to_string());
    }

    // Create client with custom proxy mode
    let client = build_client(ProxyMode::Custom, proxy_url, Some(10), false)?;

    // Test with httpbin.org - it's designed for testing HTTP clients
    let response = client
        .get("https://httpbin.org/ip")
        .send()
        .await
        .map_err(|e| format!("Proxy connection failed: {}", e))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "Proxy test failed with status: {}",
            response.status()
        ))
    }
}

/// Read proxy settings from database.
///
/// This is a public function that can be used by any module needing proxy configuration.
/// Returns (proxy_mode, proxy_url) tuple.
///
/// # Arguments
/// * `db_state` - Database state to read proxy settings from
///
/// # Returns
/// Tuple of (proxy_mode, proxy_url)
pub async fn get_proxy_from_settings(
    db_state: &SqliteDbState,
) -> Result<(ProxyMode, String), String> {
    let settings = load_settings_from_sqlite_state(db_state)?;
    Ok((ProxyMode::parse(&settings.proxy_mode), settings.proxy_url))
}

/// Build a reqwest::Proxy from URL string.
///
/// Supports:
/// - HTTP proxy: http://[user:pass@]host:port
/// - HTTPS proxy: https://[user:pass@]host:port
/// - SOCKS5 proxy: socks5://[user:pass@]host:port
///
/// Auto-detects protocol from URL scheme.
fn build_proxy(url: &str) -> Result<Option<Proxy>, String> {
    if url.is_empty() {
        return Ok(None);
    }

    let normalized_url = normalize_proxy_url(url);

    // Use Proxy::all() to apply proxy to all protocols (HTTP and HTTPS)
    let proxy =
        Proxy::all(&normalized_url).map_err(|e| format!("Invalid proxy URL '{}': {}", url, e))?;

    Ok(Some(proxy))
}

/// Normalize proxy URL by ensuring it has a scheme.
///
/// If no scheme is provided, defaults to http://
fn normalize_proxy_url(url: &str) -> String {
    let url_lower = url.to_lowercase();

    if url_lower.starts_with("http://")
        || url_lower.starts_with("https://")
        || url_lower.starts_with("socks5://")
        || url_lower.starts_with("socks5h://")
    {
        url.to_string()
    } else {
        // Default to http:// if no scheme provided
        format!("http://{}", url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn account_client_reuses_connection_and_observes_proxy_change() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let cache = Mutex::new(None);
            let direct_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("direct listener");
            let direct_url = format!(
                "http://127.0.0.1:{}/probe",
                direct_listener.local_addr().unwrap().port()
            );
            let direct_server = tokio::spawn(async move {
                let (mut socket, _) = direct_listener.accept().await.expect("one connection");
                for _ in 0..2 {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        socket.read_exact(&mut byte).await.expect("request byte");
                        request.push(byte[0]);
                    }
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await
                        .expect("response");
                }
            });

            let first =
                account_client_for_proxy_with_cache(&cache, ProxyMode::Direct, String::new())
                    .await
                    .expect("direct client");
            let second =
                account_client_for_proxy_with_cache(&cache, ProxyMode::Direct, String::new())
                    .await
                    .expect("cached client");
            for client in [first, second] {
                assert_eq!(
                    client
                        .get(&direct_url)
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap(),
                    "ok"
                );
            }
            tokio::time::timeout(Duration::from_secs(3), direct_server)
                .await
                .expect("direct requests reused one TCP connection")
                .expect("direct server");

            let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("proxy listener");
            let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
            let proxy_server = tokio::spawn(async move {
                let (mut socket, _) = proxy_listener.accept().await.expect("proxied connection");
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).await.expect("request byte");
                    request.push(byte[0]);
                }
                assert!(String::from_utf8_lossy(&request).contains("http://example.invalid/probe"));
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nproxy")
                    .await
                    .expect("proxy response");
            });
            let custom = account_client_for_proxy_with_cache(&cache, ProxyMode::Custom, proxy_url)
                .await
                .expect("custom client");
            assert_eq!(
                custom
                    .get("http://example.invalid/probe")
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "proxy"
            );
            tokio::time::timeout(Duration::from_secs(3), proxy_server)
                .await
                .expect("proxy settings took effect")
                .expect("proxy server");
        });
    }

    #[test]
    fn test_normalize_proxy_url() {
        assert_eq!(
            normalize_proxy_url("proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("http://proxy.example.com:8080"),
            "http://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("HTTP://proxy.example.com:8080"),
            "HTTP://proxy.example.com:8080"
        );
        assert_eq!(
            normalize_proxy_url("socks5://proxy.example.com:1080"),
            "socks5://proxy.example.com:1080"
        );
        assert_eq!(
            normalize_proxy_url("user:pass@proxy.example.com:8080"),
            "http://user:pass@proxy.example.com:8080"
        );
    }

    #[test]
    fn test_build_proxy_empty() {
        let result = build_proxy("");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_build_proxy_http() {
        let result = build_proxy("http://proxy.example.com:8080");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_build_proxy_socks5() {
        let result = build_proxy("socks5://proxy.example.com:1080");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_build_proxy_with_auth() {
        let result = build_proxy("http://user:password@proxy.example.com:8080");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    /// `describe_error` exists because reqwest's `Display` hides the cause. A
    /// refused socket (dead local proxy) must name the OS error, otherwise the
    /// stored usage failure is unactionable.
    #[test]
    fn describe_error_keeps_the_root_cause() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let error = runtime.block_on(async {
            // Port 1 has no listener; mirrors "Clash sidecar not running".
            reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(2))
                .build()
                .expect("client")
                .get("https://127.0.0.1:1/usage")
                .send()
                .await
                .expect_err("connect must fail")
        });

        assert_eq!(
            error.to_string(),
            "error sending request for url (https://127.0.0.1:1/usage)",
            "reqwest Display is the terse form this helper exists to improve"
        );
        let described = describe_error(&error);
        assert!(
            described.len() > error.to_string().len(),
            "root cause must be appended: {described}"
        );
        assert!(
            described.to_lowercase().contains("connect"),
            "a connect failure must say so: {described}"
        );
        assert!(is_transient_transport_error(&error));
    }

    #[test]
    fn describe_error_redacts_url_credentials_and_query() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let error = runtime.block_on(async {
            create_client_no_proxy(2)
                .expect("client")
                .get("http://user:secret@127.0.0.1:1/usage?token=private")
                .send()
                .await
                .expect_err("connect must fail")
        });
        let described = describe_error(&error);
        assert!(described.contains("127.0.0.1:1/usage"));
        assert!(!described.contains("secret"), "{described}");
        assert!(!described.contains("private"), "{described}");
    }
}
