#[cfg(target_os = "macos")]
const MACOS_ITEM_NOT_FOUND: i32 = -25_300;

#[cfg(target_os = "macos")]
pub fn generic_password_service_exists(
    service: &str,
    _timeout: std::time::Duration,
) -> Option<bool> {
    use security_framework::item::{ItemClass, ItemSearchOptions};

    match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .load_attributes(true)
        .skip_authenticated_items(true)
        .search()
    {
        Ok(_) => Some(true),
        Err(error) if error.code() == MACOS_ITEM_NOT_FOUND => Some(false),
        Err(_) => None,
    }
}

/// Reads another tool's item through `/usr/bin/security`, which those tools already
/// trust, so an OpenQuota update does not trigger a new Keychain prompt.
#[cfg(target_os = "macos")]
pub fn read_generic_password(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
    macos::cli()
        .read(service, Some(account))
        .map(|value| value.map(|value| value.to_vec()))
        .map_err(|error| {
            crate::app_warn!("keychain", "generic password read failed: {error}");
            "The macOS Keychain could not be read.".into()
        })
}

/// Reads the first item of `service`, whatever its account.
#[cfg(target_os = "macos")]
pub fn read_generic_password_for_service(service: &str) -> Result<Option<Vec<u8>>, String> {
    macos::cli()
        .read(service, None)
        .map(|value| value.map(|value| value.to_vec()))
        .map_err(|_| "The macOS Keychain could not be read.".into())
}

#[cfg(target_os = "macos")]
pub fn read_owned_password(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
    macos::read_owned(service, account)
}

/// Replaces the data of another tool's item without touching its access list or
/// partition list, the same way those tools update it themselves.
#[cfg(target_os = "macos")]
pub fn write_generic_password(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
    use super::security_cli::{WriteError, WriteMode};

    match macos::cli().write(service, account, value, WriteMode::UpdateExisting) {
        Ok(()) => Ok(()),
        Err(WriteError::TooLong) => {
            // Moving the value into the argument list would expose it in `ps`. Fall back
            // to the in-process update earlier builds used; it leaves the access list
            // alone but may ask for the Keychain password once per build.
            crate::app_info!(
                "keychain",
                "value too long for /usr/bin/security; updating in-process"
            );
            security_framework::passwords::set_generic_password(service, account, value)
                .map_err(|_| "The macOS Keychain could not be updated.".into())
        }
        Err(error) => {
            crate::app_warn!("keychain", "generic password update failed: {error:?}");
            Err("The macOS Keychain could not be updated.".into())
        }
    }
}

