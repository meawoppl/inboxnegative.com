use crate::salt;
use sha2::{Digest, Sha256};
use std::env;

const HASH_BYTE_LENGTH: usize = 8;
const HASH_HEX_LENGTH: usize = HASH_BYTE_LENGTH * 2;

fn hash_email(email: &str, host: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(email);
    hasher.update(salt);
    let hash = hasher.finalize();
    let truncated_hash = hash[..HASH_BYTE_LENGTH]
        .iter()
        .fold(String::new(), |mut acc, byte| {
            acc.push_str(&format!("{:02x}", byte));
            acc
        });

    truncated_hash + host
}

const EMAIL_HOST: &str = "@inboxnegative.com";

pub fn hash_email_simple(email: &str) -> String {
    // The host is public and safely defaulted; the salt is not -- see `salt`.
    let email_host = env::var("EMAIL_HOST").unwrap_or_else(|_| EMAIL_HOST.to_string());
    hash_email(email, &email_host, salt::email_salt())
}

pub fn is_valid_address(email: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = email.split("@").collect();
    if parts.len() != 2 {
        return Err(format!("Wholly invalid email address: {}", email).into());
    }

    let local = parts[0];
    let domain = parts[1];

    if domain != "inboxnegative.com" {
        return Err(format!("Wrong domain: {}", domain).into());
    }

    if local.len() != HASH_HEX_LENGTH {
        return Err(format!("Wrong hash length: {}", domain).into());
    }

    match i64::from_str_radix(&local[0..8], 16) {
        Ok(_) => {}
        Err(_) => {
            return Err(format!("Invalid hex: {}", local).into());
        }
    }

    match i64::from_str_radix(&local[8..16], 16) {
        Ok(_) => Ok(()),
        Err(_) => Err(format!("Invalid hex: {}", local).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_EMAIL: &str = "test@example.com";
    // Salts are injected explicitly here rather than read from the environment,
    // so no real salt value lives in the source tree. `TEST_SALT` matches the
    // test-only fallback in `salt`, so `hash_email_simple` agrees with these.
    const TEST_SALT: &str = "TestSalt";
    const EXPT_HASH: &str = "d10bfb93e21897f1";

    #[test]
    fn test_hash_email() {
        let actual_hash1 = hash_email(TEST_EMAIL, "", TEST_SALT);
        assert_eq!(actual_hash1, EXPT_HASH);
    }

    #[test]
    fn test_hash_email_uses_suffix() {
        let actual_hash1 = hash_email(TEST_EMAIL, EMAIL_HOST, TEST_SALT);
        assert_eq!(actual_hash1, EXPT_HASH.to_string() + EMAIL_HOST);
    }

    #[test]
    fn test_hash_email_uses_salt() {
        let actual_hash1 = hash_email(TEST_EMAIL, EMAIL_HOST, TEST_SALT);
        assert_eq!(actual_hash1, EXPT_HASH.to_string() + EMAIL_HOST);

        // A different salt must produce a different address.
        let actual_hash2 = hash_email(TEST_EMAIL, EMAIL_HOST, "MgCl");
        assert_eq!(actual_hash2, "8dd2913fd0f1fb7d".to_string() + EMAIL_HOST);

        assert_ne!(actual_hash1, actual_hash2);
    }

    #[test]
    fn test_hash_email_simple() {
        // Test with a valid email and default salt
        let hashed = hash_email_simple(TEST_EMAIL);
        assert_eq!(hashed, EXPT_HASH.to_string() + EMAIL_HOST);
    }

    #[test]
    fn test_is_valid_address() {
        // Test with a valid email
        is_valid_address("deadbeefdeadbeef@inboxnegative.com").unwrap();

        is_valid_address(&hash_email_simple(TEST_EMAIL)).unwrap();

        // Test with an invalid email
        assert!(is_valid_address("").is_err());
        assert!(is_valid_address("sdfsdsdfs").is_err());

        // Test strings we have received when probed
        assert!(is_valid_address("5O2WIZGTITLHBDIB@INBOXNEGATIVE.COM").is_err());
        assert!(is_valid_address("SMACRPAXVDYGSQPVSVNXCPZCTMWRSBGX@INBOXNEGATIVE.COM").is_err());
    }
}
