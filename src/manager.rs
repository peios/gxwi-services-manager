//! The window: every service defined, what each is doing, and the commands
//! that start, stop, restart, reload and reset them.
//!
//! WHAT IS LISTED is what peinit lists, which is every service the person may
//! query, and with it the services defined in the registry that peinit left
//! out: their definitions are readable to the person, so that they exist is
//! already theirs to know, but their state is not. Where the registry cannot
//! be read either, the window says that some may not be listed at all.
//!
//! WHAT THE PERSON MAY DO is asked of the system, never guessed: AccessCheck
//! against the descriptor peinit itself checks a command against. A command
//! is offered only where the person has its right and the service is in a
//! state the command does something in, by peinit's own admission matrix;
//! otherwise its button is there, unpressable, and the details say why.
//! Where who may control a service cannot be read, the commands are offered
//! and peinit decides, and the details say that too.
//!
//! A COMMAND is asked for and not waited on: the window follows the
//! operation it started and says what came of it, and carries on being a
//! window meanwhile. One command at a time per service.
//!
//! PERMISSIONS, who may control a service and who may change its
//! definition, open in gxwi-sd-editor, a dialog of its own. The window reads
//! the descriptor, says whether the person can change it, and applies what
//! comes back (`permissions`); then it looks again, since what the person
//! may do may have changed with it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Weak};

use gxwi_sd_editor::Part;
use libgxwi::{Facts, Fields, Live, Surface, Value, escape};
use peinit::client::{Admission, Command, Failure, State, Status, Summary, Timer, admission};

use crate::permissions::{self, Which};
use crate::system::{self, Defined, From, Hidden, Outcome, Rights, Seen, Unasked};
use crate::words;

/// What is said where peinit's control socket turns the person away.
const REFUSED: &str = "The service manager does not let you ask it anything, so the state of the services here is not yours to see, and none of them is yours to start or stop.";

/// What shows a service's logs.
const EVENT_VIEWER: &str = "/usr/bin/gxwi-event-viewer";

/// The commands, in the order their buttons are drawn.
const COMMANDS: [Command; 5] = [Command::Start, Command::Stop, Command::Restart, Command::Reload, Command::Reset];

/// What the window last said about a command, or about anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Said {
    text: String,
    bad: bool,
}

/// A service's state in full, or why it is not to be had.
struct Detail {
    service: String,
    status: Result<Status, String>,
}

/// Permissions to take away, so that the default is what applies: a
/// service's own, or the default set for every service.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Forget {
    Service(String),
    Every,
}

/// A row of the listing.
enum Row<'a> {
    Listed(&'a Summary),
    /// Defined, with its state not the person's to see.
    Hidden(&'a Defined),
}

impl Row<'_> {
    fn service(&self) -> &str {
        match self {
            Row::Listed(summary) => &summary.service,
            Row::Hidden(defined) => &defined.name,
        }
    }

    fn display_name(&self) -> Option<&str> {
        match self {
            Row::Listed(summary) => summary.display_name.as_deref(),
            Row::Hidden(defined) => defined.display_name.as_deref(),
        }
    }

    fn description(&self) -> Option<&str> {
        match self {
            Row::Listed(summary) => summary.description.as_deref(),
            Row::Hidden(defined) => defined.description.as_deref(),
        }
    }

    /// What it is called: its display name if it has one.
    fn title(&self) -> &str {
        self.display_name().unwrap_or(self.service())
    }

    fn state(&self) -> Option<State> {
        match self {
            Row::Listed(summary) => Some(summary.state),
            Row::Hidden(_) => None,
        }
    }
}

pub struct Manager {
    services: Vec<Summary>,
    hidden: Hidden,
    rights: HashMap<String, Rights>,
    from: HashMap<String, From>,
    /// Where who may control a service without its own comes from.
    every: Result<From, String>,
    /// Whether that may be changed, or why not.
    every_changeable: Result<(), String>,
    /// Whether the definition of the service picked may be changed, or why
    /// not, as last looked at.
    changeable: Option<(String, Result<(), String>)>,
    /// Whether a service may be defined, or why not.
    creatable: Result<(), String>,
    /// What is being asked of the person before it is done.
    asking: Option<Forget>,
    /// The permissions open in the editor, by service and which.
    editing: HashSet<(String, Which)>,
    /// The services whose definitions are open, each in a window of its
    /// own, by name in lower case.
    defining: HashSet<String>,
    /// Whether peinit has been asked yet.
    looked: bool,
    /// Whether peinit's control socket turned the person away: then nothing
    /// is theirs to ask it, state or command.
    refused: bool,
    /// Why peinit could not be asked, the last time it was.
    trouble: Option<String>,
    picked: Option<String>,
    detail: Option<Detail>,
    /// What is under way, by service.
    working: BTreeMap<String, String>,
    said: Option<Said>,
    /// What a timer's times are said against.
    clock: words::Clock,
    /// The window, for work done aside to say when it is done. Without one,
    /// nothing is asked of the system.
    pub window: Weak<Surface<Manager>>,
}

impl Manager {
    pub fn new() -> Manager {
        Manager {
            services: Vec::new(),
            hidden: Hidden::These(Vec::new()),
            rights: HashMap::new(),
            from: HashMap::new(),
            every: Err("it has not been read yet".into()),
            every_changeable: Err("it has not been read yet".into()),
            changeable: None,
            creatable: Err("it has not been asked yet".into()),
            asking: None,
            editing: HashSet::new(),
            defining: HashSet::new(),
            looked: false,
            refused: false,
            trouble: None,
            picked: None,
            detail: None,
            working: BTreeMap::new(),
            said: None,
            clock: words::Clock::Machine,
            window: Weak::new(),
        }
    }

    /// The service picked, for whoever looks again to ask about, if its state
    /// is the person's to ask for. One whose state is hidden is not asked
    /// about: peinit records every refusal, and a window left open on one
    /// would be refused every time it looked.
    pub fn picked(&self) -> Option<String> {
        self.picked.clone().filter(|picked| self.services.iter().any(|service| &service.service == picked))
    }

    /// What a look at the services found.
    pub fn seen(&mut self, seen: Seen) {
        self.looked = true;
        self.refused = seen.services == Err(Unasked::NotAllowed);
        match seen.services {
            Ok(services) => {
                self.services = services;
                self.trouble = None;
            }
            Err(Unasked::NotAllowed) => {
                self.services = Vec::new();
                self.trouble = Some(REFUSED.into());
            }
            // What was listed stays, under why it is not current.
            Err(Unasked::Unreachable(why)) => self.trouble = Some(why),
        }
        self.hidden = seen.hidden;
        self.rights = seen.rights;
        self.from = seen.from;
        self.every = seen.every;
        self.every_changeable = seen.every_changeable;
        self.creatable = seen.creatable;
        if seen.changeable.is_some() {
            self.changeable = seen.changeable;
        }
    }

    /// The service picked, whatever is known of it: what a look asks the
    /// registry about.
    pub fn chosen(&self) -> Option<String> {
        self.picked.clone()
    }

