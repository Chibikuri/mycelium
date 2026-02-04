use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::{AppError, Result};

type HmacSha256 = Hmac<Sha256>;

/// Verify the GitHub webhook HMAC-SHA256 signature.
///
/// GitHub sends the signature in the `X-Hub-Signature-256` header as `sha256=<hex>`.
pub fn verify_signature(secret: &str, payload: &[u8], signature_header: &str) -> Result<()> {
    let signature_hex = signature_header
        .strip_prefix("sha256=")
        .ok_or_else(|| AppError::WebhookVerification("Missing sha256= prefix".to_string()))?;

    let signature_bytes = hex::decode(signature_hex)
        .map_err(|e| AppError::WebhookVerification(format!("Invalid hex in signature: {e}")))?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| AppError::WebhookVerification(format!("Invalid HMAC key: {e}")))?;

    mac.update(payload);

    mac.verify_slice(&signature_bytes)
        .map_err(|_| AppError::WebhookVerification("Signature mismatch".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_signature() {
        let secret = "test-secret";
        let payload = b"hello world";

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let result = mac.finalize();
        let expected_hex = hex::encode(result.into_bytes());

        let header = format!("sha256={expected_hex}");
        assert!(verify_signature(secret, payload, &header).is_ok());
    }

    #[test]
    fn test_invalid_signature() {
        let secret = "test-secret";
        let payload = b"hello world";
        let header = "sha256=0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_signature(secret, payload, header).is_err());
    }

    #[test]
    fn test_missing_prefix() {
        let secret = "test-secret";
        let payload = b"hello world";
        let header = "abcdef1234567890";
        assert!(verify_signature(secret, payload, header).is_err());
    }
}
