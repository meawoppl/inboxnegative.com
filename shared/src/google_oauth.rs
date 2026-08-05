use log::debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

pub fn get_oauth_url(root_url: &str, state: &str, client_id: &str) -> String {
    // Query parameters as a tuple of key-value pairs
    let oauth_redirect_to = format!("{}/api/oauth", root_url);

    let query_params = &[
        ("response_type", "code"),
        ("client_id", client_id),
        ("scope", "openid email"),
        ("redirect_uri", oauth_redirect_to.as_str()),
        ("state", state),
    ];

    // Create a URL and append the encoded query string
    let url = Url::parse_with_params("https://accounts.google.com/o/oauth2/v2/auth", query_params)
        .unwrap();

    debug!("Encoded OAuth URL: {}", url);

    url.to_string()
}
pub struct OauthCallbackContents {
    pub code: String,
    pub state: String,
}

impl OauthCallbackContents {
    pub fn new(code: String, state: String) -> Self {
        Self { code, state }
    }
}

pub fn decode_oauth_callback(
    path: &str,
) -> Result<OauthCallbackContents, Box<dyn std::error::Error>> {
    // Parse the URI and extract query parameters
    debug!("OAuth callback path: {}", path);
    let parsed_url = Url::parse(&format!("http://placeholder{}", path)).unwrap();
    let query_pairs: HashMap<_, _> = parsed_url.query_pairs().into_owned().collect();
    debug!("OAuth query parameters: {:?}", query_pairs);

    if !query_pairs.contains_key("state") {
        return Err("No state parameter found".into());
    }

    if !query_pairs.contains_key("code") {
        return Err("No code parameter found".into());
    }

    Ok(OauthCallbackContents::new(
        query_pairs.get("code").unwrap().clone(),
        query_pairs.get("state").unwrap().clone(),
    ))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OauthCheckRequest {
    pub code: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub grant_type: String,
}

impl OauthCheckRequest {
    pub fn new(
        code: String,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    ) -> Self {
        let grant_type = "authorization_code".to_string();
        Self {
            code,
            client_id,
            client_secret,
            redirect_uri,
            grant_type,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthCheckResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub id_token: String,
    pub scope: String,
    pub token_type: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OauthIdTokenContents {
    pub iss: String,          // Issuer
    pub azp: String,          // Authorized party
    pub aud: String,          // Audience
    pub sub: String,          // Subject
    pub email: String,        // Email
    pub email_verified: bool, // Email verified
    pub at_hash: String,      // Access token hash
    pub exp: i64,             // Expiration time
    pub iat: i64,             // Issued at
}