    /// What `service` is doing in full, or why that is not to be had.
    pub fn detailed(&mut self, service: &str, status: Result<Status, Failure>) {
        if self.picked.as_deref() != Some(service) {
            return;
        }
        // Turned away, the person is told so once, and not again for each
        // service.
        if self.refused {
            self.detail = None;
            return;
        }
        let status = status.map_err(|failure| match &failure {
            Failure::Refused { code, .. } if code == "ACCESS_DENIED" => "Its state is not yours to see.".to_string(),
            Failure::Refused { code, .. } if code == "UNKNOWN_SERVICE" => "The service manager does not know it: its definition may not have been read yet.".to_string(),
            failure => system::unreachable(failure),
        });
        self.detail = Some(Detail { service: service.to_string(), status });
    }

    /// The rows, as the find field narrows them, by title.
    fn rows(&self, find: &str) -> Vec<Row<'_>> {
        let find = find.trim().to_lowercase();
        let hidden: &[Defined] = match &self.hidden {
            Hidden::These(defined) => defined,
            Hidden::Unknowable => &[],
        };
        let mut rows: Vec<Row> = self.services.iter().map(Row::Listed).chain(hidden.iter().map(Row::Hidden)).collect();
        rows.retain(|row| {
            find.is_empty()
                || row.service().to_lowercase().contains(&find)
                || row.title().to_lowercase().contains(&find)
                || row.description().is_some_and(|description| description.to_lowercase().contains(&find))
        });
        rows.sort_by_cached_key(|row| (row.title().to_lowercase(), row.service().to_string()));
        rows
    }

    fn row<'a>(&'a self, service: &str) -> Option<Row<'a>> {
        self.services
            .iter()
            .find(|summary| summary.service == service)
            .map(Row::Listed)
            .or_else(|| match &self.hidden {
                Hidden::These(defined) => defined.iter().find(|defined| defined.name == service).map(Row::Hidden),
                Hidden::Unknowable => None,
            })
    }

    fn rights_of(&self, service: &str) -> Rights {
        self.rights.get(service).cloned().unwrap_or_else(|| Rights::Unknown("they have not been read yet".into()))
    }

    /// Whether `command` is worth offering on `row`, or why not, in words.
    fn offer(&self, command: Command, row: &Row) -> Result<(), String> {
        if self.refused {
            return Err("The service manager does not let you ask it anything.".into());
        }
        if let Some(doing) = self.working.get(row.service()) {
            return Err(format!("{doing} already."));
        }
        if !self.rights_of(row.service()).allow(command) {
            return Err(format!("You are not allowed to {} it.", words::verb(command)));
        }
        // A service whose state is hidden may still be the person's to
        // control: what it would come to is peinit's to say.
        let Some(state) = row.state() else { return Ok(()) };
        let state_words = words::state(state).to_lowercase();
        match admission(command, state) {
            Admission::Acts => Ok(()),
            Admission::Already => Err(format!("It is {state_words} already.")),
            Admission::Nothing => Err(format!("It is {state_words}: there is nothing to {}.", words::verb(command))),
            Admission::Refused => Err(format!("It cannot be {} while it is {state_words}.", words::done(command))),
        }
    }

    /// Picks `service` and asks what it is doing in full.
    fn pick(&mut self, service: String) {
        if self.picked.as_ref() == Some(&service) {
            return;
        }
        self.detail = None;
        self.asking = None;
        self.picked = Some(service.clone());
        // Its state, if it is the person's to ask for, and whether its
        // definition is theirs to change, which is the registry's to say.
        let listed = self.picked().is_some();
        let Some(window) = self.window.upgrade() else { return };
        std::thread::spawn(move || {
            let seen = system::look(Some(&service));
            let status = listed.then(|| system::status(&service));
            window.update(|manager, _| {
                manager.seen(seen);
                if let Some(status) = status {
                    manager.detailed(&service, status);
                }
            });
        });
    }

    /// Asks for `command` on `service`, if it is on offer.
    fn command(&mut self, command: Command, service: &str) {
        let Some(row) = self.row(service) else { return };
        let title = row.title().to_string();
        if let Err(why) = self.offer(command, &row) {
            self.said = Some(Said { text: format!("{title} was not {}: {why}", words::done(command)), bad: true });
            return;
        }
        let doing = format!("{} {title}", words::doing(command));
        self.working.insert(service.to_string(), doing);
        self.said = None;
        let Some(window) = self.window.upgrade() else { return };
        let service = service.to_string();
        std::thread::spawn(move || {
            let going = |operation: &peinit::client::Operation| {
                let now = format!("{} {title} ({})", words::doing(command), if operation.state == peinit::client::OperationState::Pending { "waiting its turn" } else { "under way" });
                window.update(|manager, _| {
                    manager.working.insert(service.clone(), now);
                });
            };
            let outcome = system::ask(command, &service, going);
            window.update(|manager, _| manager.asked(command, &service, outcome));
            let seen = system::look(Some(&service));
            let status = system::status(&service);
            window.update(|manager, _| {
                manager.seen(seen);
                manager.detailed(&service, status);
            });
        });
    }

    /// What came of `command` on `service`.
    fn asked(&mut self, command: Command, service: &str, outcome: Outcome) {
        self.working.remove(service);
        let title = self.row(service).map_or_else(|| service.to_string(), |row| row.title().to_string());
        let (text, bad) = match outcome {
            // A reload nothing confirmed was sent, and may or may not have
            // been done: peinit says which as what it came to.
            Outcome::Done(Some(result)) if command == Command::Reload && result.contains("advisory") => {
                (format!("{title} was asked to reload, and did not say whether it had."), false)
            }
            Outcome::Done(_) => (format!("{title} was {}.", words::done(command)), false),
            Outcome::Answered(accepted) => (format!("{title} is {} now.", words::state(accepted.state).to_lowercase()), false),
            Outcome::Failed(why) => (format!("{title} could not be {}: {why}", words::done(command)), true),
            Outcome::Refused { code, message } => (
                match code.as_str() {
                    "ACCESS_DENIED" => format!("You are not allowed to {} {title}.", words::verb(command)),
                    "INVALID_STATE" => format!("{title} cannot be {} as it is now.", words::done(command)),
                    "UNKNOWN_SERVICE" => format!("{title} is no longer defined."),
                    _ => format!("{title} could not be {}: {message}", words::done(command)),
                },
                true,
            ),
            Outcome::StillGoing => (format!("{title} is still being {}. The service manager carries on with it; this window has stopped waiting.", words::done(command)), false),
        };
        self.said = Some(Said { text, bad });
    }

    /// Looks again now, rather than when the next look is due.
    fn refresh(&mut self) {
        let Some(window) = self.window.upgrade() else { return };
        let picked = self.picked();
        let chosen = self.chosen();
        std::thread::spawn(move || {
            let seen = system::look(chosen.as_deref());
            let status = picked.map(|picked| {
                let status = system::status(&picked);
                (picked, status)
            });
            window.update(|manager, _| {
                manager.seen(seen);
                if let Some((picked, status)) = status {
                    manager.detailed(&picked, status);
                }
            });
        });
    }

    /// Opens `service`'s definition, or with none a new one, in a window of
    /// its own: this program again, which finds the desktop as this one did.
    /// Nothing ends it when this window goes, so what is being changed there
    /// is not lost with it. The desktop starts programs only for the shell.
    fn definition(&mut self, service: Option<&str>) {
        // One window to a definition: two would each save over the other's
        // changes, or be refused for them. A window cannot be brought
        // forward from here, so it is said instead.
        if let Some(service) = service.filter(|service| self.defining(service)) {
            let title = self.row(service).map_or(service.to_string(), |row| row.title().to_string());
            self.said = Some(Said { text: format!("The definition of {title} is open already, in a window of its own."), bad: false });
            return;
        }
        let Some(window) = self.window.upgrade() else { return };
        let program = std::env::current_exe().unwrap_or_else(|_| "/usr/bin/gxwi-services-manager".into());
        let arguments = match service {
            Some(service) => vec!["--definition".to_string(), service.to_string()],
            None => vec!["--new".to_string()],
        };
        let open = service.map(str::to_ascii_lowercase);
        match std::process::Command::new(program).args(arguments).spawn() {
            // Waited for, so that it does not linger once it has gone, and
            // then it may be opened again.
            Ok(mut child) => {
                if let Some(open) = &open {
                    self.defining.insert(open.clone());
                }
                let window = Arc::downgrade(&window);
                std::thread::spawn(move || {
                    let _ = child.wait();
                    if let (Some(open), Some(window)) = (open, window.upgrade()) {
                        window.update(|manager, _| {
                            manager.defining.remove(&open);
                        });
                    }
                });
            }
            Err(e) => self.said = Some(Said { text: format!("The definition could not be opened: {e}."), bad: true }),
        }
    }

    /// Opens what `service` has logged, and whatever runs under it, in
    /// Event Viewer: a program of its own, whose window outlasts this one.
    fn logs(&mut self, service: &str) {
        match std::process::Command::new(EVENT_VIEWER).args(["--logs", service]).spawn() {
            // Waited for, so that it does not linger once it has gone.
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.said = Some(Said { text: "Event Viewer is not installed.".into(), bad: true }),
            Err(e) => self.said = Some(Said { text: format!("Event Viewer could not be started: {e}."), bad: true }),
        }
    }

    /// The button that opens `service`'s definition: to change, or to read.
    fn definition_button(&self, service: &str) -> String {
        let label = match &self.changeable {
            Some((chosen, Ok(()))) if chosen == service => "Edit definition…",
            _ => "Definition…",
        };
        let open = if self.defining(service) { " disabled title=\"Its definition is open already, in a window of its own.\"" } else { "" };
        format!(
            "<div class=\"opens\"><button type=\"button\" class=\"open\" fx-click=\"definition\" fx-value-service=\"{service}\"{open}>{label}</button>\
             <button type=\"button\" class=\"open\" fx-click=\"logs\" fx-value-service=\"{service}\">Logs…</button></div>",
            service = escape(service),
        )
    }

    /// Opens the editor on `which` permissions of `service`.
    fn permissions(&mut self, which: Which, service: &str) {
        let (title, open) = match which {
            Which::Every => ("every service".to_string(), (String::new(), which)),
            _ => match self.row(service) {
                Some(row) => (row.title().to_string(), (service.to_string(), which)),
                None => return,
            },
        };
        if self.editing.contains(&open) {
            self.said = Some(Said { text: format!("The permissions saying {} are open already for {title}.", which.says()), bad: false });
            return;
        }
        let (request, mut apply) = match permissions::open(which, service, &title) {
            Ok(opened) => opened,
            Err(why) => {
                self.said = Some(Said { text: format!("The permissions of {title} could not be opened: {why}."), bad: true });
                return;
            }
        };
        let looking = self.window.clone();
        let chosen = self.chosen();
        let applied = move |sd: &[u8], parts: &[Part]| {
            apply(sd, parts)?;
            // What the person may do may have changed with it.
            if let Some(window) = looking.upgrade() {
                let seen = system::look(chosen.as_deref());
                window.update(|manager, _| manager.seen(seen));
            }
            Ok(())
        };
        let window = self.window.clone();
        let closed = open.clone();
        let done = move || {
            if let Some(window) = window.upgrade() {
                window.update(|manager, _| {
                    manager.editing.remove(&closed);
                });
            }
        };
        match gxwi_sd_editor::edit(&request, applied, done) {
            Ok(()) => {
                self.editing.insert(open);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.said = Some(Said { text: "The permissions editor is not installed.".into(), bad: true }),
            Err(e) => self.said = Some(Said { text: format!("The permissions editor could not be started: {e}."), bad: true }),
        }
    }

    /// The buttons that open `service`'s permissions, and where who may
    /// control it comes from; and the default every service without its
    /// own takes, with the buttons that open and forget that.
    fn permission_buttons(&self, service: &str) -> String {
        let (from, control) = match (self.from.get(service), self.rights_of(service)) {
            (Some(from), _) => (permissions::from_words(*from).to_string(), String::new()),
            (None, Rights::Unknown(why)) => (format!("Who may control it could not be read: {why}."), " disabled".to_string()),
            (None, Rights::Granted(_)) => (String::new(), String::new()),
        };
        let service_html = escape(service);
        // A button that sets something back to the default, asked about
        // first, and offered where it may be done.
        let forget = |forget: Forget, label: &str, changeable: Result<(), String>, question: String, keep: &str| {
            if self.asking.as_ref() == Some(&forget) {
                return format!(
                    "<div class=\"asking\" role=\"group\" aria-label=\"{label}\"><p>{question}</p>\
                     <button type=\"button\" fx-click=\"forget-yes\" fx-value-service=\"{service_html}\" fx-autofocus>{label}</button>\
                     <button type=\"button\" fx-click=\"forget-no\" fx-value-service=\"{service_html}\">{keep}</button></div>",
                    question = escape(&question),
                );
            }
            let which = if forget == Forget::Every { "every" } else { "service" };
            match changeable {
                Ok(()) => format!("<button type=\"button\" fx-click=\"forget\" fx-value-which=\"{which}\" fx-value-service=\"{service_html}\">{label}</button>"),
                Err(why) => format!("<button type=\"button\" disabled title=\"{}\">{label}</button>", escape(&words::sentence(&why))),
            }
        };
        let own = if self.from.get(service) == Some(&From::Own) {
            let changeable = match &self.changeable {
                Some((chosen, changeable)) if chosen == service => changeable.clone(),
                _ => Err("whether you may has not been found out yet".into()),
            };
            let takes = match self.every {
                Ok(From::AllServices) => "the default set for every service",
                _ => "the service manager's built-in default",
            };
            let title = self.row(service).map_or_else(|| service.to_string(), |row| row.title().to_string());
            forget(Forget::Service(service.to_string()), "Use the default", changeable, format!("{title} will take {takes}. Who may control it as set for it now will be lost."), "Keep its own")
        } else {
            String::new()
        };
        let (every, every_set) = match &self.every {
            Ok(from) => (permissions::every_words(*from).to_string(), *from == From::AllServices),
            Err(why) => (format!("What a service without permissions of its own takes could not be read: {why}."), false),
        };
        let builtin = if every_set {
            let question = "Every service without permissions of its own will take the service manager's built-in default, and the default set for every service will be lost.".to_string();
            forget(Forget::Every, "Use the built-in default", self.every_changeable.clone(), question, "Keep it")
        } else {
            String::new()
        };
        let every_off = if self.every.is_err() { " disabled" } else { "" };
        format!(
            "<section class=\"permissions\" aria-label=\"Permissions\"><p class=\"note\">{from}</p>\
             <div class=\"together\"><button type=\"button\" fx-click=\"permissions\" fx-value-which=\"control\" fx-value-service=\"{service_html}\"{control}>Who may control it…</button>{own}</div>\
             <button type=\"button\" fx-click=\"permissions\" fx-value-which=\"definition\" fx-value-service=\"{service_html}\">Who may change its definition…</button>\
             </section>\
             <section class=\"permissions every\" aria-labelledby=\"every\"><h3 id=\"every\">Every service</h3><p class=\"note\">{every}</p>\
             <div class=\"together\"><button type=\"button\" fx-click=\"permissions\" fx-value-which=\"every\" fx-value-service=\"{service_html}\"{every_off}>Default permissions…</button>{builtin}</div>\
             </section>",
            from = escape(&from),
            every = escape(&every),
        )
    }

    /// Takes away what `forget` names, so that the default applies, and
    /// looks again.
    fn forget(&mut self, forget: Forget) {
        self.asking = None;
        let Some(window) = self.window.upgrade() else { return };
        let chosen = self.chosen();
        std::thread::spawn(move || {
            let (service, said) = match &forget {
                Forget::Service(service) => (Some(service.as_str()), "now takes the default"),
                Forget::Every => (None, ""),
            };
            let done = system::forget(service);
            let seen = system::look(chosen.as_deref());
            window.update(|manager, _| {
                let title = service.map(|service| manager.row(service).map_or_else(|| service.to_string(), |row| row.title().to_string()));
                manager.said = Some(match (done, title) {
                    (Ok(()), Some(title)) => Said { text: format!("{title} {said}."), bad: false },
                    (Ok(()), None) => Said { text: "Every service without permissions of its own now takes the service manager's built-in default.".into(), bad: false },
                    (Err(why), Some(title)) => Said { text: format!("{title} could not be set back to the default: {why}."), bad: true },
                    (Err(why), None) => Said { text: format!("The default for every service could not be taken away: {why}."), bad: true },
                });
                manager.seen(seen);
            });
        });
    }

    /// The buttons for `row`'s commands, each pressable or saying why not.
    fn buttons(&self, row: Option<&Row>) -> String {
        COMMANDS
            .iter()
            .map(|&command| {
                let name = words::command(command);
                match row.map(|row| (row, self.offer(command, row))) {
                    Some((row, Ok(()))) => format!(
                        "<button type=\"button\" fx-click=\"command\" fx-value-command=\"{}\" fx-value-service=\"{}\">{name}</button>",
                        words::verb(command),
                        escape(row.service()),
                    ),
                    Some((_, Err(why))) => format!("<button type=\"button\" disabled title=\"{}\">{name}</button>", escape(&why)),
                    None => format!("<button type=\"button\" disabled title=\"Pick a service first.\">{name}</button>"),
                }
            })
            .collect()
    }

    /// The menu of `row`'s commands.
    fn menu(&self, index: usize, row: &Row) -> String {
        let items: String = COMMANDS
            .iter()
            .map(|&command| {
                let disabled = if self.offer(command, row).is_ok() { "" } else { " disabled" };
                format!("<li><button type=\"button\" fx-click=\"command\" fx-value-command=\"{}\"{disabled}>{}</button></li>", words::verb(command), words::command(command))
            })
            .collect();
        let control = if self.from.contains_key(row.service()) { "" } else { " disabled" };
        format!(
            "<menu id=\"menu-{index}\" hidden>{items}<hr>\
             <li><button type=\"button\" fx-click=\"permissions\" fx-value-which=\"control\"{control}>Who may control it…</button></li>\
             <li><button type=\"button\" fx-click=\"permissions\" fx-value-which=\"definition\">Who may change its definition…</button></li><hr>\
             <li><button type=\"button\" fx-click=\"definition\"{defining}>Definition…</button></li>\
             <li><button type=\"button\" fx-click=\"logs\">Logs…</button></li>\
             <li><button type=\"button\" fx-copy=\"service\">Copy name</button></li></menu>",
            defining = if self.defining(row.service()) { " disabled" } else { "" },
        )
    }

    /// Whether `service`'s definition is open in a window already.
    fn defining(&self, service: &str) -> bool {
        self.defining.contains(&service.to_ascii_lowercase())
    }

    /// What may be done with `service`, in a sentence.
    fn may(&self, service: &str) -> String {
        match self.rights_of(service) {
            Rights::Granted(granted) => {
                let may: Vec<&str> = [Command::Start, Command::Stop, Command::Reload]
                    .into_iter()
                    .filter(|&command| Rights::Granted(granted).allow(command))
                    .map(words::verb)
                    .collect();
                match may.as_slice() {
                    [] => "You may not start, stop or reload it.".into(),
                    [one] => format!("You may {one} it, and nothing else."),
                    [first, second] => format!("You may {first} and {second} it."),
                    _ => "You may start, stop and reload it.".into(),
                }
            }
            Rights::Unknown(why) => format!("Whether you may control it is not known: {why}. Its commands are offered, and the service manager decides."),
        }
    }

    /// Its calendar timers, each under its schedule: when it runs next and
    /// when it last ran, or why it never runs. As peinit has them armed, so
    /// a timer's random delay is in its next run already.
    fn timers(&self, timers: &[Timer]) -> String {
        if timers.is_empty() {
            return String::new();
        }
        let (now, zone) = self.clock.now();
        let when = |at: &str| words::capital(&words::when(at, now, &zone));
        let each: String = timers
            .iter()
            .map(|timer| {
                let said = match (&timer.not_armed, &timer.fires_at) {
                    (Some(why), _) => format!("<p class=\"note bad\">It never runs: {}.</p>", escape(why)),
                    (None, Some(fires)) => {
                        let next = match words::until(fires, now) {
                            Some(until) => format!("{}, in {until}", when(fires)),
                            None => "Now".to_string(),
                        };
                        let last = timer.last_fired_at.as_deref().map_or_else(|| "Not since the machine started".to_string(), when);
                        let delayed = match &timer.scheduled_at {
                            Some(scheduled) if scheduled != fires => format!(
                                "<p class=\"note\">It is due {}, and put back by a random delay.</p>",
                                escape(&words::when(scheduled, now, &zone))
                            ),
                            _ => String::new(),
                        };
                        format!("<dl><dt>Next run</dt><dd>{}</dd><dt>Last run</dt><dd>{}</dd></dl>{delayed}", escape(&next), escape(&last))
                    }
                    (None, None) => String::new(),
                };
                format!("<div class=\"timer\"><p class=\"schedule\"><code>{}</code></p>{said}</div>", escape(&timer.schedule))
            })
            .collect();
        format!("<section class=\"timers\"><h3>{}</h3>{each}</section>", if timers.len() == 1 { "Timer" } else { "Timers" })
    }

    /// The pane about the picked service.
    fn details(&self) -> String {
        let Some(row) = self.picked.as_deref().and_then(|picked| self.row(picked)) else {
            return "<aside class=\"details empty\" aria-label=\"Details\"><p>Pick a service to see what it is doing.</p></aside>".into();
        };
        let row_of = |name: &str, value: &str| format!("<dt>{name}</dt><dd>{}</dd>", escape(value));
        let mut facts = String::new();
        let mut notes = String::new();
        let mut timers = String::new();
        // Its name goes under its title where the title is another.
        let (id, description) = (row.display_name().map(|_| row.service()), row.description());
        if let Row::Listed(summary) = &row {
            let state = match &summary.cause {
                Some(cause) => format!("{}, {}", words::state(summary.state), words::cause(cause)),
                None => words::state(summary.state).to_string(),
            };
            facts += &row_of("State", &state);
            if let Some(health) = summary.health {
                facts += &row_of("Health", words::health(health));
            }
        }
        match self.detail.as_ref().filter(|detail| detail.service == row.service()).map(|detail| &detail.status) {
            Some(Ok(status)) => {
                if let Some(job) = &status.job {
                    if let Some(pid) = job.pid {
                        facts += &row_of("Process", &pid.to_string());
                    }
                    facts += &row_of("Runs as", &words::principal(&job.identity));
                }
                if let Some(uptime) = status.uptime_seconds {
                    facts += &row_of("Running for", &words::duration(uptime));
                }
                if let Some(said) = &status.status_text {
                    facts += &row_of("It says", said);
                }
                if let Some(operation) = &status.operation {
                    facts += &row_of("Under way", &format!("a {} {}", operation.kind, words::source(&operation.source)));
                }
                if status.definition_removed {
                    notes += "<p class=\"note\">Its definition has been removed. It carries on until it stops, and is gone then.</p>";
                }
                for warning in &status.warnings {
                    notes += &format!("<p class=\"note bad\">An earlier run left processes behind, in {}.</p>", escape(&warning.path));
                }
                timers = self.timers(&status.timers);
            }
            Some(Err(why)) => notes += &format!("<p class=\"note\">{}</p>", escape(why)),
            None if matches!(row, Row::Listed(_)) => notes += "<p class=\"note\">Asking what it is doing…</p>",
            None => {}
        }
        if matches!(row, Row::Hidden(_)) {
            notes += "<p class=\"note\">Its state is not yours to see. It is listed because you may read which services are defined.</p>";
        }
        let may = if self.refused { "The service manager does not let you ask it anything.".to_string() } else { self.may(row.service()) };
        format!(
            "<aside class=\"details\" aria-label=\"Details\">\
             <h2>{title}</h2>{id}{description}\
             <dl>{facts}</dl>{notes}{timers}{definition}\
             <p class=\"may\">{may}</p>{permissions}\
             </aside>",
            definition = self.definition_button(row.service()),
            title = escape(row.title()),
            id = id.map(|id| format!("<p class=\"id\">{}</p>", escape(id))).unwrap_or_default(),
            description = description.map(|description| format!("<p class=\"description\">{}</p>", escape(description))).unwrap_or_default(),
            may = escape(&may),
            permissions = self.permission_buttons(row.service()),
        )
    }

    /// The footer: how many, and what is not listed.
    fn footer(&self, shown: usize) -> String {
        let running = self.services.iter().filter(|service| matches!(service.state, State::Active | State::Reloading)).count();
        if self.refused {
            let defined = match self.hidden_count() {
                1 => "1 service defined".to_string(),
                count => format!("{count} services defined"),
            };
            let defined = if self.hidden == Hidden::Unknowable { String::new() } else { defined };
            return format!("<footer class=\"status\"><span>{defined}</span><span>Their state is not yours to see</span></footer>");
        }
        if !self.looked || (self.trouble.is_some() && self.services.is_empty()) {
            return "<footer class=\"status\"><span></span><span></span></footer>".into();
        }
        let counted = if shown == self.services.len() + self.hidden_count() {
            format!("{} services · {running} running", self.services.len())
        } else {
            format!("{shown} shown of {} · {running} running", self.services.len() + self.hidden_count())
        };
        let hidden = match &self.hidden {
            Hidden::These(names) if names.is_empty() => String::new(),
            Hidden::These(names) if names.len() == 1 => "1 more whose state you may not see".into(),
            Hidden::These(names) => format!("{} more whose state you may not see", names.len()),
            Hidden::Unknowable => "Services whose state you may not see are not listed".into(),
        };
        format!("<footer class=\"status\"><span>{counted}</span><span>{hidden}</span></footer>")
    }

    fn hidden_count(&self) -> usize {
        match &self.hidden {
            Hidden::These(names) => names.len(),
            Hidden::Unknowable => 0,
        }
    }
}

