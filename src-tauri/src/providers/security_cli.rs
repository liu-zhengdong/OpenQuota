//! macOS Keychain access through `/usr/bin/security`.
//!
//! OpenQuota has no Apple team identifier, so the Keychain records each build's
//! cdhash in an item's partition list and asks for the login password again after
//! every update. `/usr/bin/security` is an Apple tool with a stable identity: items
//! other tools create usually already trust it, and items it creates carry the
//! `apple-tool:` partition. Reading and writing through it therefore survives
//! OpenQuota updates without a new prompt.
//!
//! Secrets never appear in arguments. Reads take the password from `-g` output on
//! stderr; writes send the `add-generic-password` command to `security -i` over
//! stdin, which keeps it out of `ps`. `security -i` reads at most about 4 KiB per
//! line, so a write that does not fit is refused with [`WriteError::TooLong`] rather
//! than moved into the argument list.
//!
//! This file depends only on `std` and `zeroize` so it can be compiled on its own for
//! Keychain experiments against a throwaway keychain.

use std::{
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use zeroize::Zeroizing;

pub const SECURITY_PROGRAM: &str = "/usr/bin/security";

/// Comment stored on items OpenQuota creates through `security`, so a later start can
/// tell them apart from items created in-process by builds before 0.8.4.
pub const OWNED_ITEM_COMMENT: &str = "openquota:security-cli";

const EXIT_ITEM_NOT_FOUND: i32 = 44;
const EXIT_DUPLICATE_ITEM: i32 = 45;
/// `security -i` accepted a 4089-byte line and hung on a 4109-byte one when measured
/// on macOS 27; stay clear of that boundary.
pub const MAX_INTERACTIVE_LINE: usize = 4000;

pub type Secret = Zeroizing<Vec<u8>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityError {
    Spawn,
    Failed(Option<i32>),
    UnreadableOutput,
    ValueTooLong,
}

impl std::fmt::Display for SecurityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn => formatter.write_str("/usr/bin/security could not be started"),
            Self::Failed(Some(code)) => write!(formatter, "/usr/bin/security exited with {code}"),
            Self::Failed(None) => formatter.write_str("/usr/bin/security was terminated"),
            Self::UnreadableOutput => {
                formatter.write_str("/usr/bin/security returned an unreadable password")
            }
            Self::ValueTooLong => formatter.write_str("the value is too long for security -i"),
        }
    }
}

pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: Zeroizing<Vec<u8>>,
    pub stderr: Zeroizing<Vec<u8>>,
}

pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &Path,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
    ) -> io::Result<CommandOutput>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(
        &self,
        program: &Path,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
    ) -> io::Result<CommandOutput> {
        let mut child = Command::new(program)
            .args(arguments)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(input) = stdin {
            // Dropping the handle closes the pipe, which ends `security -i`.
            let mut pipe = child.stdin.take().expect("stdin was piped");
            pipe.write_all(input)?;
        }
        let output = child.wait_with_output()?;
        Ok(CommandOutput {
            status: output.status.code(),
            stdout: Zeroizing::new(output.stdout),
            stderr: Zeroizing::new(output.stderr),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Create a new item that trusts `/usr/bin/security`. Fails if the item exists.
    CreateOwned,
    /// Replace the data of an existing item. The item's access list and partition
    /// list are left exactly as they are; passing `-T` here would ask to change the
    /// ACL, which prompts.
    UpdateExisting,
}

pub struct SecurityCli<R = ProcessRunner> {
    program: PathBuf,
    keychain: Option<PathBuf>,
    runner: R,
}

impl SecurityCli<ProcessRunner> {
    pub fn system() -> Self {
        Self::new(PathBuf::from(SECURITY_PROGRAM), None, ProcessRunner)
    }
}

impl<R: CommandRunner> SecurityCli<R> {
    pub fn new(program: PathBuf, keychain: Option<PathBuf>, runner: R) -> Self {
        Self {
            program,
            keychain,
            runner,
        }
    }

    /// Reads a generic password. `account: None` matches any account of `service`.
    pub fn read(
        &self,
        service: &str,
        account: Option<&str>,
    ) -> Result<Option<Secret>, SecurityError> {
        let output = self.run(&self.find_arguments(service, account, true), None)?;
        match output.status {
            Some(0) => parse_password_output(&output.stderr)
                .map(Some)
                .ok_or(SecurityError::UnreadableOutput),
            Some(EXIT_ITEM_NOT_FOUND) => Ok(None),
            status => Err(SecurityError::Failed(status)),
        }
    }

    /// Reads an item's attributes without its password, which never prompts.
    pub fn attributes(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, SecurityError> {
        let output = self.run(&self.find_arguments(service, Some(account), false), None)?;
        match output.status {
            Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned())),
            Some(EXIT_ITEM_NOT_FOUND) => Ok(None),
            status => Err(SecurityError::Failed(status)),
        }
    }

    pub fn write(
        &self,
        service: &str,
        account: &str,
        value: &[u8],
        mode: WriteMode,
    ) -> Result<(), WriteError> {
        let command = add_command_line(service, account, value, mode, self.keychain.as_deref());
        if command.len() > MAX_INTERACTIVE_LINE {
            return Err(WriteError::TooLong);
        }
        let output = self.run(&[OsString::from("-i")], Some(command.as_bytes()))?;
        match output.status {
            Some(0) => Ok(()),
            Some(EXIT_DUPLICATE_ITEM) => Err(WriteError::Duplicate),
            status => Err(WriteError::Security(SecurityError::Failed(status))),
        }
    }

    /// Creates or updates an item OpenQuota owns. New items trust `/usr/bin/security`;
    /// existing ones keep their access control.
    pub fn write_owned(
        &self,
        service: &str,
        account: &str,
        value: &[u8],
    ) -> Result<(), SecurityError> {
        match self.write(service, account, value, WriteMode::CreateOwned) {
            Ok(()) => Ok(()),
            Err(WriteError::Duplicate) => self
                .write(service, account, value, WriteMode::UpdateExisting)
                .map_err(WriteError::into_security),
            Err(error) => Err(error.into_security()),
        }
    }

    pub fn delete(&self, service: &str, account: &str) -> Result<(), SecurityError> {
        let mut arguments = vec![
            OsString::from("delete-generic-password"),
            OsString::from("-s"),
            OsString::from(service),
            OsString::from("-a"),
            OsString::from(account),
        ];
        arguments.extend(self.keychain.iter().map(OsString::from));
        let output = self.run(&arguments, None)?;
        match output.status {
            Some(0) | Some(EXIT_ITEM_NOT_FOUND) => Ok(()),
            status => Err(SecurityError::Failed(status)),
        }
    }

    fn find_arguments(
        &self,
        service: &str,
        account: Option<&str>,
        password: bool,
    ) -> Vec<OsString> {
        let mut arguments = vec![
            OsString::from("find-generic-password"),
            OsString::from("-s"),
            OsString::from(service),
        ];
        if let Some(account) = account {
            arguments.push(OsString::from("-a"));
            arguments.push(OsString::from(account));
        }
        if password {
            arguments.push(OsString::from("-g"));
        }
        arguments.extend(self.keychain.iter().map(OsString::from));
        arguments
    }

    fn run(
        &self,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
    ) -> Result<CommandOutput, SecurityError> {
        self.runner
            .run(&self.program, arguments, stdin)
            .map_err(|_| SecurityError::Spawn)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    Duplicate,
    /// The command would not fit on one `security -i` line.
    TooLong,
    Security(SecurityError),
}

impl WriteError {
    fn into_security(self) -> SecurityError {
        match self {
            Self::Duplicate => SecurityError::Failed(Some(EXIT_DUPLICATE_ITEM)),
            Self::TooLong => SecurityError::ValueTooLong,
            Self::Security(error) => error,
        }
    }
}

impl From<SecurityError> for WriteError {
    fn from(error: SecurityError) -> Self {
        Self::Security(error)
    }
}

/// Builds one `security -i` command line. Printable ASCII travels quoted as `-w`, which
/// keeps JSON credentials short; anything else goes as `-X <hex>`. Strings are
/// double-quoted with `"` and `\` escaped, which is all the interactive parser needs.
fn add_command_line(
    service: &str,
    account: &str,
    value: &[u8],
    mode: WriteMode,
    keychain: Option<&Path>,
) -> Zeroizing<String> {
    // Sized up front so growing the buffer never leaves a stray copy of the value.
    let capacity = 128
        + service.len() * 2
        + account.len() * 2
        + value.len() * 2
        + keychain.map_or(0, |path| path.as_os_str().len() * 2);
    let mut line = Zeroizing::new(String::with_capacity(capacity));
    line.push_str("add-generic-password");
    if mode == WriteMode::UpdateExisting {
        line.push_str(" -U");
    }
    line.push_str(" -s ");
    line.push_str(&quote(service));
    line.push_str(" -a ");
    line.push_str(&quote(account));
    if mode == WriteMode::CreateOwned {
        line.push_str(" -j ");
        line.push_str(&quote(OWNED_ITEM_COMMENT));
        line.push_str(" -T ");
        line.push_str(&quote(SECURITY_PROGRAM));
    }
    if value.iter().all(|byte| (0x20..0x7f).contains(byte)) {
        line.push_str(" -w ");
        // Printable ASCII is valid UTF-8.
        push_quoted(&mut line, std::str::from_utf8(value).unwrap_or_default());
    } else {
        line.push_str(" -X ");
        push_hex(&mut line, value);
    }
    if let Some(keychain) = keychain {
        line.push(' ');
        line.push_str(&quote(&keychain.to_string_lossy()));
    }
    line.push('\n');
    line
}

fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    push_quoted(&mut quoted, value);
    quoted
}

fn push_quoted(target: &mut String, value: &str) {
    target.push('"');
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            target.push('\\');
        }
        target.push(character);
    }
    target.push('"');
}

