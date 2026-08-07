use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use crate::oauth::make_oauth_check;
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use cookie::time::OffsetDateTime;
use http_body::Body;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{Method, Request, Response, Result, StatusCode};
use log::{debug, error, info};
use shared::google_oauth::{decode_oauth_callback, get_oauth_url, OauthCheckRequest};
use shared::jwt::{
    generate_state_token, generate_token, validate_state_token, validate_token, InboxNegativeJWT,
    AUTH_COOKIE_NAME, STATE_COOKIE_NAME,
};
use tokio::net::TcpListener;

use crate::addresses::hash_email_simple;
use crate::attachments::AttachmentStore;
use crate::deleted::EmailStats;
use crate::zmq_helpers;

fn get_google_client_id() -> String {
    std::env::var("GOOGLE_CLIENT_ID").unwrap_or_else(|_| "test-client-id".to_string())
}

fn get_google_client_secret() -> String {
    std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_else(|_| "test-client-secret".to_string())
}

/// Parse the `Cookie` header. Delegates to `shared::cookies`, which splits on the
/// first `=` and skips malformed pairs.
///
/// This previously indexed `parts[1]` after splitting, so a header with no `=` in
/// it -- `Cookie: novalue` -- panicked the request handler with an index-out-of-
/// bounds. It also `unwrap()`ed `to_str()`, which panics on non-UTF-8 header bytes.
/// Both are unauthenticated and one header away, and `/api/session` is reached
/// through here, so neither is acceptable on this path.
fn extract_cookies(req: &Request<impl Body>) -> HashMap<String, String> {
    req.headers()
        .get_all("Cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(shared::cookies::all_raw)
        .collect()
}

fn is_authenticated(
    req: &Request<impl Body>,
) -> std::result::Result<InboxNegativeJWT, Box<dyn Error>> {
    let cooks = extract_cookies(req);

    // 403 if it dosen't have an auth cookie
    if !cooks.contains_key(AUTH_COOKIE_NAME) {
        return Err("No Auth Cookie".into());
    }

    // 403 if the auth cookie is no good
    let jwt = cooks.get(AUTH_COOKIE_NAME).unwrap();
    let jwt_dec = match validate_token(jwt.clone()) {
        Ok(j) => j,
        Err(e) => {
            error!("Invalid JWT: {}", e);
            return Err("Invalid JWT".into());
        }
    };
    Ok(jwt_dec)
}

pub async fn run_http_server(
    address: SocketAddr,
    term_flag: Arc<AtomicBool>,
    email_stats: Arc<RwLock<EmailStats>>,
    attachment_store: Arc<AttachmentStore>,
    testing_mode: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Create socket with SO_REUSEADDR to allow binding even if connections are in TIME_WAIT
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;

    let listener = TcpListener::from_std(socket.into())?;
    info!("Listening on http://{} (SO_REUSEADDR enabled)", address);

    let app = build_app(AppState {
        email_stats,
        attachment_store,
        testing_mode,
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(term_flag))
        .await?;

    Ok(())
}

/// Resolves when the shared termination flag is set (SIGTERM/SIGINT), driving
/// axum's graceful shutdown.
async fn shutdown_signal(term_flag: Arc<AtomicBool>) {
    while !term_flag.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    info!("Received SIGTERM/SIGINT, shutting down HTTP server.");
}

/// Shared state handed to the axum router.
#[derive(Clone)]
pub struct AppState {
    pub email_stats: Arc<RwLock<EmailStats>>,
    pub attachment_store: Arc<AttachmentStore>,
    pub testing_mode: bool,
}

/// Build the axum application. Dispatch is delegated to `response_routing`, so
/// request-handling behavior is identical to the previous hyper server; this
/// swaps only the serving layer and makes the app testable in-process.
pub fn build_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .fallback(handle_request)
        .with_state(state)
}

/// Bridges an axum request into `response_routing` and converts the resulting
/// body back into an axum response body.
async fn handle_request(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    match response_routing(
        req,
        state.email_stats,
        state.attachment_store,
        state.testing_mode,
    )
    .await
    {
        Ok(resp) => {
            let (parts, body) = resp.into_parts();
            axum::response::Response::from_parts(parts, axum::body::Body::new(body))
        }
        Err(e) => {
            error!("Error handling request: {}", e);
            axum::response::Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from("Internal Server Error"))
                .unwrap()
        }
    }
}

