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

use std::collections::{BTreeMap, HashMap};
use std::sync::Weak;

use libgxwi::{Facts, Fields, Live, Surface, Value, escape};
use peinit::client::{Admission, Command, Failure, State, Status, Summary, admission};

use crate::system::{self, Defined, Hidden, Outcome, Rights, Seen, Unasked};
use crate::words;

/// What is said where peinit's control socket turns the person away.
const REFUSED: &str = "The service manager does not let you ask it anything, so the state of the services here is not yours to see, and none of them is yours to start or stop.";

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
            looked: false,
            refused: false,
            trouble: None,
            picked: None,
            detail: None,
            working: BTreeMap::new(),
            said: None,
            window: Weak::new(),
        }
    }

    /// The service picked, for whoever looks again to ask about.
    pub fn picked(&self) -> Option<String> {
        self.picked.clone()
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
        self.picked = Some(service.clone());
        let Some(window) = self.window.upgrade() else { return };
        std::thread::spawn(move || {
            let status = system::status(&service);
            window.update(|manager, _| manager.detailed(&service, status));
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
            let seen = system::look();
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
            Outcome::Done => (format!("{title} was {}.", words::done(command)), false),
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
        let picked = self.picked.clone();
        std::thread::spawn(move || {
            let seen = system::look();
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
        format!("<menu id=\"menu-{index}\" hidden>{items}<hr><li><button type=\"button\" fx-copy=\"service\">Copy name</button></li></menu>")
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

    /// The pane about the picked service.
    fn details(&self) -> String {
        let Some(row) = self.picked.as_deref().and_then(|picked| self.row(picked)) else {
            return "<aside class=\"details empty\" aria-label=\"Details\"><p>Pick a service to see what it is doing.</p></aside>".into();
        };
        let row_of = |name: &str, value: &str| format!("<dt>{name}</dt><dd>{}</dd>", escape(value));
        let mut facts = String::new();
        let mut notes = String::new();
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
             <dl>{facts}</dl>{notes}\
             <p class=\"may\">{may}</p>\
             </aside>",
            title = escape(row.title()),
            id = id.map(|id| format!("<p class=\"id\">{}</p>", escape(id))).unwrap_or_default(),
            description = description.map(|description| format!("<p class=\"description\">{}</p>", escape(description))).unwrap_or_default(),
            may = escape(&may),
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
                format!(
                    "<li><button type=\"button\" id=\"service-{index}\" fx-click=\"pick\" fx-menu=\"menu-{index}\" fx-value-service=\"{service}\" aria-selected=\"{picked}\">\
                     <span class=\"dot {tone}\" aria-hidden=\"true\"></span><span class=\"name\">{title}</span>\
                     <span class=\"state\">{state}</span><span class=\"description\">{description}</span>\
                     </button></li>",
                    service = escape(row.service()),
                    picked = self.picked.as_deref() == Some(row.service()),
                    title = escape(row.title()),
                    state = escape(&state),
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
             </div>\
             <div class=\"body\">\
             <div class=\"listing\" id=\"listing\" fx-columns=\"10px minmax(0, 1.3fr) 150px minmax(0, 2fr)\">\
             <div class=\"head\"><span></span><span>Name</span><span>State</span><span>Description</span></div>\
             {trouble}{said}<ul class=\"entries\">{listing}</ul>{empty}\
             </div>\
             {details}\
             </div>\
             {footer}{menus}",
            next = near(1),
            previous = near(-1),
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
        Summary { service: service.into(), display_name: display.map(Into::into), description: None, state, cause: None, health: None }
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
        manager.seen(Seen { services: Ok(Vec::new()), hidden: Hidden::Unknowable, rights: HashMap::new() });
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
    fn what_came_of_a_command_is_said_in_words() {
        let mut manager = manager(&[("timed", Rights::Granted(0xf))]);
        manager.asked(Command::Restart, "timed", Outcome::Done);
        assert_eq!(manager.said, Some(Said { text: "Time client was restarted.".into(), bad: false }));
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
            }),
        );
        let html = shown(&manager, "");
        assert!(html.contains("<dt>State</dt><dd>Running, restarted after it stopped</dd>"));
        assert!(html.contains("<dt>Process</dt><dd>412</dd>"));
        assert!(html.contains("<dt>Runs as</dt><dd>Local Service</dd>"));
        assert!(html.contains("<dt>Running for</dt><dd>1 h 6 min</dd>"));
        assert!(html.contains("<p class=\"id\">timed</p>"));
        // A hidden one says its state is not the person's.
        pick(&mut manager, "secret");
        manager.detailed("secret", Err(Failure::Refused { code: "ACCESS_DENIED".into(), message: String::new() }));
        let html = shown(&manager, "");
        assert!(html.contains("Its state is not yours to see."));
    }
}
