//! Headless subcommands handled before the Tauri application starts.
//!
//! A subcommand must not launch the GUI, register the single-instance plugin, or write to the
//! application database. `pace` stays offline unless asked to pull current readings with
//! `--refresh`; see [`live`] for where those readings are kept instead of the database.

mod freshness;
mod live;
mod report;
use report::*;

use std::{
    collections::HashSet,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;

use crate::{
    i18n::{resolve_ui_locale, system_language_tag},
    instance_lock,
    models::{ProviderSnapshot, UiLanguagePreference},
};

use freshness::RefreshOutcome;

/// The report finished successfully and printed at least one provider.
pub const EXIT_SUCCESS: i32 = 0;
/// The database could not be located, opened, or queried.
pub const EXIT_READ_FAILURE: i32 = 1;
/// The database was read but held no usable snapshots.
pub const EXIT_NO_DATA: i32 = 2;
/// The command line could not be understood.
pub const EXIT_USAGE: i32 = 64;

const PACE_COMMAND: &str = "pace";
const DATABASE_FILE_NAME: &str = "openquota.db";
const SNAPSHOT_TABLE: &str = "provider_snapshots";
const SETTINGS_TABLE: &str = "app_settings";
const DATABASE_OVERRIDE_ENV: &str = "OPENQUOTA_PACE_DB";
/// Identifier from `src-tauri/tauri.conf.json`; the application database lives
/// at `<platform data dir>/<identifier>/openquota.db`.
const APP_IDENTIFIER: &str = "io.github.deviffyy.openquota";
/// Session style windows stay at or below five hours; one hour of tolerance
/// keeps slightly longer session windows in the short column.
const SHORT_WINDOW_MAX_PERIOD_SECONDS: u64 = 6 * 60 * 60;
const DEFAULT_REFRESH_TIMEOUT_SECONDS: u64 = 20;
const MAX_REFRESH_TIMEOUT_SECONDS: u64 = 600;

const USAGE: &str = "\
Usage: openquota pace [--json] [--db <path>]
       openquota pace --refresh [--only <ids>] [--timeout <seconds>] [--json] [--db <path>]

Prints the latest quotas of the providers turned on in the OpenQuota panel.

Without --refresh it only reads the local OpenQuota database, opened read-only:
no GUI, no network access, and no writes. Account matching reuses local Claude
noncredential account facts when readable; database identity alone remains unknown.

--refresh first pulls current readings from the providers over the network,
all in parallel with a time limit each. A pull that fails or times out keeps
the earlier reading, marked stale, and never fails the command; a pull still
running at its limit may hold the exit back by up to 15 seconds after the
report is printed, so a login it is renewing gets saved. A provider read
within the last 60 seconds, by the app or an earlier pace run, is reused instead
of pulled again. Pulled readings are never written to the application database,
so they cannot collide with the running app's writes; they go to pace-live.json
next to it, which only this command reads and writes. As in the app, a pull may
renew and save that provider's own expired login token. Claude keeps account
records in the application database, so only the app refreshes it. Codex reads
local credentials without writing account records.

The DATA column reads `live` for a reading pulled just now, `cached` for a
recent read, and `stale` once the numbers are older than the panel's staleness
window or a requested pull failed, so remembered values are never mistaken for
current ones. When the OpenQuota app is not running, nothing keeps the cached
readings up to date: stderr says how old the oldest one is, and the text table
marks those rows with `*`.

Options:
  --json               Print a stable JSON array instead of a text table
  --refresh            Pull current readings before printing
  --only <ids>         With --refresh, pull only these comma-separated providers:
                       antigravity, codex, copilot, cursor, devin, grok, kimi, minimax, commandcode,
                       opencode, openrouter, zai
  --timeout <seconds>  With --refresh, time limit for each provider (default 20)
  --db <path>          Read this database file instead of the default (for testing)
  -h, --help           Show this help

Exit codes:
  0   report printed
  1   database could not be read
  2   no readings for the providers turned on in the panel
  64  invalid command line

Environment:
  OPENQUOTA_PACE_DB  Overrides the database path (for testing)
";

/// Runs a headless subcommand when one was requested and returns the exit code.
///
/// Returns `None` when the process should start the normal desktop application.
pub fn dispatch(arguments: &[String]) -> Option<i32> {
    match arguments.first().map(String::as_str) {
        Some(PACE_COMMAND) => Some(run_pace(&arguments[1..])),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
struct PaceOptions {
    json: bool,
    help: bool,
    database: Option<PathBuf>,
    refresh: Option<RefreshOptions>,
}

#[derive(Debug, PartialEq)]
struct RefreshOptions {
    /// `None` pulls every enabled provider that can be pulled.
    only: Option<Vec<String>>,
    timeout: Duration,
}

fn parse_options(arguments: &[String]) -> Result<PaceOptions, String> {
    let mut json = false;
    let mut help = false;
    let mut database = None;
    let mut refresh = false;
    let mut only = None;
    let mut timeout = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--json" => json = true,
            "-h" | "--help" => help = true,
            "--refresh" => refresh = true,
            flag @ ("--db" | "--only" | "--timeout") => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| format!("`{flag}` requires a value."))?;
                match flag {
                    "--db" => database = Some(PathBuf::from(value)),
                    "--only" => only = Some(parse_only(value)?),
                    _ => timeout = Some(parse_timeout(value)?),
                }
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option `{flag}`")),
            value => return Err(format!("unexpected argument `{value}`")),
        }
        index += 1;
    }
    if !refresh && (only.is_some() || timeout.is_some()) {
        return Err("`--only` and `--timeout` apply only with `--refresh`.".into());
    }
    Ok(PaceOptions {
        json,
        help,
        database,
        refresh: refresh.then(|| RefreshOptions {
            only,
            timeout: timeout.unwrap_or(Duration::from_secs(DEFAULT_REFRESH_TIMEOUT_SECONDS)),
        }),
    })
}

fn parse_only(value: &str) -> Result<Vec<String>, String> {
    let mut providers = Vec::new();
    for provider_id in value.split(',').map(str::trim).filter(|id| !id.is_empty()) {
        if !live::is_pullable(provider_id) {
            return Err(format!(
                "`{provider_id}` cannot be refreshed from the command line; choose from {}.",
                live::PULLABLE_PROVIDERS.join(", ")
            ));
        }
        if !providers.iter().any(|known| known == provider_id) {
            providers.push(provider_id.to_owned());
        }
    }
    if providers.is_empty() {
        return Err("`--only` requires at least one provider.".into());
    }
    Ok(providers)
}

