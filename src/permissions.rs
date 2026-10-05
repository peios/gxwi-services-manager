//! Who may do what with a service, for gxwi-sd-editor to show and change.
//!
//! A service has two descriptors, which answer different questions (peinit
//! TRM §4.6). WHO MAY CONTROL IT is the `ServiceSecurity` value of its
//! definition, which peinit checks every command against: start, stop,
//! reload, and seeing its state. WHO MAY CHANGE ITS DEFINITION is the
//! descriptor of the definition's registry key, which the registry checks.
//!
//! Each is read here and handed to the editor, with what its rights are
//! called and whether it can be changed, which is found out by asking for
//! the access that changing it takes, never guessed. What the editor sends
//! back is applied here, by whatever handle was opened for it.

use gxwi_sd_editor::{Can, Children, Generic, Object, Part, Request, Right, splice};
use peinit::client::{SERVICE_GENERIC_MAPPING, SERVICES_ROOT_KEY, ServiceAccess};
use peios::registry::{Key, KeyAccess, OpenFlags, ValueType};
use peios::security::SecurityDescriptor;

use crate::system::{self, From};

const EACCES: i32 = 13;

/// Which of a service's descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    /// `ServiceSecurity`: who may control it.
    Control,
    /// The definition key's own: who may read or change the definition.
    Definition,
    /// The Services key's `ServiceSecurity`, or peinit's built-in default:
    /// who may control a service without its own.
    Every,
}

impl Which {
    pub fn named(name: &str) -> Option<Which> {
        match name {
            "control" => Some(Which::Control),
            "definition" => Some(Which::Definition),
            "every" => Some(Which::Every),
            _ => None,
        }
    }

    /// What it says, for the person: "who may control it".
    pub fn says(self) -> &'static str {
        match self {
            Which::Control => "who may control it",
            Which::Definition => "who may change its definition",
            Which::Every => "who may control a service without its own",
        }
    }
}

/// What applies a descriptor the person asked to apply, and why it could
/// not be.
pub use gxwi_sd_editor::registry::Apply;

/// The descriptor `which` of `service`, as the editor is to be asked to
/// show it, and what applies what it sends back. `service` means nothing
/// to `Every`.
pub fn open(which: Which, service: &str, title: &str) -> Result<(Request, Apply), String> {
    match which {
        Which::Control => control(service, title),
        Which::Definition => definition(service, title),
        Which::Every => every(),
    }
}

/// The request for a `ServiceSecurity` kept on the key at `path`, which
/// `read` says what it is now, and what writes what comes back there.
fn service_security(
    path: &str,
    object: Object,
    cannot: &str,
    read: impl Fn() -> Result<SecurityDescriptor, String> + Send + 'static,
) -> Result<(Request, Apply), String> {
    // The value is the key's, and is changed by changing that.
    let (key, can) = match Key::open(None, path, KeyAccess::QUERY_VALUE | KeyAccess::SET_VALUE, OpenFlags::empty()) {
        Ok(key) => (key, Can { dacl: true, ..Can::default() }),
        Err(e) if e.raw_os_error() == Some(EACCES) => {
            let key = Key::open(None, path, KeyAccess::QUERY_VALUE, OpenFlags::empty()).map_err(|e| unreadable(&e, "where it is kept"))?;
            (key, Can { dacl: false, why: Some(cannot.into()), ..Can::default() })
        }
        Err(e) => return Err(unreadable(&e, "where it is kept")),
    };
    let mapping = SERVICE_GENERIC_MAPPING;
    let request = Request {
        object,
        sd: read()?.as_bytes().to_vec(),
        rights: service_rights(),
        generic: Generic { read: mapping.read, write: mapping.write, execute: mapping.execute, all: mapping.all },
        can,
        ..Request::default()
    };
    let apply = move |sd: &[u8], parts: &[Part]| {
        // What it is now, with what the person changed put in: the rest is
        // not the editor's to write back.
        let value = splice(read()?.as_bytes(), sd, parts)?;
        key.set_value(b"ServiceSecurity", ValueType::BINARY, &value).call().map_err(|e| refused(&e))
    };
    Ok((request, Box::new(apply)))
}

fn every() -> Result<(Request, Apply), String> {
    let object = Object { name: "Every service".into(), kind: "Services without permissions of their own".into(), container: false, children: Children::All, ..Object::default() };
    let cannot = "You may not change the definitions of services, which is where who may control them is kept.";
    service_security(SERVICES_ROOT_KEY, object, cannot, || system::every().map(|(descriptor, _)| descriptor))
}

/// The service's rights, by what they are called, from the most to the
/// least, so that Full control is above what it includes.
fn service_rights() -> Vec<Right> {
    let right = |name: &str, mask: ServiceAccess| Right { name: name.into(), mask: mask.bits(), general: true };
    vec![
        right("Full control", ServiceAccess::ALL),
        right("Start", ServiceAccess::START),
        right("Stop", ServiceAccess::STOP),
        right("Reload", ServiceAccess::INTERROGATE),
        right("See its state", ServiceAccess::QUERY_STATUS),
    ]
}

fn control(service: &str, title: &str) -> Result<(Request, Apply), String> {
    let object = Object { name: title.into(), kind: "Service".into(), container: false, children: Children::All, ..Object::default() };
    let cannot = "You may not change this service's definition, which is where who may control it is kept.";
    let service = service.to_string();
    // Its own, or the one it takes until it has its own: applied, it is its own.
    let path = format!("{SERVICES_ROOT_KEY}\\{service}");
    service_security(&path, object, cannot, move || system::security(&service).map(|(descriptor, _)| descriptor))
}

/// Who may change its definition is the descriptor of its definition's key,
/// which the editor is opened on as any key is.
fn definition(service: &str, title: &str) -> Result<(Request, Apply), String> {
    let path = format!("{SERVICES_ROOT_KEY}\\{service}");
    gxwi_sd_editor::registry::key(&path, &format!("{title} definition"), "You may not change who may read or change this service's definition.")
}

/// Where who may control a service comes from, in words: "set for it".
pub fn from_words(from: From) -> &'static str {
    match from {
        From::Own => "Who may control it is set for this service.",
        From::AllServices => "Who may control it is the default set for every service, until it is set for this one.",
        From::BuiltIn => "Who may control it is the service manager's built-in default, until it is set for this service.",
    }
}

/// Where who may control a service without its own comes from, in words.
pub fn every_words(from: From) -> &'static str {
    match from {
        From::BuiltIn => "A service without permissions of its own takes the service manager's built-in default.",
        _ => "A service without permissions of its own takes the default set for every service.",
    }
}

fn unreadable(e: &peios::Error, what: &str) -> String {
    if e.raw_os_error() == Some(EACCES) { format!("you may not read {what}") } else { format!("{what} could not be read ({e})") }
}

/// Why something could not be applied, for the editor to show.
fn refused(e: &peios::Error) -> String {
    if e.raw_os_error() == Some(EACCES) { "you are not allowed to".into() } else { e.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_rights_are_named_from_the_most() {
        let rights = service_rights();
        assert_eq!(rights[0].mask, SERVICE_GENERIC_MAPPING.all);
        let union = rights[1..].iter().fold(0, |all, right| all | right.mask);
        assert_eq!(union, rights[0].mask, "everything Full control is, the others say between them");
        assert!(rights.iter().all(|right| right.general));
    }
}