impl Live for Manager {
    fn render(&self, facts: &Facts) -> String {
        let rows = self.rows(facts.fields.get("find"));
        let (now, zone) = self.clock.now();
        let listing: String = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let (tone, state) = match (row.state(), self.working.get(row.service())) {
                    (_, Some(doing)) => ("busy", doing.clone()),
                    (Some(state), None) => (words::tone(state), words::state(state).to_string()),
                    (None, None) => ("unknown", "Not yours to see".to_string()),
                };
                let description = row.description().unwrap_or("");
                let next = match row {
                    Row::Listed(Summary { next_timer_at: Some(next), .. }) => words::capital(&words::when(next, now, &zone)),
                    _ => String::new(),
                };
                format!(
                    "<li><button type=\"button\" id=\"service-{index}\" fx-click=\"pick\" fx-menu=\"menu-{index}\" fx-value-service=\"{service}\" aria-selected=\"{picked}\">\
                     <span class=\"dot {tone}\" aria-hidden=\"true\"></span><span class=\"name\">{title}</span>\
                     <span class=\"state\">{state}</span><span class=\"next\">{next}</span><span class=\"description\">{description}</span>\
                     </button></li>",
                    service = escape(row.service()),
                    picked = self.picked.as_deref() == Some(row.service()),
                    title = escape(row.title()),
                    state = escape(&state),
                    next = escape(&next),
                    description = escape(description),
                )
            })
            .collect();
        let menus: String = rows.iter().enumerate().map(|(index, row)| self.menu(index, row)).collect();
        let picked = self.picked.as_deref().and_then(|picked| self.row(picked));
        let at = picked.as_ref().and_then(|picked| rows.iter().position(|row| row.service() == picked.service()));
        let near = |step: isize| {
            let to = match at {
                Some(at) => at.saturating_add_signed(step).min(rows.len().saturating_sub(1)),
                None => 0,
            };
            rows.get(to).map(|row| escape(row.service())).unwrap_or_default()
        };
        let trouble = self.trouble.as_ref().map(|why| format!("<p class=\"trouble\">{}</p>", escape(why))).unwrap_or_default();
        let said = self
            .said
            .as_ref()
            .map(|said| format!("<p class=\"said{}\" role=\"status\">{}</p>", if said.bad { " bad" } else { "" }, escape(&said.text)))
            .unwrap_or_default();
        let empty = match (self.looked, rows.is_empty(), self.trouble.is_some()) {
            (false, _, false) => "<p class=\"more\">Asking the service manager…</p>",
            (true, true, false) if !facts.fields.get("find").trim().is_empty() => "<p class=\"more\">No service is called that.</p>",
            (true, true, false) => "<p class=\"more\">There are no services you may see.</p>",
            _ => "",
        };
        format!(
            "<div hidden>\
             <button type=\"button\" fx-key=\"ArrowDown\" fx-click=\"pick\" fx-value-service=\"{next}\"></button>\
             <button type=\"button\" fx-key=\"ArrowUp\" fx-click=\"pick\" fx-value-service=\"{previous}\"></button>\
             </div>\
             <div class=\"bar\">\
             <input name=\"find\" autocomplete=\"off\" spellcheck=\"false\" placeholder=\"Find a service\" aria-label=\"Find a service\">\
             <span class=\"commands\">{buttons}</span>\
             <button type=\"button\" class=\"refresh\" fx-click=\"refresh\" fx-key=\"F5\" title=\"Look again (F5)\">Refresh</button>\
             {new}\
             </div>\
             <div class=\"body\">\
             <div class=\"listing\" id=\"listing\" fx-columns=\"10px minmax(0, 1.3fr) 150px 140px minmax(0, 2fr)\">\
             <div class=\"head\"><span></span><span>Name</span><span>State</span><span>Next run</span><span>Description</span></div>\
             {trouble}{said}<ul class=\"entries\">{listing}</ul>{empty}\
             </div>\
             {details}\
             </div>\
             {footer}{menus}",
            next = near(1),
            previous = near(-1),
            new = match &self.creatable {
                Ok(()) => "<button type=\"button\" class=\"new\" fx-click=\"new\">New service…</button>".to_string(),
                Err(why) => format!("<button type=\"button\" class=\"new\" disabled title=\"{}\">New service…</button>", escape(&words::sentence(why))),
            },
            buttons = self.buttons(picked.as_ref()),
            details = self.details(),
            footer = self.footer(rows.len()),
        )
    }

    fn event(&mut self, name: &str, value: &Value, _: &mut Fields) {
        let service = value["service"].as_str().filter(|service| !service.is_empty()).map(str::to_string);
        match name {
            "pick" => {
                if let Some(service) = service {
                    self.pick(service);
                }
            }
            "command" => {
                if let (Some(command), Some(service)) = (value["command"].as_str().and_then(words::named), service) {
                    self.pick(service.clone());
                    self.command(command, &service);
                }
            }
            "permissions" => {
                if let (Some(which), Some(service)) = (value["which"].as_str().and_then(Which::named), service) {
                    self.pick(service.clone());
                    self.permissions(which, &service);
                }
            }
            "forget" => {
                self.asking = match (value["which"].as_str(), service) {
                    (Some("every"), _) => Some(Forget::Every),
                    (Some("service"), Some(service)) if self.from.get(&service) == Some(&From::Own) => Some(Forget::Service(service)),
                    _ => None,
                };
            }
            "forget-yes" => {
                if let Some(forget) = self.asking.take() {
                    self.forget(forget);
                }
            }
            "forget-no" => self.asking = None,
            "definition" => {
                if let Some(service) = service {
                    self.pick(service.clone());
                    self.definition(Some(&service));
                }
            }
            "logs" => {
                if let Some(service) = service {
                    self.pick(service.clone());
                    self.logs(&service);
                }
            }
            "new" if self.creatable.is_ok() => self.definition(None),
            "refresh" => {
                self.said = None;
                self.refresh();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(service: &str, display: Option<&str>, state: State) -> Summary {
        Summary { service: service.into(), display_name: display.map(Into::into), description: None, state, cause: None, health: None, next_timer_at: None }
    }

    /// A window that has looked once and found three services, one of them
    /// only in the registry, with the rights given.
    fn manager(rights: &[(&str, Rights)]) -> Manager {
        let mut manager = Manager::new();
        manager.seen(Seen {
            services: Ok(vec![
                summary("timed", Some("Time client"), State::Active),
                summary("sshd", None, State::Inactive),
            ]),
            hidden: Hidden::These(vec![Defined::named("secret")]),
            rights: rights.iter().cloned().map(|(name, rights)| (name.to_string(), rights)).collect(),
            from: rights.iter().filter(|(_, rights)| matches!(rights, Rights::Granted(_))).map(|(name, _)| (name.to_string(), From::BuiltIn)).collect(),
            every: Ok(From::BuiltIn),
            every_changeable: Ok(()),
            changeable: None,
            creatable: Ok(()),
        });
        manager
    }

    fn shown(manager: &Manager, find: &str) -> String {
        let mut fields = Fields::default();
        fields.set("find", find);
        manager.render(&Facts { views: 1, fields: &fields })
    }

    fn pick(manager: &mut Manager, service: &str) {
        manager.event("pick", &serde_json::json!({ "service": service }), &mut Fields::default());
    }

    /// The labels and whether each is pressable, of the bar's commands.
    fn bar(html: &str) -> Vec<(String, bool)> {
        let bar = html.split("<span class=\"commands\">").nth(1).unwrap().split("</span>").next().unwrap();
        bar.split("</button>")
            .filter(|button| !button.is_empty())
            .map(|button| (button.rsplit('>').next().unwrap().to_string(), !button.contains(" disabled")))
            .collect()
    }

    #[test]
    fn services_are_listed_by_title_and_the_hidden_ones_say_so() {
        let manager = manager(&[]);
        let html = shown(&manager, "");
        let titles: Vec<&str> = html.split("<span class=\"name\">").skip(1).map(|rest| rest.split('<').next().unwrap()).collect();
        assert_eq!(titles, ["secret", "sshd", "Time client"]);
        assert!(html.contains("<span class=\"state\">Not yours to see</span>"));
        assert!(html.contains("2 services · 1 running"));
        assert!(html.contains("1 more whose state you may not see"));
        // Narrowed by what is typed, matched in the name or the title.
        let html = shown(&manager, "TIME");
        assert!(html.contains("Time client") && !html.contains("sshd"));
        assert!(html.contains("1 shown of 3"));
        assert!(shown(&manager, "nothing").contains("No service is called that."));
    }

    #[test]
    fn where_the_registry_cannot_be_read_it_says_some_may_be_missing() {
        let mut manager = manager(&[]);
        manager.seen(Seen {
            services: Ok(Vec::new()),
            hidden: Hidden::Unknowable,
            rights: HashMap::new(),
            from: HashMap::new(),
            every: Ok(From::BuiltIn),
            every_changeable: Ok(()),
            changeable: None,
            creatable: Ok(()),
        });
        let html = shown(&manager, "");
        assert!(html.contains("Services whose state you may not see are not listed"));
        assert!(html.contains("There are no services you may see."));
    }

    #[test]
    fn turned_away_by_the_service_manager_what_is_defined_is_listed_and_nothing_offered() {
        let mut manager = Manager::new();
        manager.seen(Seen {
            services: Err(Unasked::NotAllowed),
            hidden: Hidden::These(vec![
                Defined { name: "timed".into(), display_name: Some("Time client".into()), description: Some("Keeps the clock".into()) },
                Defined::named("sshd"),
            ]),
            rights: [("timed".to_string(), Rights::Granted(0xf))].into_iter().collect(),
            from: HashMap::new(),
            every: Err("you may not read who may control it".into()),
            every_changeable: Err("you may not change it".into()),
            changeable: None,
            creatable: Ok(()),
        });
        pick(&mut manager, "timed");
        manager.detailed("timed", Err(Failure::Unreachable("connect: Permission denied".into())));
        let html = shown(&manager, "");
        assert!(html.contains("The service manager does not let you ask it anything, so the state"));
        assert!(html.contains("2 services defined") && html.contains("Their state is not yours to see"));
        assert_eq!(html.matches("<span class=\"state\">Not yours to see</span>").count(), 2);
        // What the definition says it is called, and does, is shown where it can be read.
        assert!(html.contains("<span class=\"name\">Time client</span>") && html.contains("Keeps the clock"));
        assert!(html.contains("<span class=\"name\">sshd</span>"));
        // Whatever the descriptor says, peinit will not hear it.
        assert!(bar(&html).iter().all(|(_, pressable)| !pressable));
        assert!(!html.contains("Permission denied"));
    }

    #[test]
    fn a_command_is_offered_only_with_its_right_and_where_it_does_something() {
        let all = Rights::Granted(0xf);
        let mut manager = manager(&[("timed", all.clone()), ("sshd", Rights::Granted(0x1))]);
        // Nothing is picked: nothing is pressable.
        assert!(bar(&shown(&manager, "")).iter().all(|(_, pressable)| !pressable));
        pick(&mut manager, "timed");
        // Running: it can be stopped, restarted and reloaded, not started or reset.
        assert_eq!(
            bar(&shown(&manager, "")),
            [("Start".into(), false), ("Stop".into(), true), ("Restart".into(), true), ("Reload".into(), true), ("Reset".into(), false)]
        );
        let html = shown(&manager, "");
        assert!(html.contains("title=\"It is running already.\""));
        assert!(html.contains("You may start, stop and reload it."));
        // Only allowed to look: nothing is pressable, and it says why.
        pick(&mut manager, "sshd");
        let html = shown(&manager, "");
        assert!(bar(&html).iter().all(|(_, pressable)| !pressable));
        assert!(html.contains("title=\"You are not allowed to start it.\""));
        assert!(html.contains("You may not start, stop or reload it."));
        // Asked anyway, it is not sent.
        manager.event("command", &serde_json::json!({ "command": "start", "service": "sshd" }), &mut Fields::default());
        assert_eq!(manager.said.as_ref().map(|said| said.text.as_str()), Some("sshd was not started: You are not allowed to start it."));
        assert!(manager.working.is_empty());
    }

    #[test]
    fn where_who_may_control_it_cannot_be_read_the_commands_are_offered() {
        let mut manager = manager(&[("timed", Rights::Unknown("you may not read who may control it".into()))]);
        pick(&mut manager, "timed");
        let html = shown(&manager, "");
        assert!(bar(&html).iter().any(|(label, pressable)| label == "Stop" && *pressable));
        assert!(html.contains("Whether you may control it is not known: you may not read who may control it."));
    }

    #[test]
    fn the_permissions_open_from_the_details_and_say_where_they_come_from() {
        let mut manager = manager(&[("timed", Rights::Granted(0xf)), ("sshd", Rights::Unknown("you may not read who may control it".into()))]);
        manager.from.insert("timed".into(), From::Own);
        pick(&mut manager, "timed");
        let html = shown(&manager, "");
        assert!(html.contains("Who may control it is set for this service."));
        assert!(html.contains("fx-value-which=\"control\" fx-value-service=\"timed\">Who may control it…</button>"));
        assert!(html.contains("fx-value-which=\"definition\" fx-value-service=\"timed\">Who may change its definition…</button>"));
        // Where who may control it cannot be read, that is not offered, and
        // why is said; the definition's own may still be.
        pick(&mut manager, "sshd");
        let html = shown(&manager, "");
        assert!(html.contains("Who may control it could not be read: you may not read who may control it."));
        assert!(html.contains("fx-value-service=\"sshd\" disabled>Who may control it…"));
        assert!(html.contains("fx-value-service=\"sshd\">Who may change its definition…"));
        // A hidden service's are offered too: they are the registry's, not peinit's.
        pick(&mut manager, "secret");
        assert!(shown(&manager, "").contains("fx-value-service=\"secret\">Who may change its definition…"));
    }

    #[test]
    fn what_was_set_goes_back_to_the_default_once_the_person_says_so() {
        let mut manager = manager(&[("timed", Rights::Granted(0xf)), ("sshd", Rights::Granted(0xf))]);
        manager.from.insert("timed".into(), From::Own);
        pick(&mut manager, "timed");
        // Whether its definition may be changed is not known yet: the
        // button says so, unpressable.
        assert!(shown(&manager, "").contains("disabled title=\"Whether you may has not been found out yet.\">Use the default</button>"));
        manager.changeable = Some(("timed".into(), Ok(())));
        let html = shown(&manager, "");
        assert!(html.contains("fx-click=\"forget\" fx-value-which=\"service\" fx-value-service=\"timed\">Use the default</button>"));
        // Pressed, it asks, saying what it will take; kept, nothing is done.
        manager.event("forget", &serde_json::json!({ "which": "service", "service": "timed" }), &mut Fields::default());
        let html = shown(&manager, "");
        assert!(html.contains("Time client will take the service manager's built-in default. Who may control it as set for it now will be lost."));
        assert!(html.contains("fx-click=\"forget-yes\"") && html.contains(">Keep its own</button>"));
        manager.event("forget-no", &serde_json::json!({}), &mut Fields::default());
        assert_eq!(manager.asking, None);
        // One taking the default already has nothing to go back to.
        pick(&mut manager, "sshd");
        assert!(!shown(&manager, "").contains("Use the default</button>"));
        // Every service: opened from any, and set back to the built-in default
        // only where something is set, and the person may.
        let html = shown(&manager, "");
        assert!(html.contains("A service without permissions of its own takes the service manager's built-in default."));
        assert!(html.contains("fx-value-which=\"every\" fx-value-service=\"sshd\">Default permissions…</button>"));
        assert!(!html.contains("Use the built-in default"));
        manager.every = Ok(From::AllServices);
        manager.every_changeable = Err("you may not change it".into());
        let html = shown(&manager, "");
        assert!(html.contains("takes the default set for every service."));
        assert!(html.contains("disabled title=\"You may not change it.\">Use the built-in default</button>"));
        // Asked for anyway, by a service with nothing of its own, nothing is asked.
        manager.event("forget", &serde_json::json!({ "which": "service", "service": "sshd" }), &mut Fields::default());
        assert_eq!(manager.asking, None);
    }

    #[test]
    fn what_came_of_a_command_is_said_in_words() {
        let mut manager = manager(&[("timed", Rights::Granted(0xf))]);
        manager.asked(Command::Restart, "timed", Outcome::Done(None));
        assert_eq!(manager.said, Some(Said { text: "Time client was restarted.".into(), bad: false }));
        manager.asked(Command::Reload, "timed", Outcome::Done(Some("reload signal confirmed".into())));
        assert_eq!(manager.said, Some(Said { text: "Time client was reloaded.".into(), bad: false }));
        manager.asked(Command::Reload, "timed", Outcome::Done(Some("reload signal advisory: detection window expired".into())));
        assert_eq!(manager.said, Some(Said { text: "Time client was asked to reload, and did not say whether it had.".into(), bad: false }));
        manager.asked(Command::Stop, "timed", Outcome::Refused { code: "ACCESS_DENIED".into(), message: "access denied".into() });
        assert_eq!(manager.said, Some(Said { text: "You are not allowed to stop Time client.".into(), bad: true }));
        manager.asked(Command::Start, "sshd", Outcome::Failed("its program could not be found".into()));
        assert_eq!(manager.said, Some(Said { text: "sshd could not be started: its program could not be found".into(), bad: true }));
    }

    #[test]
    fn the_details_say_what_it_is_doing_and_why() {
        let mut manager = manager(&[("timed", Rights::Granted(0xf))]);
        manager.services[0].cause = Some("restart_policy".into());
        pick(&mut manager, "timed");
        manager.detailed(
            "timed",
            Ok(Status {
                summary: manager.services[0].clone(),
                status_text: Some("Synchronised".into()),
                job: Some(peinit::client::Job { id: "j".into(), pid: Some(412), started_at: None, identity: "S-1-5-19".into() }),
                operation: None,
                uptime_seconds: Some(4000),
                definition_removed: false,
                warnings: Vec::new(),
                timers: Vec::new(),
                progress: None,
                granted: vec!["query_status".into(), "start".into(), "stop".into(), "interrogate".into()],
            }),
        );
        let html = shown(&manager, "");
        assert!(!html.contains("class=\"timers\""));
        assert!(html.contains("<dt>State</dt><dd>Running, restarted after it stopped</dd>"));
        assert!(html.contains("<dt>Process</dt><dd>412</dd>"));
        assert!(html.contains("<dt>Runs as</dt><dd>Local Service</dd>"));
        assert!(html.contains("<dt>Running for</dt><dd>1 h 6 min</dd>"));
        assert!(html.contains("<p class=\"id\">timed</p>"));
        // What it has logged opens in Event Viewer, from the details and its menu.
        assert!(html.contains("fx-click=\"logs\" fx-value-service=\"timed\">Logs…</button>"));
        assert!(html.contains("<li><button type=\"button\" fx-click=\"logs\">Logs…</button></li>"));
        // A hidden one says its state is not the person's, and is not asked
        // about again and again only to be refused.
        pick(&mut manager, "secret");
        assert_eq!(manager.picked(), None);
        let html = shown(&manager, "");
        assert!(html.contains("Its state is not yours to see."));
    }

    #[test]
    fn a_definition_open_in_its_window_is_not_opened_again() {
        let mut manager = manager(&[("timed", Rights::Granted(0xf))]);
        pick(&mut manager, "timed");
        manager.defining.insert("timed".into());
        let html = shown(&manager, "");
        assert!(html.contains("fx-value-service=\"timed\" disabled title=\"Its definition is open already, in a window of its own.\">Definition…</button>"));
        assert!(html.contains("<button type=\"button\" fx-click=\"definition\" disabled>Definition…</button>"));
        // Asked for all the same, it is not opened.
        manager.event("definition", &serde_json::json!({ "service": "timed" }), &mut Fields::default());
        assert_eq!(manager.said, Some(Said { text: "The definition of Time client is open already, in a window of its own.".into(), bad: false }));
    }

    #[test]
    fn a_timer_says_when_it_runs_next_and_last_ran_or_why_it_never_does() {
        let mut manager = manager(&[("sshd", Rights::Granted(0xf))]);
        // Saturday 3 October 2026, 13:20, on a clock an hour east of UTC.
        manager.clock = words::Clock::Fixed("2026-10-03T12:20:00Z".parse().unwrap(), jiff::tz::TimeZone::fixed(jiff::tz::offset(1)));
        manager.services[1].next_timer_at = Some("2026-10-03T13:00:00.000000000Z".into());
        let html = shown(&manager, "");
        assert!(html.contains("<span>Next run</span>"));
        assert!(html.contains("<span class=\"next\">Today at 14:00</span>"));
        assert!(html.contains("<span class=\"next\"></span>"));
        pick(&mut manager, "sshd");
        let timer = |schedule: &str, scheduled: Option<&str>, fires: Option<&str>, last: Option<&str>, not_armed: Option<&str>| peinit::client::Timer {
            schedule: schedule.into(),
            scheduled_at: scheduled.map(Into::into),
            fires_at: fires.map(Into::into),
            last_fired_at: last.map(Into::into),
            not_armed: not_armed.map(Into::into),
        };
        manager.detailed(
            "sshd",
            Ok(Status {
                summary: manager.services[1].clone(),
                status_text: None,
                job: None,
                operation: None,
                uptime_seconds: None,
                definition_removed: false,
                warnings: Vec::new(),
                timers: vec![
                    timer("*-*-* 01:00:00", Some("2026-10-04T00:00:00.000000000Z"), Some("2026-10-04T00:07:12.000000000Z"), Some("2026-10-03T00:03:40.000000000Z"), None),
                    timer("hourly", Some("2026-10-03T13:00:00.000000000Z"), Some("2026-10-03T13:00:00.000000000Z"), None, None),
                    timer("*-02-30", None, None, None, Some("calendar expression has no future occurrence")),
                ],
                progress: None,
                granted: vec!["query_status".into(), "start".into(), "stop".into(), "interrogate".into()],
            }),
        );
        let html = shown(&manager, "");
        assert!(html.contains("<h3>Timers</h3>"));
        assert!(html.contains("<code>*-*-* 01:00:00</code></p><dl><dt>Next run</dt><dd>Tomorrow at 01:07:12, in 11 h 47 min</dd><dt>Last run</dt><dd>Today at 01:03:40</dd></dl>"));
        assert!(html.contains("It is due tomorrow at 01:00, and put back by a random delay."));
        assert!(html.contains("<code>hourly</code></p><dl><dt>Next run</dt><dd>Today at 14:00, in 40 min</dd><dt>Last run</dt><dd>Not since the machine started</dd></dl></div>"));
        assert!(html.contains("<code>*-02-30</code></p><p class=\"note bad\">It never runs: calendar expression has no future occurrence.</p>"));
    }
}
