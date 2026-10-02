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

use gxwi_sd_editor::{Can, Children, Generic, Object, Part, Request, Right};
use peinit::client::{SERVICE_GENERIC_MAPPING, SERVICES_ROOT_KEY, ServiceAccess};
use peios::registry::{Key, KeyAccess, OpenFlags, SecInfo, ValueType};
use peios::security::{Control, SdBuilder, SdView, SecurityDescriptor};

use crate::system::{self, From};

const EACCES: i32 = 13;

/// Which of a service's descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    /// `ServiceSecurity`: who may control it.
    Control,
    /// The definition key's own: who may read or change the definition.
    Definition,
}

impl Which {
    pub fn named(name: &str) -> Option<Which> {
        match name {
            "control" => Some(Which::Control),
            "definition" => Some(Which::Definition),
            _ => None,
        }
    }

    /// What it says, for the person: "who may control it".
    pub fn says(self) -> &'static str {
        match self {
            Which::Control => "who may control it",
            Which::Definition => "who may change its definition",
        }
    }
}

/// What applies a descriptor the person asked to apply, and why it could
/// not be.
pub type Apply = Box<dyn FnMut(&[u8], &[Part]) -> Result<(), String> + Send>;

/// The descriptor `which` of `service`, as the editor is to be asked to
/// show it, and what applies what it sends back.
pub fn open(which: Which, service: &str, title: &str) -> Result<(Request, Apply), String> {
    match which {
        Which::Control => control(service, title),
        Which::Definition => definition(service, title),
    }
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
    let path = format!("{SERVICES_ROOT_KEY}\\{service}");
    // The value is the definition's, and is changed by changing that.
    let (key, can) = match Key::open(None, &path, KeyAccess::QUERY_VALUE | KeyAccess::SET_VALUE, OpenFlags::empty()) {
        Ok(key) => (key, Can { dacl: true, ..Can::default() }),
        Err(e) if e.raw_os_error() == Some(EACCES) => {
            let key = Key::open(None, &path, KeyAccess::QUERY_VALUE, OpenFlags::empty()).map_err(|e| unreadable(&e, "its definition"))?;
            let why = "You may not change this service's definition, which is where who may control it is kept.";
            (key, Can { dacl: false, why: Some(why.into()), ..Can::default() })
        }
        Err(e) => return Err(unreadable(&e, "its definition")),
    };
    let (descriptor, _) = system::security(service)?;
    let mapping = SERVICE_GENERIC_MAPPING;
    let request = Request {
        object: Object { name: title.into(), kind: "Service".into(), container: false, children: Children::All },
        sd: descriptor.as_bytes().to_vec(),
        rights: service_rights(),
        generic: Generic { read: mapping.read, write: mapping.write, execute: mapping.execute, all: mapping.all },
        can,
    };
    let service = service.to_string();
    let apply = move |sd: &[u8], parts: &[Part]| {
        // What it is now, its own or the one it takes, with what the person
        // changed put in: the rest is not the editor's to write back.
        let (now, _) = system::security(&service)?;
        let value = splice(now.as_bytes(), sd, parts)?;
        key.set_value(b"ServiceSecurity", ValueType::BINARY, &value).call().map_err(|e| refused(&e))
    };
    Ok((request, Box::new(apply)))
}

fn definition(service: &str, title: &str) -> Result<(Request, Apply), String> {
    let path = format!("{SERVICES_ROOT_KEY}\\{service}");
    // Opened for as much of changing it as the person may do, which is
    // what they are offered.
    let mut opened = None;
    for (dacl, owner) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut access = KeyAccess::READ_CONTROL;
        access.set(KeyAccess::WRITE_DAC, dacl);
        access.set(KeyAccess::WRITE_OWNER, owner);
        match Key::open(None, &path, access, OpenFlags::empty()) {
            Ok(key) => {
                opened = Some((key, dacl, owner));
                break;
            }
            Err(e) if e.raw_os_error() == Some(EACCES) => continue,
            Err(e) => return Err(unreadable(&e, "its definition")),
        }
    }
    let Some((key, dacl, owner)) = opened else { return Err("you may not read who may change its definition".into()) };
    let descriptor = key.get_security(SecInfo::OWNER | SecInfo::GROUP | SecInfo::DACL).map_err(|e| unreadable(&e, "who may change its definition"))?;
    let right = |name: &str, mask: KeyAccess| Right { name: name.into(), mask: mask.bits(), general: true };
    let why = (!dacl).then(|| "You may not change who may read or change this service's definition.".to_string());
    let request = Request {
        object: Object { name: format!("{title} definition"), kind: format!("Registry key {path}"), container: true, children: Children::Containers },
        sd: descriptor.as_bytes().to_vec(),
        rights: vec![right("Full control", KeyAccess::ALL_ACCESS), right("Read", KeyAccess::READ), right("Write", KeyAccess::WRITE)],
        generic: Generic { read: KeyAccess::READ.bits(), write: KeyAccess::WRITE.bits(), execute: 0, all: KeyAccess::ALL_ACCESS.bits() },
        can: Can { dacl, owner, audit: false, why },
    };
    let apply = move |sd: &[u8], parts: &[Part]| {
        let mut secinfo = SecInfo::empty();
        for part in parts {
            secinfo |= match part {
                Part::Owner => SecInfo::OWNER,
                Part::Group => SecInfo::GROUP,
                Part::Dacl => SecInfo::DACL,
                Part::Sacl => SecInfo::SACL,
            };
        }
        let sd = SecurityDescriptor::from_validated_bytes(sd.to_vec()).map_err(|e| format!("it is not a security descriptor ({e})"))?;
        // The registry takes the parts named, and keeps the rest as it is.
        key.set_security(secinfo, &sd, None).map_err(|e| refused(&e))
    };
    Ok((request, Box::new(apply)))
}