fn push_hex(target: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    target.reserve(bytes.len() * 2);
    for byte in bytes {
        target.push(DIGITS[usize::from(byte >> 4)] as char);
        target.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
}

/// Parses the `password:` line `find-generic-password -g` prints on stderr:
/// `password: "text"` for printable data, `password: 0x<HEX>  "<escaped>"` otherwise,
/// and `password: ` for an empty value.
fn parse_password_output(stderr: &[u8]) -> Option<Secret> {
    let line = stderr
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"password:"))?;
    let line = line.strip_prefix(b" ").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() {
        return Some(Zeroizing::new(Vec::new()));
    }
    if let Some(hex) = line.strip_prefix(b"0x") {
        let end = hex
            .iter()
            .position(|byte| !byte.is_ascii_hexdigit())
            .unwrap_or(hex.len());
        return decode_hex(&hex[..end]);
    }
    let text = line.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
    Some(Zeroizing::new(text.to_vec()))
}

fn decode_hex(hex: &[u8]) -> Option<Secret> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |digit: u8| (digit as char).to_digit(16).map(|value| value as u8);
    let mut bytes = Zeroizing::new(Vec::with_capacity(hex.len() / 2));
    for [high, low] in hex.as_chunks::<2>().0 {
        bytes.push(nibble(*high)? << 4 | nibble(*low)?);
    }
    Some(bytes)
}

/// Whether `find-generic-password` attribute output belongs to an item OpenQuota
/// created through `security`.
pub fn is_owned_item(attributes: &str) -> bool {
    let marker = format!("\"icmt\"<blob>=\"{OWNED_ITEM_COMMENT}\"");
    attributes.lines().any(|line| line.trim() == marker)
}

/// In-process Keychain access, as used by builds before 0.8.4 for their own items.
pub trait LegacyKeychain {
    fn read(&self, service: &str, account: &str) -> Result<Option<Secret>, String>;
    fn write(&self, service: &str, account: &str, value: &[u8]) -> Result<(), String>;
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    Missing,
    AlreadyCurrent,
    Migrated,
    /// The old item could not be read, or the new one could not be created and the
    /// old one was restored. The old item stays usable in-process.
    KeptLegacy(String),
    /// Recreating failed and restoring the old item failed too.
    Lost(String),
}

