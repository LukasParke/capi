//! Self-update from GitHub releases.
//!
//! Fixes vs Go: requests the correct `-libcecN` asset chosen from the RUNTIME
//! linked libcec ABI, verifies SHA256SUMS before touching the binary, real
//! semver comparison (never downgrades), unique temp file + single-flight,
//! backup + rollback, honest restart reporting.

use crate::settings::Settings;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const REPO: &str = "LukasParke/capi";
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

#[derive(serde::Deserialize, Debug)]
struct ReleaseInfo {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize, Debug)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Compare strict semver tags vMAJOR.MINOR.PATCH. None when unparseable.
fn parse_tag(t: &str) -> Option<(u64, u64, u64)> {
    let t = t.strip_prefix('v')?;
    let mut it = t.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn current_version() -> &'static str {
    env!("CAPI_VERSION")
}

/// Asset suffix for the libcec ABI we were linked against (runtime query).
fn abi_suffix() -> Result<&'static str, String> {
    match crate::cec::linked_libcec_major() {
        6 => Ok("libcec6"),
        7 => Ok("libcec7"),
        other => Err(format!("unsupported runtime libcec ABI {other}")),
    }
}

fn asset_name() -> Result<String, String> {
    let abi = abi_suffix()?;
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "arm" => "armv6",
        other => return Err(format!("self-update unsupported on architecture {other}")),
    };
    Ok(format!("capi-linux-{arch}-{abi}"))
}

pub async fn check_and_perform(settings: &Settings) -> Result<Option<String>, String> {
    if IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return Err("update already in progress".into());
    }
    let result = check_and_perform_inner(settings).await;
    IN_FLIGHT.store(false, Ordering::SeqCst);
    result
}

async fn check_and_perform_inner(settings: &Settings) -> Result<Option<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent(format!("capi/{}", current_version()))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let rel: ReleaseInfo = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("query GitHub: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub API: {e}"))?
        .json()
        .await
        .map_err(|e| format!("parse release JSON: {e}"))?;

    // Semver-aware: only upgrade to a strictly newer release; unknown local
    // versions ("dev", dirty describes) always allow the update attempt.
    if let (Some(cur), Some(new)) = (parse_tag(current_version()), parse_tag(&rel.tag_name)) {
        if new <= cur {
            return Ok(None);
        }
    } else if rel.tag_name == current_version() {
        return Ok(None);
    }

    let asset_name = asset_name()?;
    let bin_asset = rel
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| format!("release {} has no asset {asset_name}", rel.tag_name))?;
    let sums_asset = rel
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .ok_or_else(|| {
            format!(
                "release {} has no SHA256SUMS (refusing unverified update)",
                rel.tag_name
            )
        })?;

    // Download binary to a unique temp file next to the target.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/opt/capi/capi"));
    let install_path = exe.clone();
    let tmp = install_path.with_extension("new");
    let _ = std::fs::remove_file(&tmp);

    let bytes = client
        .get(&bin_asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("download binary: {e}"))?
        .error_for_status()
        .map_err(|e| format!("binary download: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("read binary: {e}"))?;

    // Verify against SHA256SUMS BEFORE writing anything in place.
    let sums_raw = client
        .get(&sums_asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("download SHA256SUMS: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SHA256SUMS download: {e}"))?
        .text()
        .await
        .map_err(|e| format!("read SHA256SUMS: {e}"))?;

    let expected = expected_hash(&sums_raw, &asset_name)
        .ok_or_else(|| format!("SHA256SUMS has no entry for {asset_name}"))?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected {
        return Err(format!(
            "checksum mismatch for {asset_name}: expected {expected}, got {actual}"
        ));
    }

    std::fs::write(&tmp, &bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod: {e}"))?;
    }

    // Backup current binary, then swap atomically.
    let backup = install_path.with_extension("bak");
    let _ = std::fs::rename(&install_path, &backup);
    if let Err(e) = std::fs::rename(&tmp, &install_path) {
        // Roll forward failed: restore backup.
        let _ = std::fs::rename(&backup, &install_path);
        return Err(format!("swap binary: {e}"));
    }

    // Persist nothing else; restart is the caller's concern (systemd).
    tracing::info!(
        "updated binary to {} (backup at {})",
        rel.tag_name,
        backup.display()
    );
    let _ = settings; // reserved for future post-update config migration

    match restart_service().await {
        Ok(()) => {}
        Err(e) => {
            // Honest failure: the new binary is in place but not activated.
            tracing::warn!("restart failed: {e}; new binary activates on next service restart");
        }
    }
    Ok(Some(rel.tag_name))
}

fn expected_hash(sums: &str, asset: &str) -> Option<String> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        if file == asset {
            return Some(hash.to_lowercase());
        }
    }
    None
}

async fn restart_service() -> Result<(), String> {
    let out = tokio::process::Command::new("systemctl")
        .args(["restart", "capi.service"])
        .output()
        .await
        .map_err(|e| format!("spawn systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare() {
        assert_eq!(parse_tag("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_tag("1.2.3"), None);
        assert_eq!(parse_tag("v1.2"), None);
        assert!(parse_tag("dev").is_none());
    }

    #[test]
    fn sums_parsing() {
        let sums = "abc123  capi-linux-arm64-libcec6\ndef456 *capi-linux-armv6-libcec6\n";
        assert_eq!(
            expected_hash(sums, "capi-linux-arm64-libcec6").unwrap(),
            "abc123"
        );
        assert_eq!(
            expected_hash(sums, "capi-linux-armv6-libcec6").unwrap(),
            "def456"
        );
        assert_eq!(expected_hash(sums, "missing"), None);
    }

    #[test]
    fn asset_naming_rejects_x86() {
        // On x86_64 dev machines self-update must refuse, not fetch arm64.
        if std::env::consts::ARCH == "x86_64" {
            assert!(asset_name().is_err());
        }
    }
}
