//! HTTP upload module — ScanPayloadV1 POST with retry, response handling, and file fallback.
//!
//! This is the ONLY module that uses tokio. All other code (plugins, core) is synchronous.
//! See PITFALLS.md Pitfall 4: Tokio/Rayon deadlock prevention.

use anyhow::anyhow;
use std::time::{Duration, UNIX_EPOCH};
use tracing::{info, warn};

use crate::core::payload::ScanPayloadV1;

/// Configuration for uploading to Arcanon Hub.
#[derive(Debug, Clone)]
pub struct UploadConfig {
    /// Hub URL (e.g., "https://hub.arcanon.dev")
    pub hub_url: String,
    /// API key for Bearer authentication
    pub api_key: String,
}

/// Upload a ScanPayloadV1 to Arcanon Hub with exponential backoff retry.
///
/// # Behavior
///
/// - Serializes payload to JSON
/// - POSTs to `{hub_url}/api/v1/scans/upload` with Bearer auth
/// - Retries on HTTP 429 and 5xx with delays: 1s, 2s, 4s (max 3 retries)
/// - Returns Ok(()) on 202 (accepted) or 409 (duplicate)
/// - Returns Err on 400/401/413 (validation/auth/size errors) immediately
/// - Saves payload to `arcanon-scan-{timestamp}.json` on network error after final retry
///
/// # Errors
///
/// - Serialization errors
/// - Authentication failures (401)
/// - Validation errors (400)
/// - Payload too large (413)
/// - Network unreachable after 3 retries (payload saved to file)
pub async fn upload(payload: &ScanPayloadV1, config: &UploadConfig) -> anyhow::Result<()> {
    let body = serde_json::to_string(payload)
        .map_err(|e| anyhow!("Failed to serialize payload: {}", e))?;

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/scans/upload", config.hub_url);

    // Retry loop: attempt 0 (immediate), attempt 1 (after 1s), attempt 2 (after 2s), attempt 3 (after 4s)
    for attempt in 0u32..=3 {
        if attempt > 0 {
            // Exponential backoff: 1 << (attempt - 1) gives 1, 2, 4 for attempts 1, 2, 3
            tokio::time::sleep(Duration::from_secs(1_u64 << (attempt - 1))).await;
        }

        match client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await
        {
            Err(e) => {
                warn!("Upload attempt {} failed: {}", attempt + 1, e);
                if attempt == 3 {
                    save_payload_to_file(payload)?;
                    return Err(anyhow!("Upload failed after 3 retries: {}", e));
                }
                // Continue to next attempt
            }
            Ok(response) => match response.status().as_u16() {
                202 => {
                    info!("Upload accepted (202)");
                    return Ok(());
                }
                409 => {
                    info!("Duplicate scan (409) — commit already processed");
                    return Ok(());
                }
                // Retryable errors: 429 (too many requests), 5xx (server errors)
                429 | 500..=599 => {
                    warn!(
                        "Retryable response {} on attempt {}",
                        response.status(),
                        attempt + 1
                    );
                    // Continue to next attempt
                }
                // Non-retryable client errors
                400 => {
                    return Err(anyhow!(
                        "Payload validation failed (400): {}",
                        response.text().await.unwrap_or_default()
                    ));
                }
                401 => {
                    return Err(anyhow!(
                        "Authentication failed (401) — check ARCANON_API_KEY"
                    ));
                }
                413 => {
                    return Err(anyhow!("Payload too large (413)"));
                }
                code => {
                    return Err(anyhow!("Unexpected response: {}", code));
                }
            },
        }
    }

    Err(anyhow!("Upload failed after 3 retries"))
}

/// Save payload to a timestamped JSON file when network is unreachable.
///
/// Called only after the final retry attempt fails with a network error.
/// Logs the file path for manual recovery.
fn save_payload_to_file(payload: &ScanPayloadV1) -> anyhow::Result<()> {
    let secs = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let filename = format!("arcanon-scan-{}.json", secs);
    let json = serde_json::to_string_pretty(payload)?;
    std::fs::write(&filename, &json)?;
    warn!("Network unreachable — payload saved to {}", filename);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_config_fields_accessible() {
        let cfg = UploadConfig {
            hub_url: "https://hub.arcanon.dev".to_string(),
            api_key: "test-key".to_string(),
        };
        assert_eq!(cfg.hub_url, "https://hub.arcanon.dev");
        assert_eq!(cfg.api_key, "test-key");
    }

    #[test]
    fn test_exponential_backoff_calculation() {
        // Verify the backoff formula: 1 << (attempt - 1) for attempts 1, 2, 3
        // Attempt 1: 1 << 0 = 1
        // Attempt 2: 1 << 1 = 2
        // Attempt 3: 1 << 2 = 4
        assert_eq!(1_u64 << (1 - 1), 1);
        assert_eq!(1_u64 << (2 - 1), 2);
        assert_eq!(1_u64 << (3 - 1), 4);
    }
}