fn parse_timeout(value: &str) -> Result<Duration, String> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=MAX_REFRESH_TIMEOUT_SECONDS).contains(seconds))
        .map(Duration::from_secs)
        .ok_or_else(|| {
            format!("`--timeout` takes whole seconds from 1 to {MAX_REFRESH_TIMEOUT_SECONDS}.")
        })
}

fn run_pace(arguments: &[String]) -> i32 {
    let options = match parse_options(arguments) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("openquota pace: {message}");
            eprint!("{USAGE}");
            return EXIT_USAGE;
        }
    };
    if options.help {
        print!("{USAGE}");
        return EXIT_SUCCESS;
    }

    let path = database_path(
        options.database.as_deref(),
        std::env::var_os(DATABASE_OVERRIDE_ENV),
    );
    if !path.exists() {
        eprintln!("OpenQuota database not found at {}.", path.display());
        return EXIT_READ_FAILURE;
    }
    let data_directory = data_directory_of(&path);
    let report = match build_report(
        &options,
        &path,
        live::provider_reader(data_directory.clone()),
        || instance_lock::is_held(&data_directory),
    ) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            return EXIT_READ_FAILURE;
        }
    };
    let exit_code = print_report(&report, &options, &path);
    if let Some(late) = report.late_pulls {
        late.settle(live::LATE_PULL_GRACE);
    }
    exit_code
}

fn print_report(report: &PaceReport, options: &PaceOptions, path: &Path) -> i32 {
    for message in &report.messages {
        eprintln!("{message}");
    }
    if report.rows.is_empty() {
        if options.json {
            println!("[]");
        }
        eprintln!(
            "No readings for the providers turned on in the OpenQuota panel in {}. \
             Start OpenQuota to refresh them.",
            path.display()
        );
        return EXIT_NO_DATA;
    }
    if let Some(notice) = &report.offline_notice {
        eprintln!("{notice}");
    }
    if options.json {
        match serde_json::to_string(&report.rows) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("openquota pace: JSON output failed: {error}");
                return EXIT_READ_FAILURE;
            }
        }
    } else {
        print!(
            "{}",
            format_table(&report.rows, report.offline_notice.as_deref())
        );
    }
    EXIT_SUCCESS
}

fn data_directory_of(database: &Path) -> PathBuf {
    database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Debug)]
struct PaceReport {
    rows: Vec<PaceRow>,
    /// Problems worth a line on stderr, in the order they happened.
    messages: Vec<String>,
    /// Set when the application is not running and some rows are not current.
    offline_notice: Option<String>,
    /// Pulls that missed their time limit; waited for after printing.
    late_pulls: Option<live::LatePulls>,
}

fn build_report(
    options: &PaceOptions,
    path: &Path,
    reader: live::Reader,
    app_running: impl FnOnce() -> Option<bool>,
) -> Result<PaceReport, PaceError> {
    let data_directory = data_directory_of(path);
    let (stored, panel) = {
        // The read-only connection closes before any network work, so a long pull never
        // holds a read snapshot of the application's database.
        let connection = open_readonly(path)?;
        (
            load_snapshots(&connection, path)?,
            load_panel_settings(&connection),
        )
    };
    let mut messages = stored.skipped;
    messages.extend(panel.warnings);

    let cache_path = live::cache_path(&data_directory);
    let cached = live::load_cache(&cache_path).unwrap_or_else(|error| {
        messages.push(format!(
            "Ignoring {}: it could not be read ({error}).",
            cache_path.display()
        ));
        Vec::new()
    });
    let mut shown =
        freshness::newest_readings(stored.snapshots.into_iter().chain(cached.iter().cloned()))
            .into_values()
            .filter(|snapshot| {
                freshness::panel_shows(panel.enabled.as_ref(), &snapshot.provider_id)
            })
            .map(|snapshot| (snapshot, RefreshOutcome::NotRequested))
            .collect::<Vec<_>>();
    shown.sort_by(|left, right| left.0.provider_id.cmp(&right.0.provider_id));

    let mut late_pulls = None;
    if let Some(refresh) = &options.refresh {
        let (fresh, late) = refresh_readings(
            &mut shown,
            refresh,
            panel.enabled.as_ref(),
            Utc::now(),
            reader,
            &mut messages,
        );
        late_pulls = Some(late);
        if !fresh.is_empty() {
            if let Err(error) = live::save_cache(&cache_path, cached, &fresh) {
                messages.push(format!(
                    "Pulled readings could not be kept in {} ({error}); the next run pulls again.",
                    cache_path.display()
                ));
            }
        }
    }

    if shown.len() > EXPORT_LIMIT
        || shown
            .iter()
            .any(|(s, _)| s.quotas.len() > EXPORT_LIMIT || s.value_metrics.len() > EXPORT_LIMIT)
    {
        return Err(PaceError::Query { path: path.to_path_buf(), detail: format!("pace export exceeds {EXPORT_LIMIT} cards or {EXPORT_LIMIT} quotas/value metrics per card; nothing truncated") });
    }
    let now = Utc::now();
    let mut rows = shown
        .into_iter()
        .map(|(snapshot, outcome)| {
            let current = match snapshot.provider_id.as_str() {
                "claude" => crate::providers::claude::observed_account_identity(),
                _ => None,
            };
            let matched = crate::providers::identity::cache_match(
                current
                    .as_deref()
                    .map(crate::providers::CacheIdentity::Resolved)
                    .unwrap_or(crate::providers::CacheIdentity::Unresolved),
                snapshot.account_identity.as_ref(),
            );
            let mut row = build_row(snapshot, now, outcome);
            row.cache_identity_match = matched;
            row
        })
        .collect::<Vec<_>>();
    sort_rows(&mut rows);
    let offline_notice = if rows.is_empty() {
        None
    } else {
        freshness::offline_oldest_hours(
            app_running(),
            rows.iter()
                .map(|row| (row.outcome, row.refreshed_hours_ago)),
        )
        .map(|hours| {
            let locale = resolve_ui_locale(panel.ui_language, &system_language_tag());
            freshness::offline_notice(locale, hours)
        })
    };
    Ok(PaceReport {
        rows,
        messages,
        offline_notice,
        late_pulls,
    })
}

