//! What the frontend needs in order to render, supplied by the server.
//!
//! This type exists so neither cookie has to be readable from JavaScript. The
//! frontend used to pull the auth and state cookies out of `document.cookie` and
//! decode them client-side, which is what prevented `HttpOnly` from being set --
//! and `HttpOnly` is the control that bounds a sanitiser bypass to defacement
//! rather than session theft, on a service whose entire job is rendering HTML
//! from arbitrary senders.
//!
//! Both fields are derived server-side from a *verified* token. The client is no
//! longer deriving identity from a token it cannot check.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Burner address of the signed-in user, or `None` when signed out.
    ///
    /// Comes from `validate_token`, which checks the signature -- unlike the
    /// `unsafe_decode_token` the frontend previously used, where a writable
    /// cookie would have made the client's notion of identity attacker-supplied.
    pub email: Option<String>,

    /// Fully built Google OAuth URL, including the signed `state` parameter.
    ///
    /// Built server-side specifically so the frontend needs neither the state
    /// cookie nor the client ID. Handing over a finished link rather than the
    /// parts to assemble one is what lets the state cookie be `HttpOnly` too.
    pub oauth_url: String,
}
