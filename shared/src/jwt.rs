use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use jwt::{self, Header, Token};
use jwt::{SignWithKey, VerifyWithKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::error;
use uuid::Uuid;

// NOTE(meawoppl) we are a functionally stateless app, this could be
// generated any time the binary starts?
// TODO(meawoppl) move this const out of shared
const JWT_SECRET: &[u8] = b"01923503-b671-70c9-a020-c4903647f69b";

pub const STATE_COOKIE_NAME: &str = "inboxnegative_state";
pub const AUTH_COOKIE_NAME: &str = "inboxnegative_jwt";

#[derive(Debug, Serialize, Deserialize)]
pub struct OauthStateToken {
    pub client_id: String,
    pub uuid: Uuid,
    pub expiry: i64,
}

impl OauthStateToken {
    pub fn new(client_id: String, duration: Duration) -> OauthStateToken {
        OauthStateToken {
            client_id,
            uuid: Uuid::new_v4(),
            expiry: (Utc::now() + duration).timestamp(),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.expiry > Utc::now().timestamp()
    }
}

pub fn generate_state_token(
    client_id: String,
    duration: Duration,
) -> Result<String, Box<dyn error::Error>> {
    let state_token = OauthStateToken::new(client_id, duration);
    let secret = get_hmac();

    // `?`, not `.unwrap()`: this runs on every index.html request, since the state
    // cookie is set for every landing-page visitor. That is the hottest path in the
    // app and not a place to panic.
    let encoded = state_token.sign_with_key(&secret)?;
    Ok(encoded)
}

pub fn validate_state_token(token: &str) -> Result<OauthStateToken, Box<dyn error::Error>> {
    let secret = get_hmac();
    // `?`, not `.unwrap()`. This is reached with attacker-controlled input -- the
    // `state` query parameter of the unauthenticated /api/oauth callback -- so a
    // failed signature must be a rejection, not a panic. `validate_token` below
    // always got this right; this one did not, which silently defeated the caller's
    // own error handling at backend/src/http.rs:387.
    let state_token: OauthStateToken = token.verify_with_key(&secret)?;

    if !state_token.is_valid() {
        return Err("Token Expired".into());
    }

    Ok(state_token)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InboxNegativeJWT {
    pub email: String,
    pub expiry: i64,
}

fn get_hmac() -> Hmac<Sha256> {
    Hmac::new_from_slice(JWT_SECRET).unwrap()
}

pub fn generate_token(email: &str, duration: Duration) -> Result<String, Box<dyn error::Error>> {
    let ending = Utc::now() + duration;
    let secret = get_hmac();

    let claims = InboxNegativeJWT {
        email: email.to_string(),
        expiry: ending.timestamp(),
    };

    let encoded = claims.sign_with_key(&secret).unwrap();
    Ok(encoded)
}

pub fn validate_token(token: String) -> Result<InboxNegativeJWT, Box<dyn error::Error>> {
    let secret = get_hmac();
    let ibn_jwt: InboxNegativeJWT = token.verify_with_key(&secret)?;

    if ibn_jwt.expiry < Utc::now().timestamp() {
        return Err("Token Expired".into());
    }

    Ok(ibn_jwt)
}

pub fn unsafe_decode_token(token: String) -> Result<InboxNegativeJWT, Box<dyn error::Error>> {
    let decoded: Token<Header, InboxNegativeJWT, _> = match Token::parse_unverified(&token) {
        Ok(t) => t,
        Err(_) => {
            return Err("Failed to decode token".into());
        }
    };

    let jtw_ref = decoded.claims();

    Ok(InboxNegativeJWT {
        email: jtw_ref.email.clone(),
        expiry: jtw_ref.expiry,
    })
}

pub fn unsafe_decode_state_token(token: String) -> Result<OauthStateToken, Box<dyn error::Error>> {
    let decoded: Token<Header, OauthStateToken, _> = match Token::parse_unverified(&token) {
        Ok(t) => t,
        Err(_) => {
            return Err("Failed to decode state token".into());
        }
    };

    let state_ref = decoded.claims();

    Ok(OauthStateToken {
        client_id: state_ref.client_id.clone(),
        uuid: state_ref.uuid,
        expiry: state_ref.expiry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_generate_token() {
        let email = "foo.bar@bax.qux";
        let duration = Duration::days(1);
        let token = generate_token(email, duration).unwrap();
        let ibn_jwt = validate_token(token).unwrap();

        assert_eq!(ibn_jwt.email, email);
        assert!(ibn_jwt.expiry > Utc::now().timestamp());
    }

    #[test]
    fn test_jwt_expiry() {
        let encoded = generate_token("bsdfsdf", Duration::days(-1)).unwrap();
        if validate_token(encoded).is_ok() {
            panic!("Token should have expired")
        }
    }

    #[test]
    fn test_state_token_round_trip() {
        let token = generate_state_token("foo".to_string(), Duration::days(1)).unwrap();
        let state_token = validate_state_token(&token).unwrap();

        assert_eq!(state_token.client_id, "foo");
    }

    /// `validate_state_token` is reached with attacker-controlled input: the `state`
    /// query parameter of the unauthenticated `/api/oauth` callback. It used to
    /// `.unwrap()` the signature check, so every one of these panicked the request
    /// handler instead of returning the 403 the caller was already written to send.
    ///
    /// Verified to fail against the pre-fix code rather than assumed: restoring the
    /// `.unwrap()` makes this panic at `jwt.rs:59`.
    #[test]
    fn state_token_rejects_garbage_without_panicking() {
        for bad in [
            "",
            "not-a-jwt",
            "a.b.c",
            "../../etc/passwd",
            // Structurally valid JWT, signed with the wrong key.
            "eyJhbGciOiJIUzI1NiJ9.eyJjbGllbnRfaWQiOiJmb28iLCJleHBpcnkiOjk5OTk5OTk5OTl9.\
             AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(
                validate_state_token(bad).is_err(),
                "expected Err, not a panic, for {bad:?}"
            );
        }
    }

    /// A token whose signature verifies but which has expired must also be a plain
    /// `Err`. That path was already correct; pinned so the fix above cannot regress
    /// it into accepting stale tokens.
    #[test]
    fn state_token_rejects_expired() {
        let token = generate_state_token("foo".to_string(), Duration::days(-1)).unwrap();
        assert!(validate_state_token(&token).is_err());
    }

    /// The same guarantee for the auth token, which was already using `?`. Pinned so
    /// the two validators cannot drift apart again -- that asymmetry is exactly what
    /// hid the bug, since the correct one sat a few lines below the broken one.
    #[test]
    fn auth_token_rejects_garbage_without_panicking() {
        for bad in ["", "not-a-jwt", "a.b.c"] {
            assert!(
                validate_token(bad.to_string()).is_err(),
                "expected Err, not a panic, for {bad:?}"
            );
        }
    }
}
