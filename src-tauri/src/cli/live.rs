//! On-demand pulls for `openquota pace --refresh`.
//!
//! Pulled readings never go into the application database, so the command cannot race the
//! running application's writes or replace a newer reading it saved. They are kept in
//! [`CACHE_FILE_NAME`] next to the database instead, a file only this command reads and writes,
//! so a repeated `pace --refresh` within a minute reuses them instead of pulling again.

use std::{
    collections::HashMap,
    io::Write,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{
    models::ProviderSnapshot,
    providers::{
        antigravity::AntigravityProvider, commandcode::CommandCodeProvider,
        copilot::CopilotProvider, cursor::CursorProvider, devin::DevinProvider, grok::GrokProvider,
        kimi::KimiProvider, minimax::MiniMaxProvider, opencode::OpenCodeProvider,
        openrouter::OpenRouterProvider, zai::ZaiProvider, UsageProvider,
    },
};

pub const CACHE_FILE_NAME: &str = "pace-live.json";

/// Providers that can be pulled without the application database. Claude and Codex keep
/// account records there, so only the application refreshes them.
pub const PULLABLE_PROVIDERS: [&str; 11] = [
    "antigravity",
    "copilot",
    "cursor",
    "devin",
    "grok",
    "kimi",
    "minimax",
    "commandcode",
    "opencode",
    "openrouter",
    "zai",
];

pub fn is_pullable(provider_id: &str) -> bool {
    PULLABLE_PROVIDERS.contains(&provider_id)
}

#[derive(Debug, Clone, PartialEq)]
pub enum PullResult {
    Fresh(ProviderSnapshot),
    Failed(String),
    TimedOut,
}

/// Reads one provider; the production reader builds the same provider runtime the app uses.
pub type Reader = Arc<dyn Fn(&str) -> Result<ProviderSnapshot, String> + Send + Sync>;

pub fn provider_reader(data_directory: PathBuf) -> Reader {
    Arc::new(move |provider_id| {
        let provider = build_provider(provider_id, &data_directory)?;
        provider.refresh().map_err(|error| error.to_string())
    })
}

fn build_provider(
    provider_id: &str,
    data_directory: &Path,
) -> Result<Box<dyn UsageProvider>, String> {
    fn boxed<P: UsageProvider + 'static, E: std::fmt::Display>(
        provider: Result<P, E>,
    ) -> Result<Box<dyn UsageProvider>, String> {
        provider
            .map(|provider| Box::new(provider) as Box<dyn UsageProvider>)
            .map_err(|error| error.to_string())
    }
    match provider_id {
        "antigravity" => boxed(AntigravityProvider::new(
            data_directory.join("antigravity").join("auth.json"),
        )),
        "copilot" => boxed(CopilotProvider::new()),
        "cursor" => boxed(CursorProvider::new()),
        "devin" => boxed(DevinProvider::new()),
        "grok" => boxed(GrokProvider::new()),
        "kimi" => boxed(KimiProvider::new()),
        "minimax" => boxed(MiniMaxProvider::new()),
        "commandcode" => boxed(CommandCodeProvider::new()),
        "opencode" => Ok(Box::new(OpenCodeProvider::new())),
        "openrouter" => boxed(OpenRouterProvider::new()),
        "zai" => boxed(ZaiProvider::new()),
        other => Err(format!("{other} cannot be refreshed from the command line")),
    }
}

/// How long the command lingers after printing for pulls that missed their time limit. Every
/// provider bounds its own requests to about this long, so a pull caught renewing a login still
/// gets to save the renewed token instead of losing it with the process.
pub const LATE_PULL_GRACE: Duration = Duration::from_secs(15);

/// Workers still running when their time limit passed. Their results are discarded.
#[derive(Debug)]
pub struct LatePulls {
    receiver: mpsc::Receiver<(String, PullResult)>,
    remaining: usize,
}

impl LatePulls {
    /// Waits up to `grace` for the late workers to finish.
    pub fn settle(self, grace: Duration) {
        let deadline = Instant::now() + grace;
        for _ in 0..self.remaining {
            let left = deadline.saturating_duration_since(Instant::now());
            if self.receiver.recv_timeout(left).is_err() {
                return;
            }
        }
    }
}

