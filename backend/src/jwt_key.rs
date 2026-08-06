//! Startup selection of the JWT signing key.
//!
//! Unlike the hash salts, this key is **safe to change**. It signs sessions and
//! nothing else -- no address, stats row, or stored record is derived from it. A
//! new key invalidates outstanding logins and costs users one re-authentication;
//! it does not orphan data. That asymmetry is why this module does the opposite of
//! `salt::init` and defaults to a fresh random key rather than refusing to boot.
//!
//! Defaulting to random matters here specifically because Watchtower deploys new
//! images unattended and has no health gating: a hard requirement for an env var
//! would turn "the variable was not staged first" into a silent crash loop that
//! nothing alerts on. Random-by-default always boots, and always boots secure.
//!
//! Set `JWT_SECRET` when you want sessions to survive a restart. Leave it unset
//! and every restart re-keys, which is safe but logs everyone out.

use std::env;

pub const JWT_SECRET_VAR: &str = "JWT_SECRET";

/// Length of a generated key. HMAC-SHA256's block size is 64 bytes and keys longer
/// than that are hashed down, so 32 bytes is the useful maximum -- it matches the
/// 256-bit output and gains nothing from being larger.
const GENERATED_KEY_BYTES: usize = 32;

/// Where the active key came from. Returned so startup can say which, since
/// "sessions will not survive a restart" is worth stating out loud.
#[derive(Debug, PartialEq, Eq)]
pub enum KeySource {
    /// Read from `JWT_SECRET`; sessions persist across restarts.
    Environment,
    /// Freshly generated; every restart invalidates outstanding sessions.
    Generated,
}

/// Choose the key, install it, and report where it came from.
pub fn init() -> KeySource {
    let (secret, source) = select_key();
    shared::jwt::init_secret(secret);
    source
}

/// Selection split from installation so it can be tested directly. Installing goes
/// through a `OnceLock`, where only the first call in a process takes effect.
fn select_key() -> (Vec<u8>, KeySource) {
    match env::var(JWT_SECRET_VAR) {
        // An empty value is treated as unset rather than honoured. Signing with an
        // empty key is a real, silent weakness, and `FOO=` in a compose file is far
        // more often an unset variable than a deliberate choice.
        Ok(value) if !value.is_empty() => (value.into_bytes(), KeySource::Environment),
        _ => (generate_key(), KeySource::Generated),
    }
}

fn generate_key() -> Vec<u8> {
    use rand::Rng;
    let mut key = vec![0u8; GENERATED_KEY_BYTES];
    // `rand::rng()` is the OS-seeded thread RNG and implements `CryptoRng`. Not
    // `SmallRng` or anything seeded from the clock: this key is the only thing
    // standing between an attacker and a forged session.
    rand::rng().fill_bytes(&mut key);
    key
}

/// Install a fixed key for backend tests.
///
/// `shared` has its own `#[cfg(test)]` fallback, but that only applies when `shared`
/// itself is the crate under test. Compiled as a dependency of this crate's test
/// binary it is an ordinary release build, so anything here that signs or validates
/// a token has to install a key first. Idempotent -- the underlying `OnceLock`
/// keeps the first value, so tests can call this freely and in any order.
#[cfg(test)]
pub fn init_for_tests() {
    shared::jwt::init_secret(b"backend-test-jwt-key".to_vec());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_the_right_length_and_not_constant() {
        let a = generate_key();
        let b = generate_key();
        assert_eq!(a.len(), GENERATED_KEY_BYTES);
        assert_ne!(a, b, "two generated keys must not match");
        assert!(a.iter().any(|&byte| byte != 0), "key must not be all zeros");
    }

    /// A set variable must be used verbatim, so an operator who wants sessions to
    /// survive restarts actually gets the key they supplied.
    #[test]
    #[serial_test::serial]
    fn env_var_is_used_verbatim() {
        let restore = env::var(JWT_SECRET_VAR).ok();
        // SAFETY: #[serial] -- no other thread reads the environment concurrently.
        unsafe { env::set_var(JWT_SECRET_VAR, "supplied-key") };

        let (key, source) = select_key();
        assert_eq!(source, KeySource::Environment);
        assert_eq!(key, b"supplied-key".to_vec());

        restore_env(restore);
    }

    /// Unset must generate rather than fall back to any constant. This is the
    /// regression that matters: the bug being fixed was a compiled-in key.
    #[test]
    #[serial_test::serial]
    fn unset_var_generates_a_fresh_key() {
        let restore = env::var(JWT_SECRET_VAR).ok();
        // SAFETY: as above.
        unsafe { env::remove_var(JWT_SECRET_VAR) };

        let (first, source) = select_key();
        let (second, _) = select_key();
        assert_eq!(source, KeySource::Generated);
        assert_eq!(first.len(), GENERATED_KEY_BYTES);
        assert_ne!(
            first, second,
            "generated keys must differ; a constant here is the original bug"
        );

        restore_env(restore);
    }

    /// `JWT_SECRET=` in a compose file is an unset variable far more often than a
    /// deliberate choice, and signing with an empty key fails silently.
    #[test]
    #[serial_test::serial]
    fn empty_var_is_treated_as_unset() {
        let restore = env::var(JWT_SECRET_VAR).ok();
        // SAFETY: as above.
        unsafe { env::set_var(JWT_SECRET_VAR, "") };

        let (key, source) = select_key();
        assert_eq!(source, KeySource::Generated);
        assert_eq!(key.len(), GENERATED_KEY_BYTES);

        restore_env(restore);
    }

    fn restore_env(restore: Option<String>) {
        // SAFETY: callers are all #[serial].
        unsafe {
            match restore {
                Some(v) => env::set_var(JWT_SECRET_VAR, v),
                None => env::remove_var(JWT_SECRET_VAR),
            }
        }
    }
}