/// Where who may control a service comes from, in words: "set for it".
pub fn from_words(from: From) -> &'static str {
    match from {
        From::Own => "Who may control it is set for this service.",
        From::AllServices => "Who may control it is what is set for every service, until it is set for this one.",
        From::BuiltIn => "Who may control it is the service manager's default, until it is set for this service.",
    }
}

/// `edited`'s `parts`, and the rest of `current`: what applying only the
/// parts the person changed comes to, where the descriptor is kept whole.
pub fn splice(current: &[u8], edited: &[u8], parts: &[Part]) -> Result<Vec<u8>, String> {
    let current = SdView::parse(current).map_err(|e| format!("what it is now could not be read ({e})"))?;
    let edited = SdView::parse(edited).map_err(|e| format!("it is not a security descriptor ({e})"))?;
    let from = |part: Part| if parts.contains(&part) { &edited } else { &current };
    let mut sd = SdBuilder::new();
    if let Some(owner) = from(Part::Owner).owner() {
        sd.owner(owner);
    }
    if let Some(group) = from(Part::Group).group() {
        sd.group(group);
    }
    match from(Part::Dacl).dacl() {
        Some(dacl) => sd.dacl(&dacl.to_acl().map_err(|e| format!("its access list could not be made ({e})"))?),
        None => sd.dacl_grant_all(),
    };
    if let Some(sacl) = from(Part::Sacl).sacl() {
        sd.sacl(&sacl.to_acl().map_err(|e| format!("its audit list could not be made ({e})"))?);
    }
    let kept = (from(Part::Dacl).control() & (Control::DACL_PROTECTED | Control::DACL_AUTO_INHERITED))
        | (from(Part::Sacl).control() & (Control::SACL_PROTECTED | Control::SACL_AUTO_INHERITED));
    sd.control(kept, Control::empty());
    let sd = sd.build().map_err(|e| format!("it could not be made ({e})"))?;
    Ok(sd.as_bytes().to_vec())
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
    use peios::security::sddl;

    fn bytes(text: &str) -> Vec<u8> {
        sddl::parse(text).unwrap().as_bytes().to_vec()
    }

    #[test]
    fn only_the_parts_changed_are_put_in() {
        let now = bytes("O:SYG:BAD:P(A;;0xf;;;SY)");
        let edited = bytes("O:BAG:SYD:(A;;0xf;;;SY)(A;;0x1;;;WD)");
        let applied = splice(&now, &edited, &[Part::Dacl]).unwrap();
        let view = SdView::parse(&applied).unwrap();
        assert_eq!(view.owner().unwrap().to_sid().to_string(), "S-1-5-18");
        assert_eq!(view.group().unwrap().to_sid().to_string(), "S-1-5-32-544");
        assert_eq!(view.dacl().unwrap().len(), 2);
        assert!(!view.control().contains(Control::DACL_PROTECTED), "protection goes with the access list it was on");
        assert_eq!(splice(&now, &edited, &[]).unwrap(), splice(&now, &now, &[Part::Dacl, Part::Owner]).unwrap());
        assert!(splice(&now, b"nonsense", &[Part::Dacl]).is_err());
    }

    #[test]
    fn the_service_rights_are_named_from_the_most() {
        let rights = service_rights();
        assert_eq!(rights[0].mask, SERVICE_GENERIC_MAPPING.all);
        let union = rights[1..].iter().fold(0, |all, right| all | right.mask);
        assert_eq!(union, rights[0].mask, "everything Full control is, the others say between them");
        assert!(rights.iter().all(|right| right.general));
    }
}