async fn response_routing(
    req: Request<impl Body>,
    email_stats: Arc<RwLock<EmailStats>>,
    attachment_store: Arc<AttachmentStore>,
    testing_mode: bool,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
    if req.uri().path().starts_with("/api") {
        return match (req.method(), req.uri().path()) {
            (&Method::GET, "/api/health") => Ok(health_check(&req)),
            (&Method::GET, "/api/version") => Ok(version_info()),
            (&Method::GET, "/api/session") => Ok(session_info(&req)),
            (&Method::GET, "/api/oauth") => oauth_redirect_target(req).await,
            (&Method::GET, "/api/stats") => Ok(get_stats(req, email_stats)),
            (&Method::GET, "/api/email") => simple_stream_send_new(req, attachment_store).await,
            (&Method::GET, "/api/logout") => Ok(do_logout()),
            (&Method::GET, "/api/test-login") if testing_mode => Ok(create_test_login()),
            (&Method::GET, path) if path.starts_with("/api/attachment/") => {
                get_attachment(req, attachment_store).await
            }
            _ => Ok(not_found()),
        };
    }

    // By default, serve files from the current directory
    simple_file_send(req.uri().path()).await
}

fn get_stats(
    req: Request<impl Body>,
    email_stats: Arc<RwLock<EmailStats>>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    // Check if the user is authenticated
    let jwt_dec = is_authenticated(&req).ok();

    let email = jwt_dec.map(|j| j.email);

    // Get stats with error handling
    let stats_result = {
        let stats = email_stats.read().unwrap();
        stats.get_user_stats(email.as_deref())
    };

    match stats_result {
        Ok(user_stats) => match serde_json::to_string(&user_stats) {
            Ok(json_string) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(full_body(Bytes::from(json_string)))
                .unwrap(),
            Err(e) => {
                error!("Error serializing stats: {}", e);
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(full_body(Bytes::from("Error processing stats")))
                    .unwrap()
            }
        },
        Err(e) => {
            error!("Error getting user stats: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(full_body(Bytes::from("Error retrieving stats")))
                .unwrap()
        }
    }
}

fn health_check(req: &Request<impl Body>) -> Response<BoxBody<Bytes, std::io::Error>> {
    debug!("Responding to Health check {:?}", req.headers());
    Response::builder()
        .status(StatusCode::OK)
        .body(Full::new("OK\n".into()).map_err(|e| match e {}).boxed())
        .unwrap()
}

/// Public root this service is reached on. Used to build the OAuth redirect back
/// to ourselves; it was previously hardcoded in the frontend.
const PUBLIC_ROOT: &str = "https://inboxnegative.com";

/// Everything the frontend needs to render, so it never has to read a cookie.
///
/// This is what makes `HttpOnly` possible on both cookies. The identity here comes
/// from `validate_token`, which verifies the signature -- the frontend previously
/// used `unsafe_decode_token`, so its notion of who you were came from a token it
/// never checked.
///
/// Unauthenticated by design: signed out, it returns `email: null` plus a login
/// URL, which is exactly what a logged-out visitor needs.
fn session_info(req: &Request<impl Body>) -> Response<BoxBody<Bytes, std::io::Error>> {
    let email = is_authenticated(req).ok().map(|jwt| jwt.email);

    // Prefer the state token already in the request, so the `state` in the OAuth
    // URL matches the cookie the browser holds. Minting a fresh one is a correct
    // fallback -- `oauth_redirect_target` validates the token by signature, not by
    // comparing it to the cookie -- but keeping them equal preserves the option of
    // enforcing that match later as CSRF protection.
    let state = extract_cookies(req)
        .get(STATE_COOKIE_NAME)
        .cloned()
        .or_else(|| generate_state_token(get_google_client_id(), Duration::days(1)).ok());

    let session = shared::session::SessionInfo {
        email,
        oauth_url: state
            .map(|state| get_oauth_url(PUBLIC_ROOT, &state, &get_google_client_id()))
            .unwrap_or_default(),
    };

    match serde_json::to_string(&session) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            // Identity varies per request; a cache must never serve one user's
            // session document to another.
            .header("Cache-Control", "no-store")
            .body(full_body(Bytes::from(json)))
            .unwrap(),
        Err(e) => {
            error!("Error serializing session info: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(full_body(Bytes::from("Error processing session")))
                .unwrap()
        }
    }
}

/// Build provenance of the running binary. Unauthenticated on purpose: it reveals
/// only the revision of an already-public repository, and gating it would defeat
/// the point of being able to ask a running container what it is.
fn version_info() -> Response<BoxBody<Bytes, std::io::Error>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(full_body(Bytes::from(crate::build_info::as_json())))
        .unwrap()
}

