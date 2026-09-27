//! Headless subcommands handled before the Tauri application starts.
//!
//! A subcommand must not launch the GUI, register the single-instance plugin,
//! refresh providers over the network, or write to the application database.

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::{
    models::{ProviderSnapshot, QuotaFormat, QuotaWindow},
    pacing,
    policy::STALE_AFTER,
};

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
const DATABASE_OVERRIDE_ENV: &str = "OPENQUOTA_PACE_DB";
/// Identifier from `src-tauri/tauri.conf.json`; the application database lives
/// at `<platform data dir>/<identifier>/openquota.db`.
const APP_IDENTIFIER: &str = "io.github.deviffyy.openquota";
/// Session style windows stay at or below five hours; one hour of tolerance
/// keeps slightly longer session windows in the short column.
const SHORT_WINDOW_MAX_PERIOD_SECONDS: u64 = 6 * 60 * 60;

const USAGE: &str = "\
Usage: openquota pace [--json] [--db <path>]

Prints the latest cached provider quotas from the local OpenQuota database,
opened read-only. No GUI, no network access, and no writes.

The DATA column reads `cached` for a recent read and `stale` once the numbers are
older than the panel's staleness window, so remembered values are never mistaken for
current ones.

Options:
  --json          Print a stable JSON array instead of a text table
  --db <path>     Read this database file instead of the default (for testing)
  -h, --help      Show this help

Exit codes:
  0   report printed
  1   database could not be read
  2   database holds no provider snapshots
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

struct PaceOptions {
    json: bool,
    help: bool,
    database: Option<PathBuf>,
}

fn parse_options(arguments: &[String]) -> Result<PaceOptions, String> {
    let mut options = PaceOptions {
        json: false,
        help: false,
        database: None,
    };
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--json" => options.json = true,
            "-h" | "--help" => options.help = true,
            "--db" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| "`--db` requires a path.".to_owned())?;
                options.database = Some(PathBuf::from(value));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option `{flag}`")),
            value => return Err(format!("unexpected argument `{value}`")),
        }
        index += 1;
    }
    Ok(options)
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
    let connection = match open_readonly(&path) {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("{error}");
            return EXIT_READ_FAILURE;
        }
    };
    let report = match load_rows(&connection, Utc::now(), &path) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            return EXIT_READ_FAILURE;
        }
    };
    for warning in &report.skipped {
        eprintln!("{warning}");
    }
    if report.rows.is_empty() {
        if options.json {
            println!("[]");
        }
        eprintln!(
            "No provider snapshots found in {}. Start OpenQuota to refresh them.",
            path.display()
        );
        return EXIT_NO_DATA;
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
        print!("{}", format_table(&report.rows));
    }
    EXIT_SUCCESS
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

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PaceRow {
    provider_id: String,
    plan: Option<String>,
    window_id: Option<String>,
    window_label: Option<String>,
    used_percent: Option<f64>,
    period_elapsed_percent: Option<f64>,
    spare_percent: Option<f64>,
    hours_to_reset: Option<f64>,
    short_window_id: Option<String>,
    short_window_used_percent: Option<f64>,
    refreshed_at: String,
    refreshed_hours_ago: f64,
    /// The row is older than the staleness the desktop panel marks, so its numbers
    /// are a remembered last read rather than a current one.
    stale: bool,
}

