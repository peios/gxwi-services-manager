//! What is asked of the system: peinit, on its control socket, for the
//! services and what they are doing, and the registry for their definitions
//! and who may control them.
//!
//! A connection to peinit is made for each look and each command and then
//! let go: peinit closes one left idle, and a window is mostly idle.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use peinit::client::{
    Accepted, CONTROL_SOCKET_PATH, Command, ControlClient, DEFAULT_SERVICE_SECURITY_SDDL, Failure,
    Operation, SERVICE_GENERIC_MAPPING, SERVICES_ROOT_KEY, Status, Summary,
};
use peios::access::AccessCheck;
use peios::registry::{Key, KeyAccess, OpenFlags, ValueType};
use peios::security::{AccessMask, GenericMapping, SecurityDescriptor, sddl};

const EACCES: i32 = 13;
const ENOENT: i32 = 2;

/// How long a command is followed before the window stops waiting for it.
/// peinit carries on regardless; the window just stops saying so.
const FOLLOWED_FOR: Duration = Duration::from_secs(180);

/// One look at the services.
pub struct Seen {
    /// What peinit lists, or why it could not be asked.
    pub services: Result<Vec<Summary>, Unasked>,
    pub hidden: Hidden,
    /// What this caller may do with each service, listed or hidden, by name.
    pub rights: HashMap<String, Rights>,
}

/// Why peinit was not asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unasked {
    /// Its control socket does not let this caller in: nothing at all is
    /// theirs to ask it.
    NotAllowed,
    /// It could not be reached, or the conversation broke, and why.
    Unreachable(String),
}

/// The services peinit does not list for this caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hidden {
    /// Defined, by the registry, which lets this caller see that much, but
    /// left out of what peinit lists because their state is not this
    /// caller's to see.
    These(Vec<Defined>),
    /// Which services are defined cannot be read, so whether any are left
    /// out cannot be told.
    Unknowable,
}

/// A service as its definition in the registry has it, as far as this
/// caller may read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defined {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

impl Defined {
    /// One known by its name and nothing else.
    pub fn named(name: &str) -> Defined {
        Defined { name: name.into(), display_name: None, description: None }
    }
}

/// What this caller may do with a service, as AccessCheck says against the
/// descriptor peinit checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rights {
    /// The service rights granted.
    Granted(u32),
    /// Not known, and why: the descriptor could not be read.
    Unknown(String),
}

impl Rights {
    /// Whether the rights allow `command`. Unknown rights allow nothing to be
    /// ruled out: the command is offered, and peinit decides.
    pub fn allow(&self, command: Command) -> bool {
        let needed = peinit::client::ServiceAccess::for_command(command).bits();
        match self {
            Rights::Granted(granted) => granted & needed == needed,
            Rights::Unknown(_) => true,
        }
    }
}

pub fn look() -> Seen {
    let services = match ControlClient::connect_default() {
        Ok(mut client) => client.services().map_err(|failure| Unasked::Unreachable(unreachable(&failure))),
        // Whether it was this caller that was refused, the error says by its
        // kind, which the client's message no longer carries.
        Err(_) if UnixStream::connect(CONTROL_SOCKET_PATH).err().is_some_and(|e| e.kind() == ErrorKind::PermissionDenied) => Err(Unasked::NotAllowed),
        Err(e) => Err(Unasked::Unreachable(unreachable(&Failure::from(e)))),
    };
    let hidden = match (&services, defined()) {
        (Ok(listed), Ok(defined)) => Hidden::These(
            defined.into_iter().filter(|defined| !listed.iter().any(|service| service.service.eq_ignore_ascii_case(&defined.name))).collect(),
        ),
        // Nothing is the caller's to ask: every service defined is one whose
        // state is hidden from them.
        (Err(Unasked::NotAllowed), Ok(defined)) => Hidden::These(defined),
        (Err(Unasked::Unreachable(_)), Ok(_)) => Hidden::These(Vec::new()),
        (_, Err(())) => Hidden::Unknowable,
    };
    let listed = services.iter().flatten().map(|service| service.service.clone());
    let unlisted = match &hidden {
        Hidden::These(defined) => defined.iter().map(|defined| defined.name.clone()).collect(),
        Hidden::Unknowable => Vec::new(),
    };
    let rights = listed.chain(unlisted).map(|name| (rights(&name), name)).map(|(rights, name)| (name, rights)).collect();
    Seen { services, hidden, rights }
}

/// What a service is doing, in full.
pub fn status(service: &str) -> Result<Status, Failure> {
    ControlClient::connect_default()?.status_of(service)
}

/// What peinit cannot be reached about, said for a person.
pub fn unreachable(failure: &Failure) -> String {
    match failure {
        Failure::Unreachable(why) => format!("The service manager could not be asked ({why})."),
        Failure::Refused { message, .. } => format!("The service manager would not say: {message}."),
    }
}