/// Pulls every provider in parallel and waits at most `timeout` for each. A provider that has
/// not answered by then is reported as timed out; its worker is returned in [`LatePulls`].
pub fn pull_all(
    provider_ids: &[String],
    timeout: Duration,
    reader: Reader,
) -> (Vec<(String, PullResult)>, LatePulls) {
    let (sender, receiver) = mpsc::channel();
    for provider_id in provider_ids {
        let reader = reader.clone();
        let spawned = thread::Builder::new()
            .name(format!("pace-refresh-{provider_id}"))
            .spawn({
                let provider_id = provider_id.clone();
                let sender = sender.clone();
                move || {
                    let read = panic::catch_unwind(AssertUnwindSafe(|| reader(&provider_id)));
                    let result = match read {
                        Ok(Ok(snapshot)) if snapshot.provider_id != provider_id => {
                            PullResult::Failed(
                                "the provider returned a reading for another provider".into(),
                            )
                        }
                        Ok(Ok(snapshot)) if snapshot.remembered => PullResult::Failed(
                            "the provider only returned its last remembered reading".into(),
                        ),
                        Ok(Ok(snapshot)) => PullResult::Fresh(snapshot),
                        Ok(Err(message)) => PullResult::Failed(message),
                        Err(_) => PullResult::Failed("the refresh stopped unexpectedly".into()),
                    };
                    let _ = sender.send((provider_id, result));
                }
            });
        if spawned.is_err() {
            let _ = sender.send((
                provider_id.clone(),
                PullResult::Failed("the refresh worker could not be started".into()),
            ));
        }
    }
    drop(sender);

    let deadline = Instant::now() + timeout;
    let mut results = HashMap::new();
    while results.len() < provider_ids.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok((provider_id, result)) => {
                results.insert(provider_id, result);
            }
            Err(_) => break,
        }
    }
    let late = LatePulls {
        remaining: provider_ids.len() - results.len(),
        receiver,
    };
    let results = provider_ids
        .iter()
        .map(|provider_id| {
            let result = results.remove(provider_id).unwrap_or(PullResult::TimedOut);
            (provider_id.clone(), result)
        })
        .collect();
    (results, late)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    snapshots: Vec<ProviderSnapshot>,
}

pub fn cache_path(data_directory: &Path) -> PathBuf {
    data_directory.join(CACHE_FILE_NAME)
}

/// Readings from earlier `pace --refresh` runs. A missing file is an empty cache.
pub fn load_cache(path: &Path) -> Result<Vec<ProviderSnapshot>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    serde_json::from_str::<CacheFile>(&text)
        .map(|cache| cache.snapshots)
        .map_err(|error| error.to_string())
}