/// Moves OpenQuota's own items created in-process by earlier builds to items created
/// through `/usr/bin/security`. Items that are already current are left alone.
#[cfg(target_os = "macos")]
pub fn migrate_owned_passwords(service: &str, accounts: &[&str]) {
    for account in accounts {
        macos::with_owned_item(service, account, |_| ());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn migrate_owned_passwords(_service: &str, _accounts: &[&str]) {}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
    };

    use super::super::security_cli::{
        migrate_owned_item, LegacyKeychain, MigrationOutcome, ProcessRunner, Secret, SecurityCli,
        WriteMode,
    };

    const MACOS_ITEM_NOT_FOUND: i32 = super::MACOS_ITEM_NOT_FOUND;

    pub(super) fn cli() -> &'static SecurityCli<ProcessRunner> {
        static CLI: OnceLock<SecurityCli<ProcessRunner>> = OnceLock::new();
        CLI.get_or_init(SecurityCli::system)
    }

    /// Where one of OpenQuota's own items lives after this process looked at it.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum OwnedItem {
        /// Missing, or created through `/usr/bin/security`.
        Current,
        /// Created in-process by an earlier build and not migrated yet.
        Legacy,
    }

    /// Runs `action` while holding the lock that serialises access to OpenQuota's own
    /// items, migrating the item first if this process has not looked at it yet.
    pub(super) fn with_owned_item<T>(
        service: &str,
        account: &str,
        action: impl FnOnce(&mut OwnedItem) -> T,
    ) -> T {
        static STATES: OnceLock<Mutex<HashMap<(String, String), OwnedItem>>> = OnceLock::new();
        let mut states = STATES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = states
            .entry((service.to_owned(), account.to_owned()))
            .or_insert_with(|| migrate(service, account));
        action(state)
    }

    fn migrate(service: &str, account: &str) -> OwnedItem {
        match migrate_owned_item(cli(), &InProcessKeychain, service, account) {
            MigrationOutcome::Missing | MigrationOutcome::AlreadyCurrent => OwnedItem::Current,
            MigrationOutcome::Migrated => {
                crate::app_info!(
                    "keychain",
                    "moved saved {account} key to an item /usr/bin/security manages"
                );
                OwnedItem::Current
            }
            MigrationOutcome::KeptLegacy(reason) => {
                crate::app_warn!(
                    "keychain",
                    "saved {account} key left in place; migration failed: {reason}"
                );
                OwnedItem::Legacy
            }
            MigrationOutcome::Lost(reason) => {
                crate::app_error!(
                    "keychain",
                    "saved {account} key was removed during migration: {reason}"
                );
                OwnedItem::Current
            }
        }
    }

    pub(super) fn read_owned(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        with_owned_item(service, account, |state| {
            let value = match state {
                OwnedItem::Current => cli()
                    .read(service, Some(account))
                    .map_err(|error| error.to_string()),
                OwnedItem::Legacy => InProcessKeychain.read(service, account),
            };
            value
                .map(|value| value.map(|value| value.to_vec()))
                .map_err(|_| "The macOS Keychain could not be read.".into())
        })
    }

    pub(super) fn write_owned(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        with_owned_item(service, account, |state| {
            if *state == OwnedItem::Legacy {
                InProcessKeychain.delete(service, account)?;
                if cli()
                    .write(service, account, value, WriteMode::CreateOwned)
                    .is_err()
                {
                    return InProcessKeychain.write(service, account, value);
                }
                *state = OwnedItem::Current;
                return Ok(());
            }
            cli()
                .write_owned(service, account, value)
                .map_err(|_| "The macOS Keychain could not be updated.".into())
        })
    }

    pub(super) fn delete_owned(service: &str, account: &str) -> Result<(), String> {
        with_owned_item(service, account, |state| {
            match state {
                OwnedItem::Current => cli()
                    .delete(service, account)
                    .map_err(|_| "The macOS Keychain item could not be removed.".to_owned()),
                OwnedItem::Legacy => InProcessKeychain.delete(service, account),
            }?;
            *state = OwnedItem::Current;
            Ok(())
        })
    }

    /// The in-process Keychain API that builds before the switch used for their own
    /// items. Only migration and its fallbacks use it.
    struct InProcessKeychain;

    impl LegacyKeychain for InProcessKeychain {
        fn read(&self, service: &str, account: &str) -> Result<Option<Secret>, String> {
            use security_framework::passwords::{generic_password, PasswordOptions};

            match generic_password(PasswordOptions::new_generic_password(service, account)) {
                Ok(value) => Ok(Some(Secret::new(value))),
                Err(error) if error.code() == MACOS_ITEM_NOT_FOUND => Ok(None),
                Err(_) => Err("The macOS Keychain could not be read.".into()),
            }
        }

        fn write(&self, service: &str, account: &str, value: &[u8]) -> Result<(), String> {
            security_framework::passwords::set_generic_password(service, account, value)
                .map_err(|_| "The macOS Keychain could not be updated.".into())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), String> {
            use security_framework::passwords::delete_generic_password;

            match delete_generic_password(service, account) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == MACOS_ITEM_NOT_FOUND => Ok(()),
                Err(_) => Err("The macOS Keychain item could not be removed.".into()),
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub fn read_generic_password(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
    use std::{ptr, slice};
    use windows_sys::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target = format!("{service}:{account}");
    let wide = target.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    let found = unsafe { CredReadW(wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if found == 0 {
        let code = std::io::Error::last_os_error().raw_os_error();
        return if code == Some(1168) {
            Ok(None)
        } else {
            Err("Windows Credential Manager could not be read.".into())
        };
    }
    if credential.is_null() {
        return Ok(None);
    }
    let value = unsafe {
        let credential_ref = &*credential;
        let bytes = slice::from_raw_parts(
            credential_ref.CredentialBlob,
            credential_ref.CredentialBlobSize as usize,
        )
        .to_vec();
        CredFree(credential.cast());
        bytes
    };
    Ok(Some(value))
}

#[cfg(target_os = "windows")]
pub fn generic_password_service_exists(
    service: &str,
    _timeout: std::time::Duration,
) -> Option<bool> {
    use std::{ptr, slice};
    use windows_sys::Win32::Security::Credentials::{CredEnumerateW, CredFree, CREDENTIALW};

    let filter = format!("{service}:*")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut count = 0_u32;
    let mut credentials: *mut *mut CREDENTIALW = ptr::null_mut();
    let found = unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut credentials) };
    if found == 0 {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(1168) => Some(false),
            _ => None,
        };
    }
    let exists = !credentials.is_null()
        && unsafe { slice::from_raw_parts(credentials, count as usize) }
            .iter()
            .any(|credential| !credential.is_null());
    if !credentials.is_null() {
        unsafe { CredFree(credentials.cast()) };
    }
    Some(exists)
}

#[cfg(target_os = "windows")]
pub fn read_owned_password(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
    read_generic_password(service, account)
}

#[cfg(target_os = "windows")]
pub fn write_generic_password(_service: &str, _account: &str, _value: &[u8]) -> Result<(), String> {
    Err("OpenQuota does not overwrite credentials owned by another Windows application.".into())
}

#[cfg(target_os = "windows")]
pub fn write_owned_password(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
    use std::ptr;
    use windows_sys::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    let target = format!("{service}:{account}")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let username = account.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let comment = format!("OpenQuota {account} API key")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut blob = zeroize::Zeroizing::new(value.to_vec());
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_ptr().cast_mut(),
        Comment: comment.as_ptr().cast_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: u32::try_from(blob.len())
            .map_err(|_| "The API key is too large for Windows Credential Manager.")?,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: ptr::null_mut(),
        TargetAlias: ptr::null_mut(),
        UserName: username.as_ptr().cast_mut(),
    };
    let written = unsafe { CredWriteW(&credential, 0) };
    if written == 0 {
        Err("Windows Credential Manager could not be updated.".into())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub fn write_owned_password(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
    macos::write_owned(service, account, value)
}

#[cfg(target_os = "windows")]
pub fn delete_generic_password(service: &str, account: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

    let target = format!("{service}:{account}")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let deleted = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if deleted != 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(1168) => Ok(()),
        _ => Err("Windows Credential Manager item could not be removed.".into()),
    }
}

#[cfg(target_os = "windows")]
pub fn delete_owned_password(service: &str, account: &str) -> Result<(), String> {
    delete_generic_password(service, account)
}

#[cfg(target_os = "macos")]
pub fn delete_owned_password(service: &str, account: &str) -> Result<(), String> {
    macos::delete_owned(service, account)
}

#[cfg(target_os = "linux")]
pub fn read_generic_password(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
    use std::collections::HashMap;

    use secret_service::{blocking::SecretService, EncryptionType};

    let secret_service = SecretService::connect(EncryptionType::Dh)
        .map_err(|_| linux_secret_service_unavailable())?;
    let mut matches = secret_service
        .search_items(HashMap::from([("service", service), ("username", account)]))
        .map_err(|_| "Linux Secret Service could not be searched.")?;
    if let Some(item) = matches.unlocked.pop() {
        return item
            .get_secret()
            .map(Some)
            .map_err(|_| "Linux Secret Service item could not be read.".into());
    }
    let Some(item) = matches.locked.pop() else {
        return Ok(None);
    };
    item.unlock()
        .map_err(|_| "Linux Secret Service item could not be unlocked.")?;
    item.get_secret()
        .map(Some)
        .map_err(|_| "Linux Secret Service item could not be read.".into())
}

#[cfg(target_os = "linux")]
pub fn generic_password_service_exists(
    service: &str,
    timeout: std::time::Duration,
) -> Option<bool> {
    use std::collections::HashMap;
    use std::sync::mpsc;

    use secret_service::{blocking::SecretService, EncryptionType};

    if timeout.is_zero() {
        return None;
    }
    let service = service.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (|| {
            let secret_service = SecretService::connect(EncryptionType::Dh).ok()?;
            let matches = secret_service
                .search_items(HashMap::from([("service", service.as_str())]))
                .ok()?;
            Some(!matches.unlocked.is_empty() || !matches.locked.is_empty())
        })();
        let _ = sender.send(result);
    });
    receiver.recv_timeout(timeout).ok().flatten()
}

