//! Hash salts, supplied exclusively via the environment.
//!
//! Both salts feed user-visible hashes: `EMAIL_SALT` derives every burner
//! address handed out to users, and `DELETED_SALT` derives the `email_hash`
//! primary key in `email_stats`. Neither fails loudly on its own when wrong --
//! a missing or altered value simply produces *different* hashes, silently
//! orphaning every existing address and stats row.
//!
//! So there is deliberately no compiled-in fallback. The process refuses to
//! start unless both are present, which turns "set the environment before
//! deploying the new image" from a convention someone has to remember into an
//! invariant the binary enforces. A crash loop is recoverable; a silent rehash
//! discovered a week later is not.

use std::env;
use std::sync::OnceLock;

pub const EMAIL_SALT_VAR: &str = "EMAIL_SALT";
pub const DELETED_SALT_VAR: &str = "DELETED_SALT";

static EMAIL_SALT: OnceLock<String> = OnceLock::new();
static DELETED_SALT: OnceLock<String> = OnceLock::new();

fn read(var: &str) -> Result<String, String> {
    match env::var(var) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!(
            "{var} is set but empty. It must hold the exact salt value; \
             an empty salt would rehash every existing record."
        )),
        Err(_) => Err(format!(
            "{var} is not set. It is required -- there is no default, because \
             a wrong salt silently invalidates every existing hash."
        )),
    }
}

/// Load and validate both salts. Call once during startup, before anything
/// hashes, so a misconfigured environment fails at boot rather than at use.
pub fn init() -> Result<(), String> {
    let email = read(EMAIL_SALT_VAR)?;
    let deleted = read(DELETED_SALT_VAR)?;

    // set() only errors if already initialised, which is harmless here.
    let _ = EMAIL_SALT.set(email);
    let _ = DELETED_SALT.set(deleted);

    Ok(())
}

/// Fixed salts for unit tests, so the suite is hermetic and does not depend on
/// ambient environment. In release builds this is `None` for every variable,
/// so an uninitialised salt panics rather than silently hashing with a default.
#[cfg(test)]
fn test_fallback(var: &str) -> Option<&'static str> {
    match var {
        EMAIL_SALT_VAR => Some("TestSalt"),
        DELETED_SALT_VAR => Some("TestDeletedSalt"),
        _ => None,
    }
}

#[cfg(not(test))]
fn test_fallback(_var: &str) -> Option<&'static str> {
    None
}

fn get(cell: &'static OnceLock<String>, var: &str) -> &'static str {
    cell.get_or_init(|| {
        test_fallback(var)
            .unwrap_or_else(|| panic!("{var} was read before salt::init() ran"))
            .to_string()
    })
    .as_str()
}

/// Salt used to derive public burner addresses.
pub fn email_salt() -> &'static str {
    get(&EMAIL_SALT, EMAIL_SALT_VAR)
}

/// Salt used to derive the `email_stats.email_hash` primary key.
pub fn deleted_salt() -> &'static str {
    get(&DELETED_SALT, DELETED_SALT_VAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_rejects_unset_var() {
        let err = read("INBOXNULL_DEFINITELY_UNSET_VAR").unwrap_err();
        assert!(err.contains("is not set"), "unexpected message: {err}");
    }

    #[test]
    fn salts_are_stable_under_test() {
        // The suite must not depend on ambient environment for salt values.
        assert_eq!(email_salt(), "TestSalt");
        assert_eq!(deleted_salt(), "TestDeletedSalt");
    }
}
