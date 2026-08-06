use std::{path::Path, time::Duration};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// npm packages pinned via `npx` in the executors' `base_command`, and where the pin lives.
/// zona de envelhecimento — revisar no merge mensal
const PINNED_CLIS: &[(&str, &str, &str)] = &[
    (
        "@anthropic-ai/claude-code",
        "2.1.223",
        "crates/executors/src/executors/claude.rs — base_command(), the claude-code npx pin",
    ),
    (
        "@musistudio/claude-code-router",
        "1.0.66",
        "crates/executors/src/executors/claude.rs — base_command(), the claude-code-router npx pin",
    ),
    (
        "@openai/codex",
        "0.124.0",
        "crates/executors/src/executors/codex.rs — base_command() npx pin",
    ),
    (
        "@google/gemini-cli",
        "0.29.3",
        "crates/executors/src/executors/gemini.rs — build_command_builder() npx pin",
    ),
    (
        "opencode-ai",
        "1.4.7",
        "crates/executors/src/executors/opencode.rs — build_command_builder() npx pin",
    ),
    (
        "@qwen-code/qwen-code",
        "0.9.1",
        "crates/executors/src/executors/qwen.rs — build_command_builder() npx pin",
    ),
    (
        "@github/copilot",
        "0.0.403",
        "crates/executors/src/executors/copilot.rs — build_command_builder() npx pin",
    ),
];

const CACHE_TTL_SECS: i64 = 24 * 60 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CliFreshnessEntry {
    pub package: String,
    pub pinned_version: String,
    pub latest_version: String,
    pub is_stale: bool,
    pub pin_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CliFreshnessReport {
    #[ts(type = "number")]
    pub checked_at: i64,
    pub entries: Vec<CliFreshnessEntry>,
}

impl CliFreshnessReport {
    fn empty() -> Self {
        Self {
            checked_at: 0,
            entries: Vec::new(),
        }
    }
}

/// Reads the cache file for the handler that serves `GET /cli-freshness`.
/// Never errors: a missing or corrupt cache file just means "nothing checked yet".
pub async fn load_report(cache_path: &Path) -> CliFreshnessReport {
    read_cache_file(cache_path)
        .await
        .unwrap_or_else(CliFreshnessReport::empty)
}

/// Refreshes the cache from the npm registry if it is missing or older than 24h.
/// Fire-and-forget: any error (offline, 4xx, bad JSON) is logged at debug level and
/// otherwise ignored per-package — this must never fail boot.
pub async fn refresh_if_stale(cache_path: std::path::PathBuf) {
    let now = chrono::Utc::now().timestamp();
    if let Some(cached) = read_cache_file(&cache_path).await
        && is_cache_fresh(cached.checked_at, now)
    {
        return;
    }

    let client = match reqwest::Client::builder().timeout(HTTP_TIMEOUT).build() {
        Ok(client) => client,
        Err(e) => {
            tracing::debug!("cli_freshness: failed to build http client: {e}");
            return;
        }
    };

    let mut entries = Vec::with_capacity(PINNED_CLIS.len());
    for (package, pinned_version, pin_hint) in PINNED_CLIS {
        match fetch_latest_version(&client, package).await {
            Ok(latest_version) => entries.push(CliFreshnessEntry {
                package: package.to_string(),
                is_stale: is_stale(pinned_version, &latest_version),
                latest_version,
                pinned_version: pinned_version.to_string(),
                pin_hint: pin_hint.to_string(),
            }),
            Err(e) => {
                tracing::debug!("cli_freshness: failed to check {package}: {e}");
            }
        }
    }

    if entries.is_empty() {
        // Nothing succeeded (likely offline) — don't cache emptiness, retry next boot instead.
        tracing::debug!("cli_freshness: no packages could be checked, skipping cache write");
        return;
    }

    let report = CliFreshnessReport {
        checked_at: now,
        entries,
    };
    if let Err(e) = write_cache_file(&cache_path, &report).await {
        tracing::debug!("cli_freshness: failed to write cache: {e}");
    }
}

/// True when a cache checked at `checked_at` is still within the 24h TTL at `now`.
fn is_cache_fresh(checked_at: i64, now: i64) -> bool {
    now - checked_at < CACHE_TTL_SECS
}

async fn read_cache_file(path: &Path) -> Option<CliFreshnessReport> {
    let content = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&content).ok()
}

async fn write_cache_file(path: &Path, report: &CliFreshnessReport) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(report)?;
    tokio::fs::write(path, json).await?;
    Ok(())
}

async fn fetch_latest_version(client: &reqwest::Client, package: &str) -> anyhow::Result<String> {
    let url = format!("https://registry.npmjs.org/{package}/latest");
    let value: serde_json::Value = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("no `version` field in registry response for {package}"))
}

/// Numeric, component-by-component comparison of `x.y.z`-style version strings.
/// Returns true when `latest` is strictly newer than `pinned`.
fn is_stale(pinned: &str, latest: &str) -> bool {
    version_parts(latest) > version_parts(pinned)
}

fn version_parts(version: &str) -> Vec<u32> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_stale_when_latest_is_newer() {
        assert!(is_stale("0.124.0", "0.146.1"));
    }

    #[test]
    fn detects_fresh_when_versions_are_equal() {
        assert!(!is_stale("2.1.223", "2.1.223"));
    }

    #[test]
    fn detects_fresh_when_pinned_is_newer() {
        assert!(!is_stale("2.1.223", "2.1.100"));
    }

    #[test]
    fn compares_numerically_not_lexicographically() {
        // String comparison would say "1.10.0" < "1.2.0" ('1' < '2'); numeric must not.
        assert!(is_stale("1.2.0", "1.10.0"));
    }

    #[test]
    fn treats_missing_trailing_component_as_zero() {
        assert!(!is_stale("1.2.0", "1.2"));
    }

    #[test]
    fn cache_is_fresh_just_inside_the_24h_window() {
        assert!(is_cache_fresh(1_000, 1_000 + CACHE_TTL_SECS - 1));
    }

    #[test]
    fn cache_is_stale_once_24h_have_passed() {
        assert!(!is_cache_fresh(1_000, 1_000 + CACHE_TTL_SECS));
    }

    #[tokio::test]
    async fn load_report_is_empty_when_cache_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");

        let report = load_report(&path).await;

        assert_eq!(report.checked_at, 0);
        assert!(report.entries.is_empty());
    }

    #[tokio::test]
    async fn load_report_round_trips_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cli_freshness.json");
        let report = CliFreshnessReport {
            checked_at: 12_345,
            entries: vec![CliFreshnessEntry {
                package: "@openai/codex".to_string(),
                pinned_version: "0.124.0".to_string(),
                latest_version: "0.146.1".to_string(),
                is_stale: true,
                pin_hint: "crates/executors/src/executors/codex.rs — base_command() npx pin"
                    .to_string(),
            }],
        };

        write_cache_file(&path, &report).await.unwrap();
        let loaded = load_report(&path).await;

        assert_eq!(loaded.checked_at, 12_345);
        assert_eq!(loaded.entries.len(), 1);
        assert!(loaded.entries[0].is_stale);
    }

    /// Verification duty: hits the real npm registry. Run with `--ignored` to capture
    /// live evidence (see task-4-report.md for actual output).
    #[tokio::test]
    #[ignore]
    async fn fetches_real_latest_version_from_registry() {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap();
        for (package, pinned_version, _) in PINNED_CLIS {
            let latest = fetch_latest_version(&client, package).await.unwrap();
            println!(
                "{package}: pinned={pinned_version} latest={latest} stale={}",
                is_stale(pinned_version, &latest)
            );
        }
    }
}