#[cfg(target_os = "linux")]
pub fn read_owned_password(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
    read_generic_password(service, account)
}

#[cfg(target_os = "linux")]
pub fn write_generic_password(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
    use std::collections::HashMap;

    use secret_service::{blocking::SecretService, EncryptionType};

    let secret_service = SecretService::connect(EncryptionType::Dh)
        .map_err(|_| linux_secret_service_unavailable())?;
    let mut matches = secret_service
        .search_items(HashMap::from([("service", service), ("username", account)]))
        .map_err(|_| "Linux Secret Service could not be searched.")?;
    let item = matches
        .unlocked
        .pop()
        .or_else(|| matches.locked.pop())
        .ok_or("The credential owned by the provider no longer exists.")?;
    item.unlock()
        .map_err(|_| "Linux Secret Service item could not be unlocked.")?;
    item.set_secret(value, "text/plain; charset=utf8")
        .map_err(|_| "Linux Secret Service item could not be updated.".into())
}

#[cfg(target_os = "linux")]
pub fn write_owned_password(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
    use std::collections::HashMap;

    use secret_service::{blocking::SecretService, EncryptionType};

    let secret_service = SecretService::connect(EncryptionType::Dh)
        .map_err(|_| linux_secret_service_unavailable())?;
    let collection = secret_service
        .get_default_collection()
        .or_else(|_| secret_service.create_collection("OpenQuota", "default"))
        .map_err(|_| {
            "The Linux Secret Service has no usable default collection. Start or unlock your keyring and try again."
        })?;
    collection.ensure_unlocked().map_err(|_| {
        "The Linux Secret Service collection is locked. Unlock your keyring and try again."
    })?;
    collection
        .create_item(
            &format!("OpenQuota {account} API Key"),
            HashMap::from([("service", service), ("username", account)]),
            value,
            true,
            "text/plain; charset=utf8",
        )
        .map(|_| ())
        .map_err(|_| "Linux Secret Service could not save the API key.".into())
}

