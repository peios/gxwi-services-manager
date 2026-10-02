//! Service definitions in the registry: read, written, and deleted, as svctl
//! does it (`svctl definition`), with the values meaning what
//! `peinit::client::Definition` says they mean.
//!
//! A change is written in one transaction, so peinit, which reads every
//! definition again when any changes, sees all of it or none. Each value
//! about to change is read again inside it first, and if it is not what was
//! read before, nothing is written: someone changed it meanwhile.

use peinit::client::{Change, RawRegistryValue, RegistryValueType, SERVICES_ROOT_KEY};
use peios::registry::{CreateFlags, Disposition, Key, KeyAccess, OpenFlags, Transaction, ValueType};

const EACCES: i32 = 13;
const ENOENT: i32 = 2;

fn path(service: &str) -> String {
    format!("{SERVICES_ROOT_KEY}\\{service}")
}

/// Why `what` could not be done, in words.
fn why(error: &peios::Error, what: &str) -> String {
    if error.raw_os_error() == Some(EACCES) { format!("you are not allowed to {what}") } else { format!("it could not {what} ({error})") }
}

fn to_registry(value_type: RegistryValueType) -> ValueType {
    match value_type {
        RegistryValueType::Sz => ValueType::SZ,
        RegistryValueType::MultiSz => ValueType::MULTI_SZ,
        RegistryValueType::Dword => ValueType::DWORD,
        RegistryValueType::Binary => ValueType::BINARY,
        RegistryValueType::Other(other) => ValueType::from_raw(other),
    }
}

fn from_registry(value_type: ValueType) -> RegistryValueType {
    match value_type {
        ValueType::SZ => RegistryValueType::Sz,
        ValueType::MULTI_SZ => RegistryValueType::MultiSz,
        ValueType::DWORD => RegistryValueType::Dword,
        ValueType::BINARY => RegistryValueType::Binary,
        other => RegistryValueType::Other(other.0),
    }
}

/// The values of `service`'s definition, or `None` where it has none.
pub fn read(service: &str) -> Result<Option<Vec<RawRegistryValue>>, String> {
    let key = match Key::open(None, &path(service), KeyAccess::QUERY_VALUE, OpenFlags::empty()) {
        Ok(key) => key,
        Err(error) if error.raw_os_error() == Some(ENOENT) => return Ok(None),
        Err(error) => return Err(why(&error, "read its definition")),
    };
    let records = key.query_values_batch(None).map_err(|error| why(&error, "read its definition"))?;
    Ok(Some(
        records
            .into_iter()
            .map(|record| RawRegistryValue { name: String::from_utf8_lossy(&record.name).into_owned(), value_type: from_registry(record.ty), data: record.data })
            .collect(),
    ))
}

/// Whether the key at `path` opens for `access`, or why not.
fn may(path: &str, access: KeyAccess, what: &str) -> Result<(), String> {
    Key::open(None, path, access, OpenFlags::empty()).map(|_| ()).map_err(|error| why(&error, what))
}

/// Whether `service`'s definition may be changed, or why not.
pub fn changeable(service: &str) -> Result<(), String> {
    may(&path(service), KeyAccess::QUERY_VALUE | KeyAccess::SET_VALUE, "change its definition")
}

/// Whether `service`'s definition may be deleted, or why not.
pub fn deletable(service: &str) -> Result<(), String> {
    may(&path(service), KeyAccess::DELETE, "delete its definition")
}

/// Whether a service may be defined here, or why not.
pub fn creatable() -> Result<(), String> {
    may(SERVICES_ROOT_KEY, KeyAccess::CREATE_SUB_KEY, "define a service")
}

/// Writes `changes` to `service`'s definition, which was read as `found`,
/// making the key first if `create`.
pub fn write(service: &str, found: &[RawRegistryValue], changes: &[Change], create: bool) -> Result<(), String> {
    let txn = Transaction::begin().map_err(|error| why(&error, "change its definition"))?;
    let access = KeyAccess::QUERY_VALUE | KeyAccess::SET_VALUE;
    let key = if create {
        let (key, disposition) =
            Key::create(None, &path(service), access, CreateFlags::empty(), None, Some(&txn)).map_err(|error| why(&error, "define it"))?;
        if disposition == Disposition::OpenedExisting {
            return Err(format!("a service called {service} is defined already"));
        }
        key
    } else {
        match Key::open(None, &path(service), access, OpenFlags::empty()) {
            Ok(key) => key,
            Err(error) if error.raw_os_error() == Some(ENOENT) => return Err("its definition has been deleted".into()),
            Err(error) => return Err(why(&error, "change its definition")),
        }
    };
    for change in changes {
        let name = match change {
            Change::Set(value) => &value.name,
            Change::Unset(name) => name,
        };
        let now = match key.query_value(name.as_bytes(), Some(&txn)) {
            Ok(value) => Some((value.ty, value.data)),
            Err(error) if error.raw_os_error() == Some(ENOENT) => None,
            Err(error) => return Err(why(&error, "read its definition")),
        };
        let then = found.iter().find(|value| value.name.eq_ignore_ascii_case(name)).map(|value| (to_registry(value.value_type), value.data.clone()));
        if now != then {
            return Err(format!("{name} has been changed by someone else since this was read, and nothing was written. Revert reads it again"));
        }
    }
    for change in changes {
        match change {
            Change::Set(value) => key.set_value(value.name.as_bytes(), to_registry(value.value_type), &value.data).in_txn(&txn).call(),
            Change::Unset(name) => key.delete_value(name.as_bytes(), None, Some(&txn)),
        }
        .map_err(|error| why(&error, "change its definition"))?;
    }
    txn.commit().map_err(|error| why(&error, "change its definition"))
}

/// Deletes `service`'s definition and whatever is under its key. Whether
/// another layer of the registry defines it still.
pub fn delete(service: &str) -> Result<bool, String> {
    let txn = Transaction::begin().map_err(|error| why(&error, "delete its definition"))?;
    let key = Key::open(None, &path(service), KeyAccess::DELETE | KeyAccess::ENUMERATE_SUB_KEYS, OpenFlags::empty())
        .map_err(|error| why(&error, "delete its definition"))?;
    delete_tree(&key, &txn)?;
    txn.commit().map_err(|error| why(&error, "delete its definition"))?;
    Ok(Key::open(None, &path(service), KeyAccess::QUERY_VALUE, OpenFlags::empty()).is_ok())
}

fn delete_tree(key: &Key, txn: &Transaction) -> Result<(), String> {
    let names = key
        .subkeys(Some(txn))
        .map(|subkey| subkey.map(|subkey| String::from_utf8_lossy(&subkey.name).into_owned()).map_err(|error| why(&error, "read what is under its definition")))
        .collect::<Result<Vec<_>, _>>()?;
    for name in names {
        let child = Key::open(Some(key), &name, KeyAccess::DELETE | KeyAccess::ENUMERATE_SUB_KEYS, OpenFlags::empty())
            .map_err(|error| why(&error, "delete what is under its definition"))?;
        delete_tree(&child, txn)?;
    }
    key.delete_key(None, Some(txn)).map_err(|error| why(&error, "delete its definition"))
}