fn full_body(contents: Bytes) -> BoxBody<Bytes, std::io::Error> {
    Full::new(contents).map_err(|e| match e {}).boxed()
}

/// HTTP status code 404
fn not_found() -> Response<BoxBody<Bytes, std::io::Error>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(full_body(Bytes::from_static(b"Not Found")))
        .unwrap()
}

fn forbidden(contents: String) -> Response<BoxBody<Bytes, std::io::Error>> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(full_body(contents.into()))
        .unwrap()
}

#[allow(dead_code)]
fn datetime_to_rfc2616(dt: DateTime<chrono::Utc>) -> String {
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn do_logout() -> Response<BoxBody<Bytes, std::io::Error>> {
    make_login_logout_response(
        "".to_string(),
        DateTime::from_timestamp_nanos(0),
        "Logging out...".to_string(),
    )
}

fn create_test_login() -> Response<BoxBody<Bytes, std::io::Error>> {
    let test_email = "testing@gmail.com";
    // Hash email to match production flow
    let hashed_email = hash_email_simple(test_email);

    // Create token valid for 24 hours
    let jwt_enc = generate_token(&hashed_email, Duration::days(1)).unwrap();
    let jwt_dec = validate_token(jwt_enc.clone()).unwrap();
    let expiry_dt = DateTime::from_timestamp(jwt_dec.expiry, 0).unwrap();

    info!(
        "🧪 Creating test login for: {} (hashed: {})",
        test_email, hashed_email
    );

    // Create cookie directly to ensure it's set correctly for localhost testing
    let cookie_contents = make_auth_cookie(jwt_enc, expiry_dt);

    // Add state cookie as well to ensure frontend works correctly
    let state_cookie = make_state_cookie();

    Response::builder()
        .status(StatusCode::FOUND)
        .header("Set-Cookie", cookie_contents)
        .header("Set-Cookie", state_cookie) // Add state cookie too
        .header("Location", "/index.html")
        .body(
            Full::new("Test login activated - redirecting to app...".into())
                .map_err(|e| match e {})
                .boxed(),
        )
        .unwrap()
}

fn make_auth_cookie(jwt_encoded: String, expiry: DateTime<Utc>) -> String {
    let converted_time = OffsetDateTime::from_unix_timestamp(expiry.timestamp()).unwrap();

    // Check if we're running in testing mode to avoid setting domain (which doesn't work on localhost)
    let is_testing =
        std::env::var("TESTING_MODE").unwrap_or_else(|_| "false".to_string()) == "true";

    // `Secure` outside testing mode: production is HTTPS-only through Traefik, so
    // without it the browser would send this JWT in cleartext to anything answering
    // plain HTTP -- which is not hypothetical while port 8080 is reachable directly
    // (see issue #16). `SameSite=Strict` does not cover this; it constrains
    // cross-site requests, not transport. Testing mode is the one path that is
    // legitimately plain HTTP on localhost, where `Secure` would suppress the cookie.
    //
    // `HttpOnly` since #18: the frontend no longer reads this cookie. It gets the
    // signed-in address from `/api/session`, so the token itself is opaque to JS.
    // This is what bounds a `clean_html` bypass to defacement rather than session
    // theft, on a service that renders HTML from arbitrary senders.
    let mut builder = cookie::CookieBuilder::new(AUTH_COOKIE_NAME, jwt_encoded)
        .same_site(cookie::SameSite::Strict)
        .secure(!is_testing)
        .http_only(true)
        .path("/")
        .expires(converted_time);

    // Only set domain in production mode
    if !is_testing {
        builder = builder.domain("inboxnegative.com");
    }

    builder.to_string()
}

fn make_login_logout_response(
    jwt_encoded: String,
    expiry: DateTime<Utc>,
    content: String,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    let cookie_contents = make_auth_cookie(jwt_encoded, expiry);
    Response::builder()
        .status(StatusCode::FOUND)
        .header("Set-Cookie", cookie_contents)
        .header("Location", "/index.html")
        .body(Full::new(content.into()).map_err(|e| match e {}).boxed())
        .unwrap()
}

fn make_state_cookie() -> String {
    let duration = Duration::days(1);
    let state_token = generate_state_token(get_google_client_id(), duration).unwrap();

    // Check if we're running in testing mode
    let is_testing =
        std::env::var("TESTING_MODE").unwrap_or_else(|_| "false".to_string()) == "true";

    // Also `HttpOnly` since #18. The frontend used to read this to extract the OAuth
    // client ID and build a login URL; `/api/session` now returns a finished URL, so
    // there is nothing left for JS to want here.
    let mut builder = cookie::CookieBuilder::new(STATE_COOKIE_NAME, state_token)
        .same_site(cookie::SameSite::Strict)
        .secure(!is_testing)
        .http_only(true)
        .path("/")
        .expires(OffsetDateTime::from_unix_timestamp((Utc::now() + duration).timestamp()).unwrap());

    // Only set domain in production mode
    if !is_testing {
        builder = builder.domain("inboxnegative.com");
    }

    builder.to_string()
}

async fn oauth_redirect_target(
    req: Request<impl Body>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
    // Parse the URI and extract query parameters
    let uri = req.uri().to_string();
    let cb_info = match decode_oauth_callback(&uri) {
        Ok(info) => info,
        Err(_) => {
            return Ok(forbidden(format!("Invalid Callback: {}", uri)));
        }
    };

    match validate_state_token(&cb_info.state) {
        Ok(_) => {}
        Err(e) => {
            error!("Invalid state token: {}", e);
            return Ok(forbidden("Invalid State Token".to_string()));
        }
    }
    debug!(
        "OAUTH Call code: {} state: {}",
        &cb_info.code, &cb_info.state
    );
    // Form the POST request that exchanges the token for th user credentials
    let oauth_req = OauthCheckRequest::new(
        cb_info.code,
        get_google_client_id(),
        get_google_client_secret(),
        "https://inboxnegative.com/api/oauth".to_string(),
    );

    // Check with google that the callback code is legit
    let google_email_address = make_oauth_check(oauth_req).await.unwrap();

    // Determine the inboxnegative email address associated with this account
    let inboxnull_email = hash_email_simple(&google_email_address);

    // Build the JWT for the user
    let jwt_enc = generate_token(&inboxnull_email, Duration::days(1)).unwrap();
    let jwt_dec = validate_token(jwt_enc.clone()).unwrap();
    let expiry_dt = DateTime::from_timestamp(jwt_dec.expiry, 0).unwrap();

    // Redirect to the main page w/ the JWT
    Ok(make_login_logout_response(
        jwt_enc,
        expiry_dt,
        "Logged in...redirecting shortly...".into(),
    ))
}

async fn get_attachment(
    req: Request<impl Body>,
    attachment_store: Arc<AttachmentStore>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
    // Check if the user is authenticated
    let _jwt_dec = match is_authenticated(&req) {
        Ok(j) => j,
        Err(err) => {
            return Ok(forbidden(format!("403 Forbidden: {}", err)));
        }
    };

    // Extract attachment ID from path
    let path = req.uri().path();
    let attachment_id = path.trim_start_matches("/api/attachment/");

    // Get attachment from store
    match attachment_store.get(attachment_id) {
        Some(attachment) => {
            let response = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", attachment.content_type.clone())
                .header(
                    "Content-Disposition",
                    format!(
                        "attachment; filename=\"{}\"",
                        attachment
                            .filename
                            .unwrap_or_else(|| "download".to_string())
                    ),
                )
                .body(full_body(Bytes::from(attachment.data)))
                .unwrap();

            Ok(response)
        }
        None => Ok(not_found()),
    }
}

async fn simple_stream_send_new(
    req: Request<impl Body>,
    attachment_store: Arc<AttachmentStore>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
    // Check if the user is authenticated
    let jwt_dec = match is_authenticated(&req) {
        Ok(j) => j,
        Err(err) => {
            return Ok(forbidden(format!("403 Forbidden: {}", err)));
        }
    };

    let socket = match zmq_helpers::connect_to_publisher(jwt_dec.email.as_str()) {
        Ok(socket) => socket,
        Err(e) => {
            error!("Unable to connect to publisher: {}", e);
            return Ok(not_found()); // TODO return a 500 error
        }
    };

    let zmq_stream_body = zmq_helpers::ZMQStreamBody::new(socket, attachment_store);
    let body_stream = BoxBody::new(zmq_stream_body);

    // Send response
    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream; charset=utf-8")
        .header("Transfer-Encoding", "chunked")
        .header("Cache-Control", "no-cache")
        .header("Allow-Origin", "*")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, PUT, POST")
        .header(
            "Access-Control-Allow-Headers",
            "Origin, X-Requested-With, Content-Type, Accept",
        )
        .body(body_stream)
        .unwrap();

    Ok(response)
}

/// Frontend assets (Trunk output). In release builds they are embedded into
/// the binary (single self-contained artifact, no filesystem dependency). In
/// debug builds rust-embed reads them from `frontend/dist` on disk for fast
/// iteration, so the frontend must be built before running.
#[derive(rust_embed::RustEmbed)]
#[folder = "../frontend/dist"]
struct FrontendAssets;

async fn simple_file_send(filename: &str) -> Result<Response<BoxBody<Bytes, std::io::Error>>> {
    debug!("Request for file {}", filename);

    let fil = match filename {
        "" => "index.html",
        "/" => "index.html",
        f => &f[1..],
    };

    let asset = match FrontendAssets::get(fil) {
        Some(asset) => asset,
        None => {
            error!("Embedded asset not found: {}", fil);
            return Ok(not_found());
        }
    };

    let mime = mime_guess::from_path(fil).first_or_octet_stream();
    let body = full_body(Bytes::from(asset.data.into_owned()));

    let builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", mime.to_string());

    // Set the state cookie on index.html only (preserves the OAuth state flow).
    let builder_headers = if fil == "index.html" {
        builder.header("Set-Cookie", make_state_cookie())
    } else {
        builder
    };

    Ok(builder_headers.body(body).unwrap())
}

mod tests {
    use super::*;

    use http_body::Body;
    use hyper::{Request, Uri};
    use std::str::FromStr;

    #[allow(dead_code)]
    struct MockBody {
        data: &'static [u8],
    }

    #[allow(dead_code)]
    impl MockBody {
        fn new(data: &'static [u8]) -> Self {
            Self { data }
        }

        fn empty() -> Self {
            Self { data: &[] }
        }
    }

    impl Body for MockBody {
        type Data = Bytes;
        type Error = hyper::Error;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Bytes>>>> {
            if self.data.is_empty() {
                std::task::Poll::Ready(None)
            } else {
                let data = self.data;
                self.data = &[];
                std::task::Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(data)))))
            }
        }
    }

    #[test]
    fn test_datetime_encoding() {
        let dt = chrono::DateTime::from_timestamp(0, 0).unwrap();
        let encoded = datetime_to_rfc2616(dt);
        assert_eq!(encoded, "Thu, 01 Jan 1970 00:00:00 GMT");
    }

    #[test]
    fn test_health_check() {
        let body: MockBody = MockBody::new(b"anything");

        // Build the request using Request::builder
        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri("/api/health") // Set the URI for the request
            .body(body) // Body can be set as empty for a GET request
            .unwrap();

        let response = health_check(&req);
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_not_found() {
        let email_stats = Arc::new(RwLock::new(EmailStats::new()));
        let attachment_store = Arc::new(AttachmentStore::new(300));
        let response = not_found();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri("/foo/bar/baz") // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        let response = response_routing(req, email_stats, attachment_store, false)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_static_assets() {
        crate::jwt_key::init_for_tests();
        let response = simple_file_send("/").await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = simple_file_send("/index.html").await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri("/index.html") // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        let email_stats = Arc::new(RwLock::new(EmailStats::new()));
        let attachment_store = Arc::new(AttachmentStore::new(300));
        let response = response_routing(req, email_stats, attachment_store, false)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Both cookies carry a JWT, and production is HTTPS-only through Traefik.
    /// Without `Secure` the browser sends them in cleartext to anything answering
    /// plain HTTP, which port 8080 currently does (issue #16).
    #[test]
    #[serial_test::serial]
    fn cookies_are_secure_outside_testing_mode() {
        crate::jwt_key::init_for_tests();
        let restore = std::env::var("TESTING_MODE").ok();
        // SAFETY: #[serial] -- no other thread reads the environment concurrently.
        unsafe { std::env::remove_var("TESTING_MODE") };

        for raw in [
            make_auth_cookie("jwt".into(), Utc::now()),
            make_state_cookie(),
        ] {
            let parsed = cookie::Cookie::parse(raw.clone()).unwrap();
            assert_eq!(parsed.secure(), Some(true), "expected Secure in {raw}");
        }

        unsafe {
            match restore {
                Some(v) => std::env::set_var("TESTING_MODE", v),
                None => std::env::remove_var("TESTING_MODE"),
            }
        }
    }

    /// The inverse matters just as much: testing mode is plain HTTP on localhost,
    /// where a `Secure` cookie is silently dropped by the browser and login breaks.
    #[test]
    #[serial_test::serial]
    fn cookies_are_not_secure_in_testing_mode() {
        crate::jwt_key::init_for_tests();
        let restore = std::env::var("TESTING_MODE").ok();
        // SAFETY: as above.
        unsafe { std::env::set_var("TESTING_MODE", "true") };

        for raw in [
            make_auth_cookie("jwt".into(), Utc::now()),
            make_state_cookie(),
        ] {
            let parsed = cookie::Cookie::parse(raw.clone()).unwrap();
            assert_ne!(parsed.secure(), Some(true), "unexpected Secure in {raw}");
        }

        unsafe {
            match restore {
                Some(v) => std::env::set_var("TESTING_MODE", v),
                None => std::env::remove_var("TESTING_MODE"),
            }
        }
    }

    /// The inverse of the guard this replaces. `HttpOnly` is what bounds a
    /// `clean_html` bypass to defacement instead of session theft, so it must stay
    /// set on both cookies -- and unlike `Secure` it applies in testing mode too,
    /// since it has nothing to do with transport.
    #[test]
    #[serial_test::serial]
    fn cookies_are_httponly_in_every_mode() {
        crate::jwt_key::init_for_tests();
        let restore = std::env::var("TESTING_MODE").ok();

        for mode in ["true", "false"] {
            // SAFETY: as above.
            unsafe { std::env::set_var("TESTING_MODE", mode) };
            for raw in [
                make_auth_cookie("jwt".into(), Utc::now()),
                make_state_cookie(),
            ] {
                let parsed = cookie::Cookie::parse(raw.clone()).unwrap();
                assert_eq!(
                    parsed.http_only(),
                    Some(true),
                    "TESTING_MODE={mode}: expected HttpOnly in {raw}"
                );
            }
        }

        // SAFETY: as above.
        unsafe {
            match restore {
                Some(v) => std::env::set_var("TESTING_MODE", v),
                None => std::env::remove_var("TESTING_MODE"),
            }
        }
    }

    /// The acceptance criterion from #18, enforced rather than remembered: if the
    /// frontend reads the auth cookie again, `HttpOnly` silently logs everyone out.
    /// A grep is the cheapest check that survives people forgetting why.
    #[test]
    fn frontend_never_reads_the_auth_cookie() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("frontend")
            .join("src");

        let mut offenders = Vec::new();
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("frontend/src exists") {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            checked += 1;
            let body = std::fs::read_to_string(&path).unwrap();
            if body.contains("AUTH_COOKIE_NAME") || body.contains("unsafe_decode_token") {
                offenders.push(path.display().to_string());
            }
        }

        assert!(checked > 0, "expected to scan some frontend sources");
        assert!(
            offenders.is_empty(),
            "frontend must not read identity from cookies -- see #18: {offenders:?}"
        );
    }

    #[tokio::test]
    async fn test_index_sets_cookie() {
        crate::jwt_key::init_for_tests();
        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri("/index.html") // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        let email_stats = Arc::new(RwLock::new(EmailStats::new()));
        let attachment_store = Arc::new(AttachmentStore::new(300));
        let response = response_routing(req, email_stats, attachment_store, false)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let set_cookie_resp = response.headers().get("Set-Cookie").unwrap();

        let parsed = cookie::Cookie::parse(set_cookie_resp.to_str().unwrap()).unwrap();

        assert!(validate_state_token(parsed.value()).is_ok());

        assert_eq!(parsed.name(), STATE_COOKIE_NAME);
        assert_eq!(parsed.path(), Some("/"));
        debug!("Expires: {:?}", parsed);
        assert!(parsed.expires().is_some());
        let expires = parsed.expires_datetime().unwrap().unix_timestamp();
        assert!(expires > Utc::now().timestamp() + (23 * 60 * 60));
    }

    #[tokio::test]
    async fn test_logout_token_expiry() {
        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri("/api/logout") // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        let email_stats = Arc::new(RwLock::new(EmailStats::new()));
        let attachment_store = Arc::new(AttachmentStore::new(300));
        let response = response_routing(req, email_stats, attachment_store, false)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);

        let set_cookie_resp = response.headers().get("Set-Cookie").unwrap();

        let parsed = cookie::Cookie::parse(set_cookie_resp.to_str().unwrap()).unwrap();

        assert_eq!(parsed.name(), AUTH_COOKIE_NAME);
        assert_eq!(parsed.path(), Some("/"));
        assert!(parsed.expires().is_some());
        let expires = parsed.expires_datetime().unwrap().unix_timestamp();
        assert_eq!(expires, 0);
    }

    // TODO(meawoppl) this test is broken
    // Not sure how to mock the google oauth response?
    async fn _test_oauth_flow() {
        // Example URI
        let jwt = generate_state_token("foobar".to_string(), Duration::days(1)).unwrap();
        let uri = Uri::from_str(
            format!(
                "http://www.inboxnegative.com/api/oauth?code=abcd&state={}",
                jwt
            )
            .as_str(),
        )
        .unwrap();

        // Build the request using Request::builder
        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri(uri) // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        let response = oauth_redirect_target(req).await.unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);
        assert!(response.headers().get("Location").is_some());
        assert!(response.headers().get("Set-Cookie").is_some());
    }

    #[tokio::test]
    async fn test_oauth_flow_invalid_state() {
        // Example URI
        let query_string = "/api/oauth?code=4%2F0AVG7fiTTp-xZ8JtOg2o1tRkG3vnJ79Yqm8a90De9Ii0Bf5cZb3O-NiEB504tfgpsOuXmtA&scope=email+openid+https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.email&authuser=0&hd=exclosure.io&prompt=consent".to_string();

        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri(query_string) // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        // No State Token
        let response = oauth_redirect_target(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let email_stats = Arc::new(RwLock::new(EmailStats::new()));

        let req: Request<MockBody> = Request::builder()
            .method("GET") // You can specify other methods like POST, PUT, DELETE, etc.
            .uri("/api/stats") // Set the URI for the request
            .body(MockBody::empty()) // Body can be set as empty for a GET request
            .unwrap();

        let response = get_stats(req, email_stats);
        assert_eq!(response.status(), StatusCode::OK);
    }

    // The /api/test-login backdoor must only exist when testing_mode is on.
    #[tokio::test]
    async fn test_test_login_disabled_outside_testing_mode() {
        let email_stats = Arc::new(RwLock::new(EmailStats::new()));
        let attachment_store = Arc::new(AttachmentStore::new(300));
        let req: Request<MockBody> = Request::builder()
            .method("GET")
            .uri("/api/test-login")
            .body(MockBody::empty())
            .unwrap();
        let response = response_routing(req, email_stats, attachment_store, false)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_test_login_enabled_in_testing_mode() {
        crate::jwt_key::init_for_tests();
        let email_stats = Arc::new(RwLock::new(EmailStats::new()));
        let attachment_store = Arc::new(AttachmentStore::new(300));
        let req: Request<MockBody> = Request::builder()
            .method("GET")
            .uri("/api/test-login")
            .body(MockBody::empty())
            .unwrap();
        let response = response_routing(req, email_stats, attachment_store, true)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers().get("Location").unwrap(), "/index.html");

        // A valid auth cookie must be issued.
        let auth_cookie = response
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|v| cookie::Cookie::parse(v.to_str().unwrap()).ok())
            .find(|c| c.name() == AUTH_COOKIE_NAME)
            .expect("test login issues an auth cookie");
        assert!(validate_token(auth_cookie.value().to_string()).is_ok());
    }

    #[test]
    fn test_is_authenticated_missing_cookie() {
        let req: Request<MockBody> = Request::builder()
            .method("GET")
            .uri("/api/stats")
            .body(MockBody::empty())
            .unwrap();
        assert!(is_authenticated(&req).is_err());
    }

    #[test]
    fn test_is_authenticated_valid_cookie() {
        crate::jwt_key::init_for_tests();
        let email_hash = hash_email_simple("person@example.com");
        let token = generate_token(&email_hash, Duration::days(1)).unwrap();
        let req: Request<MockBody> = Request::builder()
            .method("GET")
            .uri("/api/stats")
            .header("Cookie", format!("{}={}", AUTH_COOKIE_NAME, token))
            .body(MockBody::empty())
            .unwrap();
        let jwt = is_authenticated(&req).expect("valid auth cookie is accepted");
        assert_eq!(jwt.email, email_hash);
    }

    #[test]
    fn test_is_authenticated_tampered_cookie() {
        crate::jwt_key::init_for_tests();
        let email_hash = hash_email_simple("person@example.com");
        let token = generate_token(&email_hash, Duration::days(1)).unwrap();
        // Flip the final character to break the signature.
        let mut tampered = token.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        let req: Request<MockBody> = Request::builder()
            .method("GET")
            .uri("/api/stats")
            .header("Cookie", format!("{}={}", AUTH_COOKIE_NAME, tampered))
            .body(MockBody::empty())
            .unwrap();
        assert!(is_authenticated(&req).is_err());
    }

    #[allow(dead_code)]
    fn test_state(testing_mode: bool) -> AppState {
        AppState {
            email_stats: Arc::new(RwLock::new(EmailStats::new())),
            attachment_store: Arc::new(AttachmentStore::new(300)),
            testing_mode,
        }
    }

    // Drive the assembled axum app in-process via oneshot (no bound port),
    // exercising the full serving layer through to response_routing.
    #[tokio::test]
    async fn build_app_serves_health_via_oneshot() {
        use tower::ServiceExt;
        let app = build_app(test_state(false));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Signed out, `/api/session` must still answer with a usable login URL --
    /// that is the whole logged-out rendering path.
    #[tokio::test]
    async fn session_signed_out_returns_oauth_url_and_no_email() {
        use tower::ServiceExt;
        crate::jwt_key::init_for_tests();
        let resp = build_app(test_state(false))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/session")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("Cache-Control").unwrap(), "no-store");

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: shared::session::SessionInfo =
            serde_json::from_slice(&body).expect("valid SessionInfo JSON");

        assert_eq!(session.email, None);
        assert!(
            session
                .oauth_url
                .starts_with("https://accounts.google.com/"),
            "expected a Google OAuth URL, got {}",
            session.oauth_url
        );
        assert!(
            session.oauth_url.contains("state="),
            "{}",
            session.oauth_url
        );
    }

    /// Signed in, it reports the address from a *verified* token.
    #[tokio::test]
    async fn session_signed_in_reports_verified_email() {
        use tower::ServiceExt;
        crate::jwt_key::init_for_tests();
        let email_hash = "deadbeefdeadbeef@inboxnegative.com";
        let token = generate_token(email_hash, Duration::days(1)).unwrap();

        let resp = build_app(test_state(false))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/session")
                    .header("Cookie", format!("{AUTH_COOKIE_NAME}={token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: shared::session::SessionInfo = serde_json::from_slice(&body).unwrap();
        assert_eq!(session.email.as_deref(), Some(email_hash));
    }

    /// A forged token must read as signed out, not as that user. This is exactly
    /// the difference between `validate_token` and the `unsafe_decode_token` the
    /// frontend used to call on a cookie it could not verify.
    #[tokio::test]
    async fn session_ignores_a_forged_token() {
        use tower::ServiceExt;
        crate::jwt_key::init_for_tests();
        let resp = build_app(test_state(false))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/session")
                    .header("Cookie", format!("{AUTH_COOKIE_NAME}=not-a-real-token"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: shared::session::SessionInfo = serde_json::from_slice(&body).unwrap();
        assert_eq!(session.email, None, "a forged token must not authenticate");
    }

    /// `Cookie: novalue` used to panic the handler on an index-out-of-bounds, and
    /// `/api/session` is reached through the same parser.
    #[tokio::test]
    async fn malformed_cookie_header_does_not_panic() {
        use tower::ServiceExt;
        crate::jwt_key::init_for_tests();
        for header in ["novalue", "a=1; novalue; b=2", "=", "; ;"] {
            let resp = build_app(test_state(false))
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/session")
                        .header("Cookie", header)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "header {header:?}");
        }
    }

    /// Deployment tooling reads this to confirm what is serving, so it has to work
    /// unauthenticated and report the same revision the startup log line does.
    #[tokio::test]
    async fn build_app_serves_version_via_oneshot() {
        use tower::ServiceExt;
        let app = build_app(test_state(false));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/version")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "application/json"
        );

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains(crate::build_info::REVISION), "{body}");
        assert!(body.contains(crate::build_info::VERSION), "{body}");
    }

    #[tokio::test]
    async fn build_app_unknown_api_route_is_404() {
        use tower::ServiceExt;
        let app = build_app(test_state(false));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/nope")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn build_app_test_login_gated_through_axum() {
        use tower::ServiceExt;
        // testing_mode = false: the backdoor must 404 through the full axum stack.
        let app = build_app(test_state(false));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/test-login")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