/// Pulls the requested providers into `shown` and returns the readings pulled just now, with
/// the pulls that missed their time limit. Every problem is reported in `messages` and leaves
/// the earlier reading in place.
fn refresh_readings(
    shown: &mut Vec<(ProviderSnapshot, RefreshOutcome)>,
    options: &RefreshOptions,
    enabled: Option<&HashSet<String>>,
    now: DateTime<Utc>,
    reader: live::Reader,
    messages: &mut Vec<String>,
) -> (Vec<ProviderSnapshot>, live::LatePulls) {
    let targets = match &options.only {
        Some(only) => only
            .iter()
            .filter(|provider_id| {
                let shows = freshness::panel_shows(enabled, provider_id);
                if !shows {
                    messages.push(format!(
                        "{provider_id} is turned off in the OpenQuota panel; not refreshed."
                    ));
                }
                shows
            })
            .cloned()
            .collect::<Vec<_>>(),
        None => live::PULLABLE_PROVIDERS
            .iter()
            .filter(|provider_id| match enabled {
                Some(enabled) => enabled.contains(**provider_id),
                // Without panel settings, only providers that were read before are pulled.
                None => shown
                    .iter()
                    .any(|(snapshot, _)| snapshot.provider_id == **provider_id),
            })
            .map(|provider_id| (*provider_id).to_owned())
            .collect(),
    };

    let mut to_pull = Vec::new();
    for provider_id in targets {
        let index = shown
            .iter()
            .position(|(snapshot, _)| snapshot.provider_id == provider_id);
        if freshness::needs_pull(index.map(|index| &shown[index].0), now) {
            to_pull.push(provider_id);
        } else if let Some(index) = index {
            shown[index].1 = RefreshOutcome::Reused;
        }
    }

    let (results, late) = live::pull_all(&to_pull, options.timeout, reader);
    let mut fresh = Vec::new();
    for (provider_id, result) in results {
        let index = shown
            .iter()
            .position(|(snapshot, _)| snapshot.provider_id == provider_id);
        let reason = match result {
            live::PullResult::Fresh(snapshot)
                if snapshot.quotas.len() > EXPORT_LIMIT
                    || snapshot.value_metrics.len() > EXPORT_LIMIT =>
            {
                format!("refresh exceeds pace quota/value metric limit {EXPORT_LIMIT}; nothing truncated")
            }
            live::PullResult::Fresh(snapshot) => {
                let snapshot = crate::providers::cache::cloud_facts(*snapshot);
                fresh.push(snapshot.clone());
                match index {
                    Some(index) => shown[index] = (snapshot, RefreshOutcome::Live),
                    None => shown.push((snapshot, RefreshOutcome::Live)),
                }
                continue;
            }
            live::PullResult::Failed(message) => format!("refresh failed ({message})"),
            live::PullResult::TimedOut => {
                format!("refresh timed out after {}s", options.timeout.as_secs())
            }
        };
        match index {
            Some(index) => {
                let earlier = &mut shown[index];
                earlier.1 = RefreshOutcome::Failed;
                let hours = round1(hours_between(earlier.0.refreshed_at, now).max(0.0));
                messages.push(format!(
                    "{provider_id}: {reason}; showing the reading from {hours:.1} hours ago, marked stale."
                ));
            }
            None => messages.push(format!(
                "{provider_id}: {reason}; there is no earlier reading to show."
            )),
        }
    }
    (fresh, late)
}

#[derive(Debug)]
enum PaceError {
    Open { path: PathBuf, detail: String },
    Query { path: PathBuf, detail: String },
    TableMissing { path: PathBuf },
}

impl fmt::Display for PaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, detail } => write!(
                formatter,
                "OpenQuota database could not be opened read-only at {}: {detail}",
                path.display()
            ),
            Self::Query { path, detail } => write!(
                formatter,
                "OpenQuota database could not be read at {}: {detail}",
                path.display()
            ),
            Self::TableMissing { path } => write!(
                formatter,
                "Table {SNAPSHOT_TABLE} was not found in {}.",
                path.display()
            ),
        }
    }
}

#[derive(Debug, Default)]
struct StoredSnapshots {
    snapshots: Vec<ProviderSnapshot>,
    skipped: Vec<String>,
}

/// What `pace` needs from the panel settings stored by the application.
#[derive(Debug, Default)]
struct PanelSettings {
    /// Providers turned on in the panel, or `None` when the settings are unavailable.
    enabled: Option<HashSet<String>>,
    ui_language: UiLanguagePreference,
    warnings: Vec<String>,
}

/// The subset of the application's settings payload that `pace` reads; every other field
/// is ignored so settings written by newer or older releases still parse.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    #[serde(default)]
    providers: Vec<StoredProvider>,
    #[serde(default)]
    ui_language: UiLanguagePreference,
}

#[derive(Debug, Deserialize)]
struct StoredProvider {
    id: String,
    #[serde(default)]
    enabled: bool,
}

fn open_readonly(path: &Path) -> Result<Connection, PaceError> {
    match open_and_probe(path, false) {
        Ok(connection) => Ok(connection),
        Err(first_error) => {
            // A WAL database without its shared-memory sidecar can only be read
            // in immutable mode, and only when no write-ahead log content would
            // be skipped by it.
            if wal_has_content(path) {
                return Err(first_error);
            }
            match open_and_probe(path, true) {
                Ok(connection) => Ok(connection),
                Err(_) => Err(first_error),
            }
        }
    }
}