/// Stores `fresh` over the cached readings of the same providers, replacing the file
/// atomically so a concurrent reader never sees a partial file.
pub fn save_cache(
    path: &Path,
    previous: Vec<ProviderSnapshot>,
    fresh: &[ProviderSnapshot],
) -> Result<(), String> {
    let mut snapshots = previous
        .into_iter()
        .filter(|cached| {
            is_pullable(&cached.provider_id)
                && !fresh
                    .iter()
                    .any(|snapshot| snapshot.provider_id == cached.provider_id)
        })
        .collect::<Vec<_>>();
    snapshots.extend(fresh.iter().cloned());
    snapshots.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    let payload =
        serde_json::to_vec(&CacheFile { snapshots }).map_err(|error| error.to_string())?;
    let directory = path
        .parent()
        .ok_or_else(|| "the cache path has no directory".to_owned())?;
    let mut temporary = NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
    temporary
        .write_all(&payload)
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map_err(|error| error.error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use chrono::{TimeZone, Utc};

    use super::{
        build_provider, cache_path, is_pullable, load_cache, pull_all, save_cache, PullResult,
        Reader, PULLABLE_PROVIDERS,
    };
    use crate::models::ProviderSnapshot;

    fn reading(provider_id: &str, plan: &str) -> ProviderSnapshot {
        ProviderSnapshot {
            provider_id: provider_id.into(),
            plan: Some(plan.into()),
            quotas: Vec::new(),
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
            remembered: false,
        }
    }

    #[test]
    fn every_pullable_provider_builds_without_the_application_database() {
        let directory = tempfile::tempdir().unwrap();
        for provider_id in PULLABLE_PROVIDERS {
            let provider = build_provider(provider_id, directory.path())
                .unwrap_or_else(|error| panic!("{provider_id}: {error}"));
            assert_eq!(provider.definition().id, provider_id);
        }
        assert!(build_provider("claude", directory.path()).is_err());
        assert!(build_provider("codex", directory.path()).is_err());
        assert!(!is_pullable("claude"));
        assert!(!is_pullable("claude@1234abcd"));
        assert!(!is_pullable("codex"));
    }

    #[test]
    fn slow_and_failing_providers_do_not_hold_back_the_others() {
        let reader: Reader = Arc::new(|provider_id| match provider_id {
            "cursor" => Ok(reading("cursor", "fresh")),
            "grok" => Err("Could not reach Grok.".into()),
            "kimi" => {
                std::thread::sleep(Duration::from_secs(4));
                Ok(reading("kimi", "late"))
            }
            "devin" => panic!("devin broke"),
            "zai" => Ok(reading("kimi", "wrong provider")),
            _ => Ok(ProviderSnapshot {
                remembered: true,
                ..reading(provider_id, "remembered")
            }),
        });
        let requested = ["cursor", "grok", "kimi", "zai", "antigravity", "devin"]
            .map(str::to_owned)
            .to_vec();
        let started = std::time::Instant::now();
        let (results, late) = pull_all(&requested, Duration::from_secs(1), reader);
        assert!(started.elapsed() < Duration::from_secs(3));

        assert_eq!(
            results,
            [
                (
                    "cursor".to_owned(),
                    PullResult::Fresh(reading("cursor", "fresh"))
                ),
                (
                    "grok".to_owned(),
                    PullResult::Failed("Could not reach Grok.".into())
                ),
                ("kimi".to_owned(), PullResult::TimedOut),
                (
                    "zai".to_owned(),
                    PullResult::Failed(
                        "the provider returned a reading for another provider".into()
                    )
                ),
                (
                    "antigravity".to_owned(),
                    PullResult::Failed(
                        "the provider only returned its last remembered reading".into()
                    )
                ),
                (
                    "devin".to_owned(),
                    PullResult::Failed("the refresh stopped unexpectedly".into())
                ),
            ]
        );

        // The late worker is waited for, so a login it renews is saved before the process
        // exits, but never beyond the grace period.
        assert_eq!(late.remaining, 1);
        let settling = std::time::Instant::now();
        late.settle(Duration::from_secs(10));
        let waited = settling.elapsed();
        assert!(waited > Duration::from_millis(1_500), "{waited:?}");
        assert!(waited < Duration::from_secs(8), "{waited:?}");

        let reader: Reader = Arc::new(|_| {
            std::thread::sleep(Duration::from_secs(5));
            Ok(reading("cursor", "late"))
        });
        let (_, late) = pull_all(&["cursor".to_owned()], Duration::from_millis(100), reader);
        let settling = std::time::Instant::now();
        late.settle(Duration::from_millis(300));
        assert!(settling.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn the_cache_file_keeps_other_providers_and_replaces_pulled_ones() {
        let directory = tempfile::tempdir().unwrap();
        let path = cache_path(directory.path());
        assert!(load_cache(&path).unwrap().is_empty());

        save_cache(
            &path,
            Vec::new(),
            &[reading("cursor", "first"), reading("grok", "first")],
        )
        .unwrap();
        save_cache(
            &path,
            load_cache(&path).unwrap(),
            &[reading("cursor", "second")],
        )
        .unwrap();

        let cached = load_cache(&path).unwrap();
        assert_eq!(
            cached,
            [reading("cursor", "second"), reading("grok", "first")]
        );

        std::fs::write(&path, "{not json").unwrap();
        assert!(load_cache(&path).is_err());
    }
}
