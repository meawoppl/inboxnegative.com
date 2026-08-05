use jwt::{Header, Token};
use log::error;
use shared::google_oauth::{OAuthCheckResponse, OauthCheckRequest, OauthIdTokenContents};

pub async fn make_oauth_check(
    request: OauthCheckRequest,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::ClientBuilder::new().use_rustls_tls().build()?;

    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .json(&request)
        .send()
        .await?;

    if resp.status() != 200 {
        error!("Google OAuth check failed: {:?}", resp.text().await);
        return Err("Google OAuth Check Failed".into());
    }

    let decoded: OAuthCheckResponse = resp.json().await?;

    let token: Token<Header, OauthIdTokenContents, _> =
        Token::parse_unverified(decoded.id_token.as_str()).unwrap();

    let contents: &OauthIdTokenContents = token.claims();

    if !contents.email_verified {
        return Err("Google OAuth Check Failed: Email not verified".into());
    }

    Ok(contents.email.clone())
}