fn open_and_probe(path: &Path, immutable: bool) -> Result<Connection, PaceError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let opened = if immutable {
        Connection::open_with_flags(database_uri(path), flags)
    } else {
        Connection::open_with_flags(path, flags)
    };
    let connection = opened.map_err(|error| PaceError::Open {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|error| PaceError::Query {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
    Ok(connection)
}

fn database_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(raw.len() + 16);
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    format!("file:{encoded}?immutable=1")
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn wal_has_content(path: &Path) -> bool {
    std::fs::metadata(sidecar_path(path, "-wal"))
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

fn load_snapshots(connection: &Connection, path: &Path) -> Result<StoredSnapshots, PaceError> {
    let query_error = |error: rusqlite::Error| PaceError::Query {
        path: path.to_path_buf(),
        detail: error.to_string(),
    };
    if !table_exists(connection, SNAPSHOT_TABLE).map_err(query_error)? {
        return Err(PaceError::TableMissing {
            path: path.to_path_buf(),
        });
    }
    let has_identity = connection
        .prepare("PRAGMA table_info(provider_snapshots)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(query_error)?
        .iter()
        .any(|column| column == "identity_key");
    let identity_column = if has_identity { "identity_key" } else { "NULL" };
    let mut statement = connection
        .prepare(&format!(
            "SELECT provider_id, payload, {identity_column} FROM {SNAPSHOT_TABLE} ORDER BY provider_id"
        ))
        .map_err(query_error)?;
    let mut stored = StoredSnapshots::default();
    let mut entries = statement.query([]).map_err(query_error)?;
    while let Some(entry) = entries.next().map_err(query_error)? {
        let provider_id: String = entry.get(0).unwrap_or_else(|_| "<unknown provider>".into());
        let Ok(payload) = entry.get::<_, String>(1) else {
            stored.skipped.push(format!(
                "Skipping snapshot for {provider_id}: payload is not text."
            ));
            continue;
        };
        match serde_json::from_str::<ProviderSnapshot>(&payload) {
            Ok(snapshot)
                if snapshot.quotas.len() > EXPORT_LIMIT
                    || snapshot.value_metrics.len() > EXPORT_LIMIT =>
            {
                stored.skipped.push(format!("Skipping snapshot for {provider_id}: quota/value metric count {}/{} exceeds pace limit {EXPORT_LIMIT}/{EXPORT_LIMIT}; nothing truncated.", snapshot.quotas.len(), snapshot.value_metrics.len()));
            }
            Ok(mut snapshot) if snapshot.provider_id == provider_id => {
                if snapshot.account_identity.is_none() {
                    let identity: Option<String> = entry.get(2).unwrap_or(None);
                    snapshot.account_identity =
                        crate::providers::identity::account_fact(&provider_id, identity.as_deref());
                }
                stored
                    .snapshots
                    .push(crate::providers::cache::cloud_facts(snapshot))
            }
            Ok(_) => stored.skipped.push(format!(
                "Skipping snapshot for {provider_id}: payload belongs to another provider."
            )),
            Err(error) => stored.skipped.push(format!(
                "Skipping snapshot for {provider_id}: payload could not be parsed ({error})."
            )),
        }
    }
    Ok(stored)
}

/// Reads which providers the panel shows. Settings that are missing or unreadable only
/// cost the filter: every cached provider is shown then.
fn load_panel_settings(connection: &Connection) -> PanelSettings {
    let payload = match table_exists(connection, SETTINGS_TABLE).and_then(|exists| {
        if !exists {
            return Ok(None);
        }
        connection
            .query_row(
                &format!("SELECT payload FROM {SETTINGS_TABLE} WHERE id = 1"),
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
    }) {
        Ok(Some(payload)) => payload,
        Ok(None) => return PanelSettings::default(),
        Err(error) => {
            return PanelSettings {
                warnings: vec![format!(
                    "Panel settings could not be read ({error}); showing every provider."
                )],
                ..PanelSettings::default()
            }
        }
    };
    panel_settings_from_payload(&payload)
}

fn panel_settings_from_payload(payload: &str) -> PanelSettings {
    match serde_json::from_str::<StoredSettings>(payload) {
        Ok(settings) => PanelSettings {
            // The application always lists every provider; an empty list means the settings
            // were never completed, so nothing can be concluded from it.
            enabled: (!settings.providers.is_empty()).then(|| {
                settings
                    .providers
                    .into_iter()
                    .filter(|provider| provider.enabled)
                    .map(|provider| provider.id)
                    .collect()
            }),
            ui_language: settings.ui_language,
            warnings: Vec::new(),
        },
        Err(error) => PanelSettings {
            warnings: vec![format!(
                "Panel settings could not be parsed ({error}); showing every provider."
            )],
            ..PanelSettings::default()
        },
    }
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn database_path(explicit: Option<&Path>, environment: Option<OsString>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Some(value) = environment.filter(|value| !value.is_empty()) {
        return PathBuf::from(value);
    }
    default_database_path()
}

fn default_database_path() -> PathBuf {
    data_directory()
        .join(APP_IDENTIFIER)
        .join(DATABASE_FILE_NAME)
}

fn data_directory() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return home_directory().join("Library").join("Application Support");
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(value) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return PathBuf::from(value);
        }
        return home_directory().join("AppData").join("Roaming");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(value) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(value);
        }
        return home_directory().join(".local").join("share");
    }
    #[allow(unreachable_code)]
    home_directory()
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

const TEXT_COLUMNS: usize = 11;
const RIGHT_ALIGNED: [usize; 7] = [3, 4, 5, 6, 7, 8, 10];

/// Renders the aligned table. `offline_notice` explains the rows marked with `*` when the
/// application is not running.
fn format_table(rows: &[PaceRow], offline_notice: Option<&str>) -> String {
    const HEADERS: [&str; TEXT_COLUMNS] = [
        "PROVIDER",
        "PLAN",
        "WINDOW",
        "USED%",
        "ELAPSED%",
        "SPARE%",
        "RESET(h)",
        "SHORT%",
        "REFRESHED",
        "AGE(h)",
        "DATA",
    ];
    let mut table = vec![HEADERS.map(str::to_owned)];
    let app_offline = offline_notice.is_some();
    table.extend(rows.iter().map(|row| text_cells(row, app_offline)));
    let mut widths = [0; TEXT_COLUMNS];
    for line in &table {
        for (index, cell) in line.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let mut output = String::new();
    for line in &table {
        for (index, cell) in line.iter().enumerate() {
            if index > 0 {
                output.push_str("  ");
            }
            let padding = widths[index].saturating_sub(cell.chars().count());
            if RIGHT_ALIGNED.contains(&index) {
                for _ in 0..padding {
                    output.push(' ');
                }
                output.push_str(cell);
            } else {
                output.push_str(cell);
                for _ in 0..padding {
                    output.push(' ');
                }
            }
        }
        output.push('\n');
    }
    if let Some(notice) = offline_notice {
        output.push_str("* ");
        output.push_str(notice);
        output.push('\n');
    }
    output
}

fn text_cells(row: &PaceRow, app_offline: bool) -> [String; TEXT_COLUMNS] {
    [
        row.provider_id.clone(),
        optional(&row.plan),
        optional(&row.window_id),
        percent(row.used_percent),
        percent(row.period_elapsed_percent),
        spare(row.spare_percent),
        hours(row.hours_to_reset),
        percent(row.short_window_used_percent),
        row.refreshed_at.clone(),
        hours(row.refreshed_hours_ago.into()),
        freshness::data_label(row.outcome, row.stale, app_offline),
    ]
}

fn optional(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "-".into())
}

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("{value:.1}"))
}

fn spare(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("{value:+.1}"))
}

