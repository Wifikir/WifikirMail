use sha2::{Digest, Sha256};

pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn validate_email(email: &str) -> bool {
    let email_regex = regex::Regex::new(
        r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$"
    ).unwrap();
    email_regex.is_match(email)
}

pub fn validate_template_key(key: &str) -> bool {
    matches!(
        key,
        "EmailVerification" | "PasswordReset" | "Notification"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_api_key() {
        let key = "test-api-key";
        let hash1 = hash_api_key(key);
        let hash2 = hash_api_key(key);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_validate_email() {
        assert!(validate_email("user@example.com"));
        assert!(validate_email("test.user+tag@domain.co.uk"));
        assert!(!validate_email("invalid"));
        assert!(!validate_email("@example.com"));
        assert!(!validate_email("user@"));
    }

    #[test]
    fn test_validate_template_key() {
        assert!(validate_template_key("EmailVerification"));
        assert!(validate_template_key("PasswordReset"));
        assert!(validate_template_key("Notification"));
        assert!(!validate_template_key("Unknown"));
    }
}