#[cfg(target_os = "linux")]
pub fn delete_generic_password(service: &str, account: &str) -> Result<(), String> {
    use std::collections::HashMap;

    use secret_service::{blocking::SecretService, EncryptionType};

    let secret_service = SecretService::connect(EncryptionType::Dh)
        .map_err(|_| linux_secret_service_unavailable())?;
    let matches = secret_service
        .search_items(HashMap::from([("service", service), ("username", account)]))
        .map_err(|_| "Linux Secret Service could not be searched.")?;
    for item in matches.unlocked {
        item.delete()
            .map_err(|_| "Linux Secret Service item could not be removed.")?;
    }
    for item in matches.locked {
        item.unlock()
            .map_err(|_| "Linux Secret Service item could not be unlocked for removal.")?;
        item.delete()
            .map_err(|_| "Linux Secret Service item could not be removed.")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn delete_owned_password(service: &str, account: &str) -> Result<(), String> {
    delete_generic_password(service, account)
}

#[cfg(target_os = "linux")]
fn linux_secret_service_unavailable() -> String {
    "Linux Secret Service is unavailable. Start a Secret Service-compatible keyring, such as GNOME Keyring or KWallet, and try again."
        .into()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn generic_password_service_exists(
    _service: &str,
    _timeout: std::time::Duration,
) -> Option<bool> {
    Some(false)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn read_generic_password(_service: &str, _account: &str) -> Result<Option<Vec<u8>>, String> {
    Ok(None)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn read_owned_password(_service: &str, _account: &str) -> Result<Option<Vec<u8>>, String> {
    Ok(None)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn write_generic_password(_service: &str, _account: &str, _value: &[u8]) -> Result<(), String> {
    Err("The system credential store is unavailable on this platform.".into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn write_owned_password(_service: &str, _account: &str, _value: &[u8]) -> Result<(), String> {
    Err("The system credential store is unavailable on this platform.".into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn delete_generic_password(_service: &str, _account: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn delete_owned_password(_service: &str, _account: &str) -> Result<(), String> {
    Ok(())
}

pub fn decode_go_keyring_value(value: &[u8]) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let text = std::str::from_utf8(value).ok()?.trim();
    let encoded = text.strip_prefix("go-keyring-base64:")?;
    String::from_utf8(STANDARD.decode(encoded).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD, Engine};

    use super::decode_go_keyring_value;

    #[test]
    fn decodes_go_keyring_wrapped_json() {
        let json = r#"{"access_token":"placeholder"}"#;
        let wrapped = format!("go-keyring-base64:{}", STANDARD.encode(json));
        assert_eq!(
            decode_go_keyring_value(wrapped.as_bytes()).as_deref(),
            Some(json)
        );
        assert!(decode_go_keyring_value(b"plain text").is_none());
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn system_credential_store_round_trip_when_requested() {
        if std::env::var("OPENQUOTA_TEST_CREDENTIAL_STORE").as_deref() != Ok("1") {
            return;
        }

        let service = format!(
            "io.github.deviffyy.openquota.credential-test.{}",
            std::process::id()
        );
        let account = "round-trip";
        let result = (|| -> Result<(), String> {
            super::delete_owned_password(&service, account)?;
            super::write_owned_password(&service, account, b"first-value")?;
            if super::read_owned_password(&service, account)?.as_deref()
                != Some(b"first-value".as_slice())
            {
                return Err("The first credential round-trip value did not match.".into());
            }
            super::write_owned_password(&service, account, b"second-value")?;
            if super::read_owned_password(&service, account)?.as_deref()
                != Some(b"second-value".as_slice())
            {
                return Err("The updated credential round-trip value did not match.".into());
            }
            Ok(())
        })();
        let cleanup = super::delete_owned_password(&service, account);

        result.unwrap();
        cleanup.unwrap();
        assert!(super::read_owned_password(&service, account)
            .unwrap()
            .is_none());
    }
}