/// The services defined in the registry, if which they are can be read, each
/// with what it is called and what it does where those can be read too.
fn defined() -> Result<Vec<Defined>, ()> {
    let key = Key::open(None, SERVICES_ROOT_KEY, KeyAccess::ENUMERATE_SUB_KEYS, OpenFlags::empty()).map_err(|_| ())?;
    let names: Vec<String> = key.subkeys(None).map(|subkey| subkey.map(|subkey| String::from_utf8_lossy(&subkey.name).into_owned()).map_err(|_| ())).collect::<Result<_, _>>()?;
    Ok(names
        .into_iter()
        .map(|name| {
            let definition = Key::open(None, &format!("{SERVICES_ROOT_KEY}\\{name}"), KeyAccess::QUERY_VALUE, OpenFlags::empty()).ok();
            let text = |value: &[u8]| definition.as_ref().and_then(|definition| string(definition, value)).filter(|text| !text.is_empty());
            Defined { display_name: text(b"DisplayName"), description: text(b"Description"), name }
        })
        .collect())
}

/// A string value of a key, if it is one and may be read.
fn string(key: &Key, name: &[u8]) -> Option<String> {
    let value = key.query_value(name, None).ok().filter(|value| value.ty == ValueType::SZ)?;
    let text = value.data.strip_suffix(&[0]).unwrap_or(&value.data);
    String::from_utf8(text.to_vec()).ok()
}

pub fn rights(service: &str) -> Rights {
    let descriptor = match security(service) {
        Ok(descriptor) => descriptor,
        Err(why) => return Rights::Unknown(why),
    };
    let mapping = GenericMapping::new(SERVICE_GENERIC_MAPPING.read, SERVICE_GENERIC_MAPPING.write, SERVICE_GENERIC_MAPPING.execute, SERVICE_GENERIC_MAPPING.all);
    match AccessCheck::new(&descriptor, AccessMask::MAXIMUM_ALLOWED, mapping).check() {
        Ok(decision) => Rights::Granted(decision.granted.bits() & SERVICE_GENERIC_MAPPING.all),
        Err(e) => Rights::Unknown(format!("they could not be checked ({e})")),
    }
}

/// The descriptor peinit checks commands on `service` against (§4.6): its
/// own, or else the Services key's, or else peinit's built-in default.
fn security(service: &str) -> Result<SecurityDescriptor, String> {
    for path in [format!("{SERVICES_ROOT_KEY}\\{service}"), SERVICES_ROOT_KEY.to_string()] {
        let key = match Key::open(None, &path, KeyAccess::QUERY_VALUE, OpenFlags::empty()) {
            Ok(key) => key,
            Err(e) if e.raw_os_error() == Some(EACCES) => return Err("you may not read who may control it".into()),
            Err(e) => return Err(format!("who may control it could not be read ({e})")),
        };
        match key.query_value(b"ServiceSecurity", None) {
            Ok(value) => {
                return SecurityDescriptor::from_validated_bytes(value.data).map_err(|e| format!("who may control it is not readable ({e})"));
            }
            Err(e) if e.raw_os_error() == Some(ENOENT) => continue,
            Err(e) if e.raw_os_error() == Some(EACCES) => return Err("you may not read who may control it".into()),
            Err(e) => return Err(format!("who may control it could not be read ({e})")),
        }
    }
    sddl::parse(DEFAULT_SERVICE_SECURITY_SDDL).map_err(|e| format!("peinit's default could not be read ({e})"))
}

/// How a command came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It did what was asked, and what peinit said it came to.
    Done(Option<String>),
    /// It was answered at once, with nothing to follow: already so, or
    /// nothing to do.
    Answered(Accepted),
    /// It failed, and why.
    Failed(String),
    /// peinit refused it: its code and message.
    Refused { code: String, message: String },
    /// It was still going when the window stopped waiting.
    StillGoing,
}

/// Asks for `command` on `service`, and follows what it started until it is
/// over, telling `going` what it is doing on the way.
pub fn ask(command: Command, service: &str, mut going: impl FnMut(&Operation)) -> Outcome {
    let accepted = match ControlClient::connect_default().map_err(Failure::from).and_then(|mut client| client.command(command, service)) {
        Ok(accepted) => accepted,
        Err(Failure::Refused { code, message }) => return Outcome::Refused { code, message },
        Err(failure) => return Outcome::Failed(unreachable(&failure)),
    };
    let Some(mut id) = accepted.operation_id.clone() else { return Outcome::Answered(accepted) };
    let until = Instant::now() + FOLLOWED_FOR;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
        let operation = match ControlClient::connect_default().map_err(Failure::from).and_then(|mut client| client.operation(&id)) {
            Ok(operation) => operation,
            Err(failure) => return Outcome::Failed(unreachable(&failure)),
        };
        going(&operation);
        if !operation.finished() {
            continue;
        }
        match operation.state {
            peinit::client::OperationState::Completed => return Outcome::Done(operation.result),
            // Joined with one already under way: that one is what to follow.
            peinit::client::OperationState::Merged => match operation.merged_into {
                Some(into) => id = into,
                None => return Outcome::Done(None),
            },
            _ => {
                let why = operation.error.or(operation.result).map(|error| crate::words::failure(&error));
                return Outcome::Failed(why.unwrap_or_else(|| "it did not finish".into()));
            }
        }
    }
    Outcome::StillGoing
}
