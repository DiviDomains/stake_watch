use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tracing::debug;

type HmacSha256 = Hmac<Sha256>;

/// Represents an authenticated Telegram user extracted from initData.
#[derive(Debug, Clone)]
pub struct TelegramUser {
    pub id: i64,
    pub first_name: String,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUserJson {
    id: i64,
    first_name: String,
    username: Option<String>,
}

/// Validate Telegram WebApp initData using HMAC-SHA256 and extract the user.
///
/// The validation follows the official Telegram algorithm:
/// 1. Parse the URL-encoded initData into key=value pairs
/// 2. Extract the `hash` value
/// 3. Sort the remaining pairs alphabetically and join with `\n`
/// 4. Compute `secret_key = HMAC-SHA256(key="WebAppData", data=bot_token)`
/// 5. Compute `computed_hash = HMAC-SHA256(key=secret_key, data=data_check_string)`
/// 6. Compare hex(computed_hash) with the extracted hash
/// 7. Parse the `user` JSON field to extract id, first_name, username
///
/// Returns `None` if validation fails at any step.
pub fn validate_init_data(init_data: &str, bot_token: &str) -> Option<TelegramUser> {
    // 1. Parse URL-encoded pairs
    let pairs: Vec<(String, String)> = form_urlencoded::parse(init_data.as_bytes())
        .into_owned()
        .collect();

    if pairs.is_empty() {
        debug!("initData has no pairs");
        return None;
    }

    // 2. Extract hash
    let hash_value = pairs.iter().find(|(k, _)| k == "hash").map(|(_, v)| v)?;

    // 3. Build data_check_string: sort remaining pairs alphabetically by key, join with \n
    let mut check_pairs: Vec<String> = pairs
        .iter()
        .filter(|(k, _)| k != "hash")
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    check_pairs.sort();
    let data_check_string = check_pairs.join("\n");

    // 4. secret_key = HMAC-SHA256("WebAppData", bot_token)
    let mut secret_mac =
        HmacSha256::new_from_slice(b"WebAppData").expect("HMAC can take any size key");
    secret_mac.update(bot_token.as_bytes());
    let secret_key = secret_mac.finalize().into_bytes();

    // 5. computed_hash = HMAC-SHA256(secret_key, data_check_string)
    let mut data_mac =
        HmacSha256::new_from_slice(&secret_key).expect("HMAC can take any size key");
    data_mac.update(data_check_string.as_bytes());
    let computed = data_mac.finalize().into_bytes();
    let computed_hex = hex::encode(computed);

    // 6. Compare
    if computed_hex != *hash_value {
        debug!(
            expected = %hash_value,
            computed = %computed_hex,
            "initData HMAC mismatch"
        );
        return None;
    }

    // 7. Parse user JSON
    let user_json = pairs.iter().find(|(k, _)| k == "user").map(|(_, v)| v)?;
    let user: TelegramUserJson = serde_json::from_str(user_json).ok()?;

    Some(TelegramUser {
        id: user.id,
        first_name: user.first_name,
        username: user.username,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_init_data_rejects_bad_hash() {
        let init_data = "user=%7B%22id%22%3A123%2C%22first_name%22%3A%22Test%22%7D&hash=badhash";
        assert!(validate_init_data(init_data, "fake_token").is_none());
    }

    #[test]
    fn test_validate_init_data_rejects_empty() {
        assert!(validate_init_data("", "fake_token").is_none());
    }

    #[test]
    fn test_validate_init_data_rejects_no_hash() {
        let init_data = "user=%7B%22id%22%3A123%2C%22first_name%22%3A%22Test%22%7D";
        assert!(validate_init_data(init_data, "fake_token").is_none());
    }
}