#[derive(Debug, Default)]
struct PaceReport {
    rows: Vec<PaceRow>,
    skipped: Vec<String>,
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

fn load_rows(
    connection: &Connection,
    now: DateTime<Utc>,
    path: &Path,
) -> Result<PaceReport, PaceError> {
    if !table_exists(connection).map_err(|error| PaceError::Query {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })? {
        return Err(PaceError::TableMissing {
            path: path.to_path_buf(),
        });
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT provider_id, payload FROM {SNAPSHOT_TABLE} ORDER BY provider_id"
        ))
        .map_err(|error| PaceError::Query {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
    let mut report = PaceReport::default();
    let mut entries = statement.query([]).map_err(|error| PaceError::Query {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    while let Some(entry) = entries.next().map_err(|error| PaceError::Query {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })? {
        let provider_id: String = entry.get(0).unwrap_or_else(|_| "<unknown provider>".into());
        let Ok(payload) = entry.get::<_, String>(1) else {
            report.skipped.push(format!(
                "Skipping snapshot for {provider_id}: payload is not text."
            ));
            continue;
        };
        match serde_json::from_str::<ProviderSnapshot>(&payload) {
            Ok(snapshot) => report.rows.push(build_row(snapshot, now)),
            Err(error) => report.skipped.push(format!(
                "Skipping snapshot for {provider_id}: payload could not be parsed ({error})."
            )),
        }
    }
    sort_rows(&mut report.rows);
    Ok(report)
}

fn table_exists(connection: &Connection) -> Result<bool, rusqlite::Error> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [SNAPSHOT_TABLE],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn build_row(snapshot: ProviderSnapshot, now: DateTime<Utc>) -> PaceRow {
    let comparison = comparison_window(&snapshot.quotas);
    let short = short_window(&snapshot.quotas);
    PaceRow {
        provider_id: snapshot.provider_id,
        plan: snapshot.plan,
        window_id: comparison.map(|window| window.id.clone()),
        window_label: comparison.map(|window| window.label.clone()),
        used_percent: comparison.map(|window| round1(window.used_percent.clamp(0.0, 100.0))),
        period_elapsed_percent: comparison
            .and_then(|window| pacing::period_elapsed_percent(window, now))
            .map(round1),
        spare_percent: comparison
            .and_then(|window| pacing::spare_percent(window, now))
            .map(round1),
        hours_to_reset: comparison
            .and_then(|window| window.resets_at)
            .map(|reset| round1(hours_between(now, reset))),
        short_window_id: short.map(|window| window.id.clone()),
        short_window_used_percent: short
            .map(|window| round1(window.used_percent.clamp(0.0, 100.0))),
        refreshed_at: snapshot
            .refreshed_at
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        refreshed_hours_ago: round1(hours_between(snapshot.refreshed_at, now).max(0.0)),
        stale: now.signed_duration_since(snapshot.refreshed_at) >= STALE_AFTER,
    }
}

/// Sorts by spare percentage, largest first, keeping providers without a
/// comparable window at the end in their original order.
fn sort_rows(rows: &mut [PaceRow]) {
    rows.sort_by(
        |left, right| match (left.spare_percent, right.spare_percent) {
            (Some(left_spare), Some(right_spare)) => right_spare
                .partial_cmp(&left_spare)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        },
    );
}

fn comparison_window(quotas: &[QuotaWindow]) -> Option<&QuotaWindow> {
    let percent = percent_windows(quotas).collect::<Vec<_>>();
    let weekly = percent
        .iter()
        .copied()
        .filter(|window| matches_name(window, "week"))
        .collect::<Vec<_>>();
    if !weekly.is_empty() {
        return longest_window(weekly);
    }
    longest_window(percent)
}

fn short_window(quotas: &[QuotaWindow]) -> Option<&QuotaWindow> {
    let percent = percent_windows(quotas).collect::<Vec<_>>();
    if let Some(session) = percent
        .iter()
        .copied()
        .find(|window| matches_name(window, "session"))
    {
        return Some(session);
    }
    percent
        .iter()
        .copied()
        .filter(|window| {
            window.period_seconds > 0 && window.period_seconds <= SHORT_WINDOW_MAX_PERIOD_SECONDS
        })
        .reduce(|best, window| {
            if window.period_seconds < best.period_seconds {
                window
            } else {
                best
            }
        })
}

fn percent_windows(quotas: &[QuotaWindow]) -> impl Iterator<Item = &QuotaWindow> {
    quotas
        .iter()
        .filter(|window| window.format == QuotaFormat::Percent)
}

fn matches_name(window: &QuotaWindow, needle: &str) -> bool {
    window.id.to_ascii_lowercase().contains(needle)
        || window.label.to_ascii_lowercase().contains(needle)
}

fn longest_window(windows: Vec<&QuotaWindow>) -> Option<&QuotaWindow> {
    windows.into_iter().reduce(|best, window| {
        if window.period_seconds > best.period_seconds {
            window
        } else {
            best
        }
    })
}

fn hours_between(start: DateTime<Utc>, end: DateTime<Utc>) -> f64 {
    end.signed_duration_since(start).num_seconds() as f64 / 3_600.0
}

fn round1(value: f64) -> f64 {
    // The `+ 0.0` normalizes negative zero away from JSON and text output.
    (value * 10.0).round() / 10.0 + 0.0
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

fn format_table(rows: &[PaceRow]) -> String {
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
    table.extend(rows.iter().map(text_cells));
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
    output
}

fn text_cells(row: &PaceRow) -> [String; TEXT_COLUMNS] {
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
        if row.stale { "stale" } else { "cached" }.to_owned(),
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
        build_row, comparison_window, database_path, format_table, load_rows, open_readonly,
        run_pace, short_window, sort_rows, APP_IDENTIFIER, DATABASE_FILE_NAME, EXIT_NO_DATA,
        EXIT_READ_FAILURE, EXIT_SUCCESS, EXIT_USAGE,
    };
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
            ),
            build_row(
                snapshot(
                    "fast",
                    vec![window("weekly", "Weekly", 604_800, 60.0, Some(resets_at))],
                ),
                now(),
            ),
            build_row(
                snapshot("unknown", vec![window("weekly", "Weekly", 0, 20.0, None)]),
                now(),
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
        );
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
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
            ),
            build_row(
                snapshot(
                    "claude-max",
                    vec![window("weekly", "Weekly", 604_800, 4.0, Some(resets_at))],
                ),
                now(),
            ),
        ];
        let table = format_table(&rows);
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

        assert!(!build_row(recent.clone(), now()).stale);
        assert!(build_row(old.clone(), now()).stale);

        let table = format_table(&[build_row(recent, now()), build_row(old, now())]);
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
        let report = load_rows(&connection, now(), &path).unwrap();
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].provider_id, "codex");
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("Skipping snapshot for broken"));
    }

    #[test]
    fn missing_snapshot_table_is_a_read_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE_NAME);
        Connection::open(&path).unwrap().close().unwrap();

        let connection = open_readonly(&path).unwrap();
        let error = load_rows(&connection, now(), &path).unwrap_err();
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
}
