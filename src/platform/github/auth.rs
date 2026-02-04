use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use std::path::Path;

use crate::error::{AppError, Result};

#[derive(Debug, Serialize)]
struct JwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// Generate a JWT for GitHub App authentication.
pub fn generate_app_jwt(app_id: u64, private_key_path: &Path) -> Result<String> {
    let key_pem = std::fs::read(private_key_path).map_err(|e| {
        AppError::Config(format!(
            "Failed to read private key at {}: {e}",
            private_key_path.display()
        ))
    })?;

    let encoding_key = EncodingKey::from_rsa_pem(&key_pem)
        .map_err(|e| AppError::Config(format!("Invalid RSA private key: {e}")))?;

    let now = chrono::Utc::now().timestamp();
    let claims = JwtClaims {
        iat: now - 60,      // 60 seconds in the past to account for clock drift
        exp: now + 10 * 60, // 10 minute maximum
        iss: app_id.to_string(),
    };

    let header = Header::new(Algorithm::RS256);
    encode(&header, &claims, &encoding_key)
        .map_err(|e| AppError::Config(format!("Failed to generate JWT: {e}")))
}