fn hours(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("{value:.1}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use rusqlite::Connection;

    use super::{
        build_report, build_row, comparison_window, database_path, format_table, load_snapshots,
        open_readonly, panel_settings_from_payload, parse_options, run_pace, short_window,
        sort_rows, PaceReport, RefreshOptions, APP_IDENTIFIER, DATABASE_FILE_NAME, EXIT_NO_DATA,
        EXIT_READ_FAILURE, EXIT_SUCCESS, EXIT_USAGE,
    };
    use crate::cli::{freshness::RefreshOutcome, live};
    use crate::models::UiLanguagePreference;
    use crate::models::{ProviderSnapshot, QuotaFormat, QuotaWindow};
    use crate::policy::STALE_AFTER;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap()
    }

    fn window(
        id: &str,
        label: &str,
        period_seconds: u64,
        used_percent: f64,
        resets_at: Option<DateTime<Utc>>,
    ) -> QuotaWindow {
        QuotaWindow {
            id: id.into(),
            label: label.into(),
            used_percent,
            resets_at,
            period_seconds,
            format: QuotaFormat::Percent,
            used_value: None,
            limit_value: None,
            remaining_value: None,
            unit: None,
            estimated: false,
            source_note: None,
        }
    }

    fn snapshot(provider_id: &str, quotas: Vec<QuotaWindow>) -> ProviderSnapshot {
        ProviderSnapshot {
            provider_id: provider_id.into(),
            plan: Some("Plus".into()),
            quotas,
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: now(),
            remembered: false,
            account_identity: None,
            shared_scope: None,
        }
    }

    fn create_database(path: &PathBuf, payloads: &[(&str, &str)]) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS provider_snapshots (
                   provider_id TEXT PRIMARY KEY,
                   payload TEXT NOT NULL,
                   refreshed_at TEXT NOT NULL,
                   identity_key TEXT
                 );",
            )
            .unwrap();
        for (provider_id, payload) in payloads {
            connection
                .execute(
                    "INSERT OR REPLACE INTO provider_snapshots(provider_id, payload, refreshed_at)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![provider_id, payload, now().to_rfc3339()],
                )
                .unwrap();
        }
        drop(connection);
    }

    /// Reset point halfway through a window's period, so 50% has elapsed.
    fn half_period_reset(period_seconds: u64) -> DateTime<Utc> {
        now() + Duration::seconds((period_seconds / 2) as i64)
    }

    #[test]
    fn comparison_window_prefers_weekly_then_the_longest_period() {
        let resets_at = now() + Duration::days(2);
        let quotas = vec![
            window("session", "Session", 18_000, 40.0, Some(resets_at)),
            window("weekly", "Weekly", 604_800, 30.0, Some(resets_at)),
            window(
                "sparkWeekly",
                "Spark Weekly",
                604_800,
                20.0,
                Some(resets_at),
            ),
            window("credits", "Extra Usage", 2_592_000, 10.0, Some(resets_at)),
        ];
        assert_eq!(comparison_window(&quotas).unwrap().id, "weekly");

        let no_weekly = vec![
            window("session", "Session", 18_000, 40.0, Some(resets_at)),
            window("monthly", "Monthly", 2_592_000, 10.0, Some(resets_at)),
            window("sonnet", "Sonnet", 604_800, 15.0, Some(resets_at)),
        ];
        assert_eq!(comparison_window(&no_weekly).unwrap().id, "monthly");

        let dollars = vec![QuotaWindow {
            format: QuotaFormat::Dollars,
            ..window("extra", "Extra Usage", 0, 90.0, None)
        }];
        assert!(comparison_window(&dollars).is_none());
        assert!(comparison_window(&[]).is_none());
    }

    #[test]
    fn timed_absolute_quotas_preserve_existing_pace_summary() {
        for format in [QuotaFormat::Count, QuotaFormat::Dollars] {
            let quotas = vec![
                QuotaWindow {
                    format,
                    ..window(
                        "session",
                        "Session",
                        18_000,
                        99.6,
                        Some(now() + Duration::hours(2)),
                    )
                },
                QuotaWindow {
                    format,
                    ..window(
                        "weekly",
                        "Weekly",
                        604_800,
                        23.456,
                        Some(now() + Duration::days(2)),
                    )
                },
            ];
            let row = build_row(
                snapshot("zai", quotas),
                now(),
                super::RefreshOutcome::NotRequested,
            );
            assert_eq!(row.window_id.as_deref(), Some("weekly"));
            assert_eq!(row.used_percent, Some(23.456));
            assert_eq!(row.short_window_id.as_deref(), Some("session"));
            assert_eq!(row.short_window_used_percent, Some(99.6));
            assert!(row.spare_percent.is_some());
            assert!(row.hours_to_reset.is_some());
        }
    }

    #[test]
    fn short_window_prefers_session_then_five_hour_periods() {
        let resets_at = now() + Duration::hours(2);
        let quotas = vec![
            window("weekly", "Weekly", 604_800, 30.0, Some(resets_at)),
            window("session", "Session", 18_000, 40.0, Some(resets_at)),
        ];
        assert_eq!(short_window(&quotas).unwrap().id, "session");

        let without_session = vec![
            window("weekly", "Weekly", 604_800, 30.0, Some(resets_at)),
            window("hourly", "Hourly", 3_600, 5.0, Some(resets_at)),
            window("quick", "Quick", 7_200, 5.0, Some(resets_at)),
        ];
        assert_eq!(short_window(&without_session).unwrap().id, "hourly");

        let monthly_only = vec![window(
            "monthly",
            "Monthly",
            2_592_000,
            30.0,
            Some(resets_at),
        )];
        assert!(short_window(&monthly_only).is_none());
    }

    #[test]
    fn rows_sort_by_spare_descending_with_unknown_values_last() {
        let resets_at = half_period_reset(604_800);
        let mut rows = vec![
            build_row(
                snapshot(
                    "slow",
                    vec![window("weekly", "Weekly", 604_800, 10.0, Some(resets_at))],
                ),
                now(),
                RefreshOutcome::NotRequested,
            ),
            build_row(
                snapshot(
                    "fast",
                    vec![window("weekly", "Weekly", 604_800, 60.0, Some(resets_at))],
                ),
                now(),
                RefreshOutcome::NotRequested,
            ),
            build_row(
                snapshot("unknown", vec![window("weekly", "Weekly", 0, 20.0, None)]),
                now(),
                RefreshOutcome::NotRequested,
            ),
        ];
        sort_rows(&mut rows);

        let order = rows
            .iter()
            .map(|row| row.provider_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, ["slow", "fast", "unknown"]);
        assert_eq!(rows[0].spare_percent, Some(40.0));
        assert_eq!(rows[1].spare_percent, Some(-10.0));
        assert_eq!(rows[2].spare_percent, None);
    }

    #[test]
    fn json_output_uses_camel_case_and_iso_8601_utc() {
        let row = build_row(
            snapshot(
                "codex",
                vec![
                    window(
                        "session",
                        "Session",
                        18_000,
                        25.0,
                        Some(half_period_reset(18_000)),
                    ),
                    window(
                        "weekly",
                        "Weekly",
                        604_800,
                        40.0,
                        Some(half_period_reset(604_800)),
                    ),
                ],
            ),
            now(),
            RefreshOutcome::NotRequested,
        );
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "quotas": serde_json::to_value(&row.quotas).unwrap(),
                "valueMetrics": [], "statusMetrics": [], "notices": [], "warnings": [],
                "accountIdentity": null, "sharedScope": null, "cacheIdentityMatch": "unknown",
                "remembered": false, "lastAttemptAt": null, "errorKind": null,
                "refreshOutcome": "notRequested", "dataQuality": "cache",
                "quotaCount": 2, "quotaLimit": 64, "valueMetricCount": 0, "valueMetricLimit": 64,
                "providerId": "codex",
                "plan": "Plus",
                "windowId": "weekly",
                "windowLabel": "Weekly",
                "usedPercent": 40.0,
                "periodElapsedPercent": 50.0,
                "sparePercent": 10.0,
                "hoursToReset": 84.0,
                "shortWindowId": "session",
                "shortWindowUsedPercent": 25.0,
                "refreshedAt": "2027-01-15T08:00:00Z",
                "refreshedHoursAgo": 0.0,
                "stale": false,
            })
        );
    }

    #[test]
    fn text_table_aligns_every_column() {
        let resets_at = half_period_reset(604_800);
        let rows = vec![
            build_row(
                snapshot(
                    "codex",
                    vec![window("weekly", "Weekly", 604_800, 40.0, Some(resets_at))],
                ),
                now(),
                RefreshOutcome::NotRequested,
            ),
            build_row(
                snapshot(
                    "claude-max",
                    vec![window("weekly", "Weekly", 604_800, 4.0, Some(resets_at))],
                ),
                now(),
                RefreshOutcome::NotRequested,
            ),
        ];
        let table = format_table(&rows, None);
        let lines = table.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("PROVIDER"));
        assert!(lines[0].contains("SPARE%"));
        let widths = lines
            .iter()
            .map(|line| line.len())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(widths.len(), 1, "every line has the same width: {table:?}");
        assert!(lines[2].contains("claude-max"));
        assert!(!table.ends_with(" \n"));
    }

    #[test]
    fn rows_are_labelled_stale_once_they_pass_the_staleness_window() {
        let recent = snapshot("claude", vec![]);
        let mut old = snapshot("codex", vec![]);
        old.refreshed_at = now() - STALE_AFTER - chrono::Duration::seconds(60);

        assert!(!build_row(recent.clone(), now(), RefreshOutcome::NotRequested).stale);
        assert!(build_row(old.clone(), now(), RefreshOutcome::NotRequested).stale);

        let table = format_table(
            &[
                build_row(recent, now(), RefreshOutcome::NotRequested),
                build_row(old, now(), RefreshOutcome::NotRequested),
            ],
            None,
        );
        let lines = table.lines().collect::<Vec<_>>();
        assert!(lines[0].ends_with("DATA"));
        assert!(lines[1].ends_with("cached"), "{table}");
        assert!(lines[2].ends_with("stale"), "{table}");
    }

    #[test]
    fn unreadable_snapshots_are_skipped_without_dropping_the_report() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE_NAME);
        let resets_at = half_period_reset(604_800);
        let valid = serde_json::to_string(&snapshot(
            "codex",
            vec![window("weekly", "Weekly", 604_800, 40.0, Some(resets_at))],
        ))
        .unwrap();
        create_database(&path, &[("codex", &valid), ("broken", "{not json")]);

        let connection = open_readonly(&path).unwrap();
        let stored = load_snapshots(&connection, &path).unwrap();
        assert_eq!(stored.snapshots.len(), 1);
        assert_eq!(stored.snapshots[0].provider_id, "codex");
        assert_eq!(stored.skipped.len(), 1);
        assert!(stored.skipped[0].contains("Skipping snapshot for broken"));
    }

    #[test]
    fn missing_snapshot_table_is_a_read_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE_NAME);
        Connection::open(&path).unwrap().close().unwrap();

        let connection = open_readonly(&path).unwrap();
        let error = load_snapshots(&connection, &path).unwrap_err();
        assert!(error.to_string().contains("provider_snapshots"));
    }

    #[test]
    fn exit_codes_distinguish_success_no_data_and_read_failures() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE_NAME);
        create_database(&path, &[]);

        let missing = directory.path().join("absent.db");
        assert_eq!(
            run_pace(&["--db".into(), missing.to_string_lossy().into()]),
            EXIT_READ_FAILURE
        );
        assert_eq!(
            run_pace(&["--db".into(), path.to_string_lossy().into()]),
            EXIT_NO_DATA
        );
        assert_eq!(run_pace(&["--db".into()]), EXIT_USAGE);
        assert_eq!(run_pace(&["--nope".into()]), EXIT_USAGE);
        assert_eq!(run_pace(&["--help".into()]), EXIT_SUCCESS);

        let resets_at = half_period_reset(604_800);
        let valid = serde_json::to_string(&snapshot(
            "codex",
            vec![window("weekly", "Weekly", 604_800, 40.0, Some(resets_at))],
        ))
        .unwrap();
        create_database(&path, &[("codex", &valid)]);
        assert_eq!(
            run_pace(&["--db".into(), path.to_string_lossy().into()]),
            EXIT_SUCCESS
        );
        assert_eq!(
            run_pace(&[
                "--json".into(),
                "--db".into(),
                path.to_string_lossy().into()
            ]),
            EXIT_SUCCESS
        );
    }

    #[test]
    fn database_path_prefers_the_flag_then_the_environment() {
        let explicit = PathBuf::from("/tmp/explicit.db");
        assert_eq!(
            database_path(Some(&explicit), Some("/tmp/env.db".into())),
            explicit
        );
        assert_eq!(
            database_path(None, Some("/tmp/env.db".into())),
            PathBuf::from("/tmp/env.db")
        );
        let default = database_path(None, Some(String::new().into()));
        assert!(default.ends_with(PathBuf::from(APP_IDENTIFIER).join(DATABASE_FILE_NAME)));
    }

    #[test]
    fn database_identifier_matches_the_tauri_configuration() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let configuration = std::fs::read_to_string(manifest.join("tauri.conf.json")).unwrap();
        assert!(
            configuration.contains(&format!("\"identifier\": \"{APP_IDENTIFIER}\"")),
            "tauri.conf.json must keep the identifier used by `openquota pace`"
        );
    }

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// A reading taken `age` before the real clock, since reports read `Utc::now()`.
    fn aged(provider_id: &str, age: Duration) -> String {
        let mut reading = snapshot(
            provider_id,
            vec![window(
                "weekly",
                "Weekly",
                604_800,
                40.0,
                Some(Utc::now() + Duration::days(3)),
            )],
        );
        reading.refreshed_at = Utc::now() - age;
        serde_json::to_string(&reading).unwrap()
    }

    fn store_settings(path: &PathBuf, payload: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS app_settings (
                   id INTEGER PRIMARY KEY CHECK (id = 1),
                   payload TEXT NOT NULL
                 );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO app_settings(id, payload) VALUES (1, ?1)",
                [payload],
            )
            .unwrap();
    }

    const PANEL_SETTINGS: &str = r#"{
        "schemaVersion": 7,
        "uiLanguage": "zh",
        "futureField": {"kept": true},
        "providers": [
            {"id": "claude", "enabled": true, "detected": true, "expanded": false, "metrics": []},
            {"id": "cursor", "enabled": true, "detected": true, "expanded": false, "metrics": []},
            {"id": "grok", "enabled": true, "detected": true, "expanded": false, "metrics": []},
            {"id": "kimi", "enabled": true, "detected": true, "expanded": false, "metrics": []},
            {"id": "antigravity", "enabled": true, "detected": false, "expanded": false, "metrics": []},
            {"id": "copilot", "enabled": false, "detected": true, "expanded": false, "metrics": []}
        ]
    }"#;

    /// A database shaped like the one on the reporting machine: Copilot turned off with a
    /// reading from days ago, the rest enabled.
    fn panel_database(directory: &std::path::Path) -> PathBuf {
        let path = directory.join(DATABASE_FILE_NAME);
        create_database(
            &path,
            &[
                ("claude", &aged("claude", Duration::hours(1))),
                ("copilot", &aged("copilot", Duration::minutes(6_810))),
                ("cursor", &aged("cursor", Duration::hours(5))),
                ("grok", &aged("grok", Duration::hours(2))),
                ("kimi", &aged("kimi", Duration::seconds(20))),
            ],
        );
        store_settings(&path, PANEL_SETTINGS);
        path
    }

    fn report(
        path: &std::path::Path,
        flags: &[&str],
        reader: live::Reader,
        app_running: Option<bool>,
    ) -> PaceReport {
        let options = parse_options(&arguments(flags)).unwrap();
        build_report(&options, path, reader, || app_running).unwrap()
    }

    fn unreachable_reader() -> live::Reader {
        std::sync::Arc::new(|provider_id| panic!("{provider_id} must not be pulled"))
    }

    fn row<'a>(report: &'a PaceReport, provider_id: &str) -> &'a super::PaceRow {
        report
            .rows
            .iter()
            .find(|row| row.provider_id == provider_id)
            .unwrap_or_else(|| panic!("{provider_id} missing from {:?}", report.rows))
    }

    #[test]
    fn refresh_options_parse_and_reject_what_cannot_be_pulled() {
        let plain = parse_options(&arguments(&["--json"])).unwrap();
        assert!(plain.json);
        assert_eq!(plain.refresh, None);

        let everything = parse_options(&arguments(&["--refresh"])).unwrap();
        assert_eq!(
            everything.refresh,
            Some(RefreshOptions {
                only: None,
                timeout: std::time::Duration::from_secs(20),
            })
        );

        let chosen = parse_options(&arguments(&[
            "--refresh",
            "--only",
            " cursor,grok,,cursor ,kimi,antigravity",
            "--timeout",
            "5",
        ]))
        .unwrap();
        assert_eq!(
            chosen.refresh,
            Some(RefreshOptions {
                only: Some(arguments(&["cursor", "grok", "kimi", "antigravity"])),
                timeout: std::time::Duration::from_secs(5),
            })
        );

        assert!(parse_options(&arguments(&[
            "--refresh",
            "--only",
            "codex,grok,kimi,opencode,zai"
        ]))
        .is_ok());

        for invalid in [
            &["--only", "cursor"][..],
            &["--timeout", "5"],
            &["--refresh", "--only"],
            &["--refresh", "--only", ","],
            &["--refresh", "--only", "claude"],
            &["--refresh", "--only", "nope"],
            &["--refresh", "--timeout", "0"],
            &["--refresh", "--timeout", "601"],
            &["--refresh", "--timeout", "1.5"],
            &["--refresh", "--timeout"],
        ] {
            assert!(parse_options(&arguments(invalid)).is_err(), "{invalid:?}");
            assert_eq!(run_pace(&arguments(invalid)), EXIT_USAGE, "{invalid:?}");
        }
    }

    #[test]
    fn panel_settings_list_enabled_providers_and_tolerate_other_shapes() {
        let settings = panel_settings_from_payload(PANEL_SETTINGS);
        let enabled = settings.enabled.unwrap();
        assert!(enabled.contains("cursor"));
        assert!(!enabled.contains("copilot"));
        assert_eq!(settings.ui_language, UiLanguagePreference::Zh);
        assert!(settings.warnings.is_empty());

        let legacy = panel_settings_from_payload(r#"{"theme":"dark","uiLanguage":"klingon"}"#);
        assert_eq!(legacy.enabled, None);
        assert_eq!(legacy.ui_language, UiLanguagePreference::System);
        assert!(legacy.warnings.is_empty());

        let broken = panel_settings_from_payload("{not json");
        assert_eq!(broken.enabled, None);
        assert_eq!(broken.warnings.len(), 1);
    }

    #[test]
    fn providers_turned_off_in_the_panel_are_not_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = panel_database(directory.path());

        let report = report(&path, &[], unreachable_reader(), Some(true));
        let providers = report
            .rows
            .iter()
            .map(|row| row.provider_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            providers,
            ["claude", "cursor", "grok", "kimi"].into_iter().collect()
        );
        assert!(report.messages.is_empty(), "{:?}", report.messages);
        assert_eq!(report.offline_notice, None);
    }

    #[test]
    fn a_closed_app_is_named_with_the_age_of_the_oldest_reading() {
        let directory = tempfile::tempdir().unwrap();
        let path = panel_database(directory.path());

        let closed = report(&path, &[], unreachable_reader(), Some(false));
        assert_eq!(
            closed.offline_notice.as_deref(),
            Some("OpenQuota 没在运行，下列读数最久 5.0 小时前。")
        );
        assert!(row(&closed, "cursor").stale);
        assert_eq!(row(&closed, "cursor").refreshed_hours_ago, 5.0);
        assert!(!row(&closed, "kimi").stale);

        let table = format_table(&closed.rows, closed.offline_notice.as_deref());
        let lines = table.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), closed.rows.len() + 2);
        assert!(lines[1..=closed.rows.len()]
            .iter()
            .all(
                |line| line.trim_end().ends_with("stale*") || line.trim_end().ends_with("cached*")
            ));
        assert_eq!(
            *lines.last().unwrap(),
            "* OpenQuota 没在运行，下列读数最久 5.0 小时前。"
        );

        assert_eq!(
            report(&path, &[], unreachable_reader(), None).offline_notice,
            None
        );
    }

    #[test]
    fn refresh_pulls_enabled_providers_and_keeps_earlier_readings_when_a_pull_fails() {
        let directory = tempfile::tempdir().unwrap();
        let path = panel_database(directory.path());
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let reader: live::Reader = {
            let calls = calls.clone();
            std::sync::Arc::new(move |provider_id| {
                calls.lock().unwrap().push(provider_id.to_owned());
                match provider_id {
                    "cursor" => {
                        Ok(serde_json::from_str(&aged("cursor", Duration::zero())).unwrap())
                    }
                    "antigravity" => Err("Antigravity is not running.".into()),
                    "grok" => {
                        std::thread::sleep(std::time::Duration::from_secs(3));
                        Err("late".into())
                    }
                    other => panic!("{other} must not be pulled"),
                }
            })
        };

        let refreshed = report(&path, &["--refresh", "--timeout", "1"], reader, Some(false));
        let mut pulled = calls.lock().unwrap().clone();
        pulled.sort();
        assert_eq!(pulled, ["antigravity", "cursor", "grok"]);

        let cursor = row(&refreshed, "cursor");
        assert_eq!(cursor.outcome, RefreshOutcome::Live);
        assert!(!cursor.stale);
        assert_eq!(cursor.refreshed_hours_ago, 0.0);
        let grok = row(&refreshed, "grok");
        assert_eq!(grok.outcome, RefreshOutcome::Failed);
        assert!(grok.stale);
        assert_eq!(grok.refreshed_hours_ago, 2.0);
        assert_eq!(row(&refreshed, "kimi").outcome, RefreshOutcome::Reused);
        assert!(!row(&refreshed, "kimi").stale);
        assert_eq!(
            row(&refreshed, "claude").outcome,
            RefreshOutcome::NotRequested
        );
        assert!(refreshed
            .rows
            .iter()
            .all(|row| row.provider_id != "antigravity"));
        assert!(refreshed
            .rows
            .iter()
            .all(|row| row.provider_id != "copilot"));
        assert_eq!(
            refreshed.messages,
            [
                "antigravity: refresh failed (Antigravity is not running.); there is no earlier reading to show.",
                "grok: refresh timed out after 1s; showing the reading from 2.0 hours ago, marked stale.",
            ]
        );
        assert_eq!(
            refreshed.offline_notice.as_deref(),
            Some("OpenQuota 没在运行，下列读数最久 2.0 小时前。")
        );

        // The pulled reading stays out of the application database and is reused by the next
        // run within a minute instead of being pulled again.
        let connection = open_readonly(&path).unwrap();
        let stored = load_snapshots(&connection, &path).unwrap();
        let stored_cursor = stored
            .snapshots
            .iter()
            .find(|snapshot| snapshot.provider_id == "cursor")
            .unwrap();
        assert!(Utc::now() - stored_cursor.refreshed_at > Duration::hours(4));
        drop(connection);
        let again = report(
            &path,
            &["--refresh", "--only", "cursor", "--json"],
            unreachable_reader(),
            Some(true),
        );
        assert_eq!(row(&again, "cursor").outcome, RefreshOutcome::Reused);
        assert!(!row(&again, "cursor").stale);
        assert_eq!(row(&again, "cursor").refreshed_hours_ago, 0.0);
        let plain = report(&path, &[], unreachable_reader(), Some(true));
        assert_eq!(row(&plain, "cursor").refreshed_hours_ago, 0.0);
        assert_eq!(row(&plain, "cursor").outcome, RefreshOutcome::NotRequested);
    }

    #[test]
    fn only_pulls_the_named_enabled_providers() {
        let directory = tempfile::tempdir().unwrap();
        let path = panel_database(directory.path());
        let reader: live::Reader = std::sync::Arc::new(|provider_id| match provider_id {
            "cursor" => Ok(serde_json::from_str(&aged("cursor", Duration::zero())).unwrap()),
            other => panic!("{other} must not be pulled"),
        });

        let refreshed = report(
            &path,
            &["--refresh", "--only", "cursor,copilot"],
            reader,
            Some(true),
        );
        assert_eq!(row(&refreshed, "cursor").outcome, RefreshOutcome::Live);
        assert_eq!(
            row(&refreshed, "grok").outcome,
            RefreshOutcome::NotRequested
        );
        assert!(refreshed
            .rows
            .iter()
            .all(|row| row.provider_id != "copilot"));
        assert_eq!(
            refreshed.messages,
            ["copilot is turned off in the OpenQuota panel; not refreshed."]
        );
    }

    #[test]
    fn databases_with_obsolete_tables_and_old_settings_still_report() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE_NAME);
        create_database(
            &path,
            &[
                ("codex", &aged("codex", Duration::minutes(1))),
                ("copilot", &aged("copilot", Duration::minutes(1))),
            ],
        );
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE daily_usage (tokens INTEGER);
                 INSERT INTO daily_usage VALUES (42);
                 CREATE TABLE log_file_cache (modified_millis INTEGER);
                 INSERT INTO log_file_cache VALUES (1);",
            )
            .unwrap();
        store_settings(
            &path,
            r#"{"theme":"dark","showTotalSpend":true,"totalSpendPeriod":"today"}"#,
        );

        let report = report(&path, &[], unreachable_reader(), Some(true));
        assert_eq!(report.rows.len(), 2);
        assert!(report.messages.is_empty(), "{:?}", report.messages);
        let connection = Connection::open(&path).unwrap();
        let obsolete: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('daily_usage', 'log_file_cache')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(obsolete, 2, "pace never changes the application database");
    }
}