/// Moves one item created in-process by an older build to an item created through
/// `security`. A Keychain can hold only one item per service and account, so the old
/// item is removed before the new one is added; if adding fails the old value is
/// written back in-process.
pub fn migrate_owned_item<R: CommandRunner>(
    cli: &SecurityCli<R>,
    legacy: &dyn LegacyKeychain,
    service: &str,
    account: &str,
) -> MigrationOutcome {
    match cli.attributes(service, account) {
        Ok(None) => return MigrationOutcome::Missing,
        Ok(Some(attributes)) if is_owned_item(&attributes) => {
            return MigrationOutcome::AlreadyCurrent
        }
        Ok(Some(_)) => {}
        Err(error) => {
            return MigrationOutcome::KeptLegacy(format!("attribute lookup failed: {error}"))
        }
    }
    let value = match legacy.read(service, account) {
        Ok(Some(value)) => value,
        Ok(None) => return MigrationOutcome::Missing,
        Err(error) => return MigrationOutcome::KeptLegacy(format!("old item unreadable: {error}")),
    };
    if let Err(error) = legacy.delete(service, account) {
        return MigrationOutcome::KeptLegacy(format!("old item could not be removed: {error}"));
    }
    let created = cli
        .write(service, account, &value, WriteMode::CreateOwned)
        .map_err(WriteError::into_security)
        .and_then(|()| match cli.read(service, Some(account))? {
            Some(copy) if copy.as_slice() == value.as_slice() => Ok(()),
            _ => Err(SecurityError::UnreadableOutput),
        });
    let Err(error) = created else {
        return MigrationOutcome::Migrated;
    };
    let _ = cli.delete(service, account);
    match legacy.write(service, account, &value) {
        Ok(()) => MigrationOutcome::KeptLegacy(format!("new item could not be created: {error}")),
        Err(restore) => MigrationOutcome::Lost(format!(
            "new item could not be created ({error}) and the old item could not be restored ({restore})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        ffi::OsString,
        io,
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use zeroize::Zeroizing;

    use super::*;

    #[derive(Default)]
    struct Recorded {
        calls: Vec<(Vec<String>, Option<String>)>,
    }

    /// Scripted runner: answers each call with the next queued response.
    #[derive(Default)]
    struct FakeRunner {
        responses: Mutex<Vec<(i32, &'static str, &'static str)>>,
        recorded: Mutex<Recorded>,
    }

    impl FakeRunner {
        fn with(responses: &[(i32, &'static str, &'static str)]) -> Self {
            let mut queued = responses.to_vec();
            queued.reverse();
            Self {
                responses: Mutex::new(queued),
                recorded: Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<(Vec<String>, Option<String>)> {
            self.recorded.lock().unwrap().calls.clone()
        }
    }

    impl CommandRunner for &FakeRunner {
        fn run(
            &self,
            program: &Path,
            arguments: &[OsString],
            stdin: Option<&[u8]>,
        ) -> io::Result<CommandOutput> {
            assert_eq!(program, Path::new(SECURITY_PROGRAM));
            self.recorded.lock().unwrap().calls.push((
                arguments
                    .iter()
                    .map(|value| value.to_string_lossy().into_owned())
                    .collect(),
                stdin.map(|value| String::from_utf8(value.to_vec()).unwrap()),
            ));
            let (status, stdout, stderr) = self
                .responses
                .lock()
                .unwrap()
                .pop()
                .expect("unexpected security call");
            Ok(CommandOutput {
                status: Some(status),
                stdout: Zeroizing::new(stdout.as_bytes().to_vec()),
                stderr: Zeroizing::new(stderr.as_bytes().to_vec()),
            })
        }
    }

    fn cli(runner: &FakeRunner) -> SecurityCli<&FakeRunner> {
        SecurityCli::new(PathBuf::from(SECURITY_PROGRAM), None, runner)
    }

    #[test]
    fn read_passes_service_and_account_as_arguments_and_parses_stderr() {
        let runner = FakeRunner::with(&[(0, "keychain: \"login\"\n", "password: \"sk-test\"\n")]);
        let value = cli(&runner)
            .read("Claude Code-credentials", Some("me"))
            .unwrap();
        assert_eq!(
            value.as_deref().map(Vec::as_slice),
            Some(b"sk-test".as_slice())
        );
        assert_eq!(
            runner.calls(),
            vec![(
                vec![
                    "find-generic-password".into(),
                    "-s".into(),
                    "Claude Code-credentials".into(),
                    "-a".into(),
                    "me".into(),
                    "-g".into()
                ],
                None
            )]
        );
    }

    #[test]
    fn service_only_read_omits_the_account_and_keychain_is_appended() {
        let runner = FakeRunner::with(&[(0, "", "password: \"t\"\n")]);
        let cli = SecurityCli::new(
            PathBuf::from(SECURITY_PROGRAM),
            Some(PathBuf::from("/tmp/test.keychain-db")),
            &runner,
        );
        cli.read("gh:github.com", None).unwrap();
        assert_eq!(
            runner.calls()[0].0,
            vec![
                "find-generic-password",
                "-s",
                "gh:github.com",
                "-g",
                "/tmp/test.keychain-db"
            ]
        );
    }

    #[test]
    fn missing_items_read_as_none_and_other_failures_are_errors() {
        let runner = FakeRunner::with(&[
            (44, "", "security: SecKeychainSearchCopyNext: not found\n"),
            (51, "", ""),
            (0, "", "no password line\n"),
        ]);
        let cli = cli(&runner);
        assert!(cli.read("s", Some("a")).unwrap().is_none());
        assert_eq!(
            cli.read("s", Some("a")).unwrap_err(),
            SecurityError::Failed(Some(51))
        );
        assert_eq!(
            cli.read("s", Some("a")).unwrap_err(),
            SecurityError::UnreadableOutput
        );
    }

    #[test]
    fn parses_quoted_hex_and_empty_password_lines() {
        let parse = |text: &str| parse_password_output(text.as_bytes()).map(|value| value.to_vec());
        assert_eq!(parse("password: \"abcd\"\n"), Some(b"abcd".to_vec()));
        // Printable text keeps inner quotes verbatim; only the outer pair is removed.
        assert_eq!(
            parse("password: \"new\"val\"\n"),
            Some(b"new\"val".to_vec())
        );
        assert_eq!(
            parse("password: 0x6C696E65310A6C696E6532  \"line1\\012line2\"\n"),
            Some(b"line1\nline2".to_vec())
        );
        assert_eq!(
            parse("password: 0xC3A9  \"\\303\\251\"\n"),
            Some("é".as_bytes().to_vec())
        );
        assert_eq!(
            parse("password: 0x7B0A7D0A  \"{\\012}\\012\"\n"),
            Some(b"{\n}\n".to_vec())
        );
        assert_eq!(parse("password: \n"), Some(Vec::new()));
        assert_eq!(parse("password: 0xABC  \"?\"\n"), None);
        assert_eq!(parse("keychain: \"x\"\n"), None);
    }

    #[test]
    fn writes_send_hex_over_stdin_and_never_as_arguments() {
        let runner = FakeRunner::with(&[(0, "", "")]);
        cli(&runner)
            .write("svc \"x\"", "a\\b", b"s3cret\n", WriteMode::CreateOwned)
            .unwrap();
        let calls = runner.calls();
        assert_eq!(calls[0].0, vec!["-i"]);
        let line = calls[0].1.as_deref().unwrap();
        assert_eq!(
            line,
            "add-generic-password -s \"svc \\\"x\\\"\" -a \"a\\\\b\" -j \"openquota:security-cli\" -T \"/usr/bin/security\" -X 7333637265740a\n"
        );
        assert!(!line.contains("s3cret"));
    }

    #[test]
    fn updates_never_touch_the_access_list() {
        let runner = FakeRunner::with(&[(0, "", "")]);
        let cli = SecurityCli::new(
            PathBuf::from(SECURITY_PROGRAM),
            Some(PathBuf::from("/tmp/k.keychain-db")),
            &runner,
        );
        cli.write("s", "", b"v", WriteMode::UpdateExisting).unwrap();
        assert_eq!(
            runner.calls()[0].1.as_deref(),
            Some("add-generic-password -U -s \"s\" -a \"\" -w \"v\" \"/tmp/k.keychain-db\"\n")
        );
    }

    #[test]
    fn printable_values_are_quoted_and_escaped() {
        let runner = FakeRunner::with(&[(0, "", "")]);
        cli(&runner)
            .write("s", "a", br#"{"k":"q\"x"}"#, WriteMode::UpdateExisting)
            .unwrap();
        assert_eq!(
            runner.calls()[0].1.as_deref(),
            Some(
                r#"add-generic-password -U -s "s" -a "a" -w "{\"k\":\"q\\\"x\"}"
"#
            )
        );
    }

    #[test]
    fn values_that_do_not_fit_one_interactive_line_are_refused() {
        let runner = FakeRunner::with(&[(0, "", "")]);
        let cli = cli(&runner);
        let fits = vec![b'x'; MAX_INTERACTIVE_LINE - 100];
        cli.write("s", "a", &fits, WriteMode::UpdateExisting)
            .unwrap();
        let binary = vec![0_u8; MAX_INTERACTIVE_LINE / 2];
        assert_eq!(
            cli.write("s", "a", &binary, WriteMode::UpdateExisting),
            Err(WriteError::TooLong)
        );
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn owned_writes_fall_back_to_updating_an_existing_item() {
        let runner = FakeRunner::with(&[(45, "", "already exists"), (0, "", "")]);
        cli(&runner).write_owned("s", "a", b"v").unwrap();
        let calls = runner.calls();
        assert!(calls[0].1.as_deref().unwrap().contains(" -T "));
        assert!(calls[1]
            .1
            .as_deref()
            .unwrap()
            .starts_with("add-generic-password -U "));
        assert!(!calls[1].1.as_deref().unwrap().contains(" -T "));
    }

    #[test]
    fn delete_treats_a_missing_item_as_success() {
        let runner = FakeRunner::with(&[(44, "", ""), (1, "", "")]);
        let cli = cli(&runner);
        cli.delete("s", "a").unwrap();
        assert!(cli.delete("s", "a").is_err());
        assert_eq!(
            runner.calls()[0].0,
            vec!["delete-generic-password", "-s", "s", "-a", "a"]
        );
    }

    #[test]
    fn recognises_the_owned_item_marker() {
        assert!(is_owned_item(
            "attributes:\n    \"icmt\"<blob>=\"openquota:security-cli\"\n"
        ));
        assert!(!is_owned_item("attributes:\n    \"icmt\"<blob>=<NULL>\n"));
    }

    #[derive(Default)]
    struct MemoryLegacy {
        items: Mutex<HashMap<(String, String), Vec<u8>>>,
        fail_read: bool,
        fail_delete: bool,
        fail_write: bool,
    }

    impl MemoryLegacy {
        fn holding(value: &[u8]) -> Self {
            let legacy = Self::default();
            legacy
                .items
                .lock()
                .unwrap()
                .insert(("svc".into(), "kimi".into()), value.to_vec());
            legacy
        }

        fn value(&self) -> Option<Vec<u8>> {
            self.items
                .lock()
                .unwrap()
                .get(&("svc".into(), "kimi".into()))
                .cloned()
        }
    }

    impl LegacyKeychain for MemoryLegacy {
        fn read(&self, service: &str, account: &str) -> Result<Option<Secret>, String> {
            if self.fail_read {
                return Err("denied".into());
            }
            Ok(self
                .items
                .lock()
                .unwrap()
                .get(&(service.into(), account.into()))
                .cloned()
                .map(Zeroizing::new))
        }

        fn write(&self, service: &str, account: &str, value: &[u8]) -> Result<(), String> {
            if self.fail_write {
                return Err("denied".into());
            }
            self.items
                .lock()
                .unwrap()
                .insert((service.into(), account.into()), value.to_vec());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), String> {
            if self.fail_delete {
                return Err("denied".into());
            }
            self.items
                .lock()
                .unwrap()
                .remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    const LEGACY_ATTRIBUTES: &str = "attributes:\n    \"icmt\"<blob>=<NULL>\n";

    #[test]
    fn migration_skips_missing_and_current_items() {
        let runner = FakeRunner::with(&[
            (44, "", ""),
            (0, "    \"icmt\"<blob>=\"openquota:security-cli\"\n", ""),
        ]);
        let legacy = MemoryLegacy::default();
        let cli = cli(&runner);
        assert_eq!(
            migrate_owned_item(&cli, &legacy, "svc", "kimi"),
            MigrationOutcome::Missing
        );
        assert_eq!(
            migrate_owned_item(&cli, &legacy, "svc", "kimi"),
            MigrationOutcome::AlreadyCurrent
        );
        assert_eq!(
            runner.calls()[0].0,
            vec!["find-generic-password", "-s", "svc", "-a", "kimi"]
        );
    }

    #[test]
    fn migration_recreates_a_legacy_item_through_security() {
        let runner = FakeRunner::with(&[
            (0, LEGACY_ATTRIBUTES, ""),
            (0, "", ""),
            (0, "", "password: \"sk-kimi\"\n"),
        ]);
        let legacy = MemoryLegacy::holding(b"sk-kimi");
        assert_eq!(
            migrate_owned_item(&cli(&runner), &legacy, "svc", "kimi"),
            MigrationOutcome::Migrated
        );
        assert!(legacy.value().is_none());
        let calls = runner.calls();
        assert_eq!(
            calls[1].1.as_deref(),
            Some("add-generic-password -s \"svc\" -a \"kimi\" -j \"openquota:security-cli\" -T \"/usr/bin/security\" -w \"sk-kimi\"\n")
        );
    }

    #[test]
    fn migration_leaves_the_legacy_item_when_it_cannot_be_read_or_removed() {
        let runner = FakeRunner::with(&[(0, LEGACY_ATTRIBUTES, ""), (0, LEGACY_ATTRIBUTES, "")]);
        let cli = cli(&runner);
        let unreadable = MemoryLegacy {
            fail_read: true,
            ..MemoryLegacy::holding(b"sk-kimi")
        };
        assert!(matches!(
            migrate_owned_item(&cli, &unreadable, "svc", "kimi"),
            MigrationOutcome::KeptLegacy(_)
        ));
        assert_eq!(unreadable.value().as_deref(), Some(b"sk-kimi".as_slice()));
        let undeletable = MemoryLegacy {
            fail_delete: true,
            ..MemoryLegacy::holding(b"sk-kimi")
        };
        assert!(matches!(
            migrate_owned_item(&cli, &undeletable, "svc", "kimi"),
            MigrationOutcome::KeptLegacy(_)
        ));
        assert_eq!(undeletable.value().as_deref(), Some(b"sk-kimi".as_slice()));
        assert_eq!(runner.calls().len(), 2);
    }

    #[test]
    fn migration_restores_the_legacy_item_when_creation_fails() {
        let runner = FakeRunner::with(&[(0, LEGACY_ATTRIBUTES, ""), (1, "", ""), (44, "", "")]);
        let legacy = MemoryLegacy::holding(b"sk-kimi");
        assert!(matches!(
            migrate_owned_item(&cli(&runner), &legacy, "svc", "kimi"),
            MigrationOutcome::KeptLegacy(_)
        ));
        assert_eq!(legacy.value().as_deref(), Some(b"sk-kimi".as_slice()));
    }

    #[test]
    fn migration_restores_the_legacy_item_when_the_copy_does_not_match() {
        let runner = FakeRunner::with(&[
            (0, LEGACY_ATTRIBUTES, ""),
            (0, "", ""),
            (0, "", "password: \"other\"\n"),
            (0, "", ""),
        ]);
        let legacy = MemoryLegacy::holding(b"sk-kimi");
        assert!(matches!(
            migrate_owned_item(&cli(&runner), &legacy, "svc", "kimi"),
            MigrationOutcome::KeptLegacy(_)
        ));
        assert_eq!(legacy.value().as_deref(), Some(b"sk-kimi".as_slice()));
        assert_eq!(
            runner.calls()[3].0,
            vec!["delete-generic-password", "-s", "svc", "-a", "kimi"]
        );
    }

    #[test]
    fn migration_reports_a_lost_item_when_restoring_fails() {
        let runner = FakeRunner::with(&[(0, LEGACY_ATTRIBUTES, ""), (1, "", ""), (44, "", "")]);
        let legacy = MemoryLegacy {
            fail_write: true,
            ..MemoryLegacy::holding(b"sk-kimi")
        };
        assert!(matches!(
            migrate_owned_item(&cli(&runner), &legacy, "svc", "kimi"),
            MigrationOutcome::Lost(_)
        ));
    }
}
