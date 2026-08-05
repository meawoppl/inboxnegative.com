use std::collections::HashMap;

fn process_key_value_str(key_value_str: &str) -> Result<(&str, &str), ()> {
    match key_value_str.split_once('=') {
        Some((key, value)) => Ok((key.trim(), value.trim())),
        None => Err(()),
    }
}

/// Returns all cookies as key-value pairs, with undecoded keys and values.
pub fn all_iter_raw(cookie_string: &str) -> impl Iterator<Item = (&str, &str)> {
    cookie_string.split(';').filter_map(|key_value_str| {
        match process_key_value_str(key_value_str) {
            Ok((key, value)) => Some((key, value)),
            Err(_) => None,
        }
    })
}

/// Returns all cookies, with undecoded keys and values.
pub fn all_raw(cookie_string: &str) -> HashMap<String, String> {
    all_iter_raw(cookie_string)
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}
