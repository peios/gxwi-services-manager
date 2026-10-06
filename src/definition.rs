//! A service's definition, in a window of its own: every field, by group,
//! with what it does, its default and when a change takes effect, to read,
//! change, and save; or a new service, to define.
//!
//! Services Manager opens it as a program of its own
//! (`gxwi-services-manager --definition sshd`, or `--new`), so that it can sit
//! beside the list and outlive it.
//!
//! WHAT IS SHOWN is the key's values as peinit reads them
//! (`peinit::client::Definition`), each field in words, and the values that
//! are not fields, which are kept as they are. A field reports a change when
//! the keyboard leaves it, and the whole definition is then checked as peinit
//! checks it: what is wrong is said beside the field it is about, and it
//! cannot be saved until it is put right.
//!
//! WHAT MAY BE DONE is asked of the registry: whether the key opens for
//! setting values, and for deleting it. A definition the person may not
//! change is shown with every field fixed, and why.
//!
//! SAVING writes the values that changed and nothing else, in one
//! transaction that is refused if any of them has changed since it was read
//! (`store`), and then says which changes wait for a restart.

use std::collections::BTreeMap;
use std::sync::Weak;

use libgxwi::{Closer, Facts, Fields, Live, Surface, Value, escape};
use peinit::client::{Definition, FIELDS, FieldGroup, FieldInfo, FieldKind, Problem, State, TakesEffect, changes, service_field};

use crate::{fields, store, system, words};

/// What is being asked of the person before it is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asking {
    Delete,
    Close,
}

pub struct Editor {
    /// Whether the definition is in the registry yet: a new one is not,
    /// until it is saved.
    made: bool,
    /// What it was when read, or last saved: what is changed is against it.
    found: Definition,
    /// What it is now, with what the person has done.
    edited: Definition,
    /// What the person typed that does not say a value, by field.
    wrong: BTreeMap<&'static str, String>,
    /// Why peinit would not take it as it is now.
    problem: Option<Problem>,
    /// Whether the definition may be changed, or defined, or why not.
    may_change: Result<(), String>,
    may_delete: Result<(), String>,
    /// Why there is nothing to show, where there is not.
    missing: Option<String>,
    asking: Option<Asking>,
    saving: bool,
    said: Option<(String, bool)>,
    /// What when a schedule comes round is said against.
    clock: words::Clock,
    pub window: Weak<Surface<Editor>>,
    pub closer: Option<Closer>,
}

/// What a running service is, for saying what waits for a restart.
fn running(state: State) -> bool {
    matches!(state, State::Starting | State::Active | State::Reloading | State::Stopping | State::Backoff)
}

impl Editor {
    /// The definition of `service`, as it is in the registry now.
    pub fn open(service: &str) -> Editor {
        let (values, missing) = match store::read(service) {
            Ok(Some(values)) => (values, None),
            Ok(None) => (Vec::new(), Some(format!("There is no service called {service}. It may have been deleted."))),
            Err(why) => (Vec::new(), Some(format!("Its definition could not be read: {why}."))),
        };
        let found = Definition::new(service, values);
        let (may_change, may_delete) = if missing.is_some() {
            (Err("there is nothing to change".into()), Err("there is nothing to delete".into()))
        } else {
            (store::changeable(service), store::deletable(service))
        };
        Editor::with(true, found, may_change, may_delete, missing)
    }

    /// A service not defined yet.
    pub fn new_service() -> Editor {
        Editor::with(false, Definition::new("", Vec::new()), store::creatable(), Err("it is not defined yet".into()), None)
    }

    fn with(made: bool, found: Definition, may_change: Result<(), String>, may_delete: Result<(), String>, missing: Option<String>) -> Editor {
        let mut editor = Editor {
            made,
            edited: found.clone(),
            found,
            wrong: BTreeMap::new(),
            problem: None,
            may_change,
            may_delete,
            missing,
            asking: None,
            saving: false,
            said: None,
            clock: words::Clock::Machine,
            window: Weak::new(),
            closer: None,
        };
        editor.check();
        editor
    }

    /// What the window is called.
    pub fn title(&self) -> String {
        match (self.made, self.found.text("DisplayName")) {
            (false, _) => "New service".into(),
            (true, Some(title)) => format!("{title} definition"),
            (true, None) => format!("{} definition", self.found.name),
        }
    }

    /// Puts what the definition holds in the fields.
    pub fn fill(&self, fields: &mut Fields) {
        for info in FIELDS {
            fields.set(&format!("f-{}", info.name), &self.edited.text(info.name).unwrap_or_default());
        }
    }

    fn check(&mut self) {
        self.problem = if self.missing.is_some() { None } else { self.edited.check().err() };
    }

    fn changed(&self) -> bool {
        !self.wrong.is_empty() || !changes(&self.found, &self.edited).is_empty() || (!self.made && !self.edited.name.is_empty())
    }

    fn fixed(&self) -> bool {
        self.may_change.is_err() || self.missing.is_some() || self.saving
    }

    fn close(&self) {
        if let Some(closer) = &self.closer {
            closer.close();
        }
    }

    /// Writes what changed, and says what came of it.
    fn save(&mut self) {
        if self.fixed() || !self.wrong.is_empty() || self.problem.is_some() {
            return;
        }
        let changed = changes(&self.found, &self.edited);
        if changed.is_empty() && self.made {
            return;
        }
        let create = !self.made;
        let (service, found) = (self.edited.name.clone(), self.found.values.clone());
        self.saving = true;
        self.said = None;
        let Some(window) = self.window.upgrade() else { return };
        // What a running service holds to until it is restarted, by what
        // the person calls it.
        let held: Vec<&'static str> = changed
            .iter()
            .map(|change| match change {
                peinit::client::Change::Set(value) => value.name.as_str(),
                peinit::client::Change::Unset(name) => name.as_str(),
            })
            .filter_map(service_field)
            .filter(|info| info.takes_effect != TakesEffect::Runtime)
            .map(|info| fields::words(info.name).0)
            .collect();
        std::thread::spawn(move || {
            let written = store::write(&service, &found, &changed, create);
            let state = written.as_ref().ok().and_then(|()| system::status(&service).ok()).map(|status| status.summary.state);
            window.update(|editor, fields| editor.saved(written, state, &held, fields));
        });
    }

    fn saved(&mut self, written: Result<(), String>, state: Option<State>, held: &[&str], fields: &mut Fields) {
        self.saving = false;
        if let Err(why) = written {
            self.said = Some((format!("It could not be saved: {why}."), true));
            return;
        }
        let created = !self.made;
        self.made = true;
        self.found = self.edited.clone();
        self.may_delete = store::deletable(&self.found.name);
        self.fill(fields);
        if let Some(window) = self.window.upgrade() {
            window.retitle(&self.title());
        }
        let title = self.found.text("DisplayName").unwrap_or_else(|| self.found.name.clone());
        let said = if created {
            match self.found.get("Triggers") {
                None => format!("{title} is defined. Nothing starts it but asking: Start, in Services Manager, does."),
                Some(_) => format!("{title} is defined, and its triggers start it."),
            }
        } else if state.is_some_and(running) && !held.is_empty() {
            format!("Saved. {title} is running, so {} {} effect when it is restarted.", list(held), if held.len() == 1 { "takes" } else { "take" })
        } else {
            "Saved.".into()
        };
        self.said = Some((said, false));
    }

    fn delete(&mut self) {
        self.asking = None;
        match store::delete(&self.found.name) {
            Ok(false) => self.close(),
            Ok(true) => {
                self.said = Some(("Its definition is deleted where this writes, but another layer of the registry defines it too, and that is there still.".into(), true))
            }
            Err(why) => self.said = Some((format!("It could not be deleted: {why}."), true)),
        }
    }

    /// One field: what it is called, its value, what it does, and what is
    /// wrong with it.
    fn field(&self, info: &FieldInfo) -> String {
        let (label, says) = fields::words(info.name);
        let id = format!("f-{}", info.name);
        let disabled = if self.fixed() { " disabled" } else { "" };
        let wrong = self.wrong.get(info.name).cloned().or_else(|| self.problem.as_ref().filter(|problem| problem.field == Some(info.name)).map(|problem| problem.message.clone()));
        let described = format!("h-{}{}", info.name, if wrong.is_some() { format!(" w-{}", info.name) } else { String::new() });
        let invalid = if wrong.is_some() { " aria-invalid=\"true\"" } else { "" };
        let default = info.default.map(|default| format!("Default: {default}")).unwrap_or_default();
        let control = match info.kind {
            FieldKind::Text => format!(
                "<input id=\"{id}\" name=\"{id}\" autocomplete=\"off\" spellcheck=\"false\" placeholder=\"{default}\" aria-describedby=\"{described}\"{invalid}{disabled}>",
                default = escape(&default),
            ),
            FieldKind::Number { unit } => format!(
                "<span class=\"number\"><input id=\"{id}\" name=\"{id}\" inputmode=\"numeric\" autocomplete=\"off\" placeholder=\"{default}\" aria-describedby=\"{described}\"{invalid}{disabled}>{unit}</span>",
                default = escape(info.default.unwrap_or("")),
                unit = unit.map(|unit| format!("<span class=\"unit\">{unit}</span>")).unwrap_or_default(),
            ),
            FieldKind::List => {
                let rows = self.edited.text(info.name).map_or(0, |text| text.lines().count()).max(1) + 1;
                format!(
                    "<textarea id=\"{id}\" name=\"{id}\" rows=\"{rows}\" spellcheck=\"false\" placeholder=\"One a line\" aria-describedby=\"{described}\"{invalid}{disabled}></textarea>"
                )
            }
            FieldKind::YesNo | FieldKind::Choice(_) => {
                let names: Vec<&str> = match info.kind {
                    FieldKind::Choice(names) => names.iter().map(|(_, name)| *name).collect(),
                    _ => vec!["yes", "no"],
                };
                let options: String = names.iter().map(|name| format!("<option value=\"{name}\">{}</option>", capital(name))).collect();
                format!(
                    "<select id=\"{id}\" name=\"{id}\" aria-describedby=\"{described}\"{invalid}{disabled}><option value=\"\">Default ({})</option>{options}</select>",
                    capital(info.default.unwrap_or("none")),
                )
            }
            FieldKind::Binary => format!(
                "<p class=\"value\" id=\"{id}\">{}</p>",
                if self.edited.get(info.name).is_some() { "Set for this service." } else { "Not set: the default applies." }
            ),
        };
        let when = match info.takes_effect {
            TakesEffect::Restart => " A change takes effect when it is restarted.",
            TakesEffect::NextStart => " A change takes effect when it next starts.",
            TakesEffect::Runtime => "",
        };
        let changed = if self.edited.get(info.name) != self.found.get(info.name) { " changed" } else { "" };
        let schedules = if info.name == "Triggers" { self.schedules() } else { String::new() };
        format!(
            "<div class=\"field{changed}\"><label for=\"{id}\">{label} <code>{name}</code></label>{control}\
             <p class=\"says\" id=\"h-{name}\">{says}{when}</p>{wrong}{schedules}</div>",
            name = info.name,
            wrong = wrong.map(|wrong| format!("<p class=\"wrong\" id=\"w-{}\">{}</p>", info.name, escape(&wrong))).unwrap_or_default(),
        )
    }

    /// When each timer schedule as written comes round next, so that one
    /// that never does is seen to before it is saved. A schedule that will
    /// not read is the check's to say.
    fn schedules(&self) -> String {
        let (now, zone) = self.clock.now();
        let after = u64::try_from(now.as_nanosecond()).unwrap_or(0);
        let each: String = self
            .edited
            .schedules(after)
            .into_iter()
            .filter_map(|(schedule, next)| {
                let schedule = escape(&schedule);
                match next.ok()? {
                    Some(next) => {
                        let next = jiff::Timestamp::from_nanosecond(i128::from(next)).ok()?;
                        Some(format!("<li><code>{schedule}</code> comes round next {}.</li>", escape(&words::when_at(next, now, &zone))))
                    }
                    None => Some(format!("<li class=\"bad\"><code>{schedule}</code> never comes round, so it never starts the service.</li>")),
                }
            })
            .collect();
        if each.is_empty() { String::new() } else { format!("<ul class=\"schedules\">{each}</ul>") }
    }
}

/// `items` as a list in a sentence.
fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

impl Live for Editor {
    fn render(&self, _: &Facts) -> String {
        let name = if self.made {
            format!("<p class=\"id\">{}</p>", escape(&self.found.name))
        } else {
            let wrong = self.problem.as_ref().filter(|problem| problem.field == Some("name")).map(|problem| format!("<p class=\"wrong\" id=\"w-name\">{}</p>", escape(&problem.message))).unwrap_or_default();
            format!(
                "<div class=\"field name\"><label for=\"name\">Name</label><input id=\"name\" name=\"name\" autocomplete=\"off\" spellcheck=\"false\" fx-autofocus \
                 placeholder=\"Such as web or backup-nightly\" aria-describedby=\"h-name\"{disabled}>\
                 <p class=\"says\" id=\"h-name\">What it is known by: letters, digits, dots, dashes and underscores. It cannot be changed once it is defined.</p>{wrong}</div>",
                disabled = if self.fixed() { " disabled" } else { "" },
            )
        };
        let banner = match (&self.missing, &self.may_change) {
            (Some(missing), _) => format!("<p class=\"banner bad\" role=\"alert\">{}</p>", escape(missing)),
            (None, Err(why)) if self.made => format!("<p class=\"banner\">You may read this definition, not change it: {}.</p>", escape(why)),
            (None, Err(why)) => format!("<p class=\"banner\">You may not define a service: {}.</p>", escape(why)),
            _ => String::new(),
        };
        let sections: String = if self.missing.is_some() {
            String::new()
        } else {
            FieldGroup::ALL
                .iter()
                .map(|group| {
                    let fields: String = FIELDS.iter().filter(|info| info.group == *group).map(|info| self.field(info)).collect();
                    format!("<section aria-labelledby=\"g-{group:?}\"><h2 id=\"g-{group:?}\">{}</h2>{fields}</section>", group.title())
                })
                .collect()
        };
        let others: Vec<String> = self
            .edited
            .others()
            .map(|value| {
                let what = if value.name.eq_ignore_ascii_case("LastTimerRun") { "when its timer last ran, kept by the service manager" } else { "not a field of a service definition" };
                format!("<li><code>{}</code> {what}</li>", escape(&value.name))
            })
            .collect();
        let others = if others.is_empty() { String::new() } else { format!("<section class=\"others\"><h2>Kept as they are</h2><ul>{}</ul></section>", others.concat()) };
        // What it comes to: what the person is asked, what came of the last
        // thing done, or whether peinit would take it.
        let status = if let Some((said, bad)) = &self.said {
            format!("<p class=\"outcome{}\" role=\"status\">{}</p>", if *bad { " bad" } else { "" }, escape(said))
        } else if self.saving {
            "<p class=\"outcome\" role=\"status\">Saving…</p>".into()
        } else if self.missing.is_some() || self.fixed() {
            "<p class=\"outcome\" role=\"status\"></p>".into()
        } else if let Some(field) = self.wrong.keys().next() {
            format!("<p class=\"outcome bad\" role=\"status\">{} is not right yet.</p>", fields::words(field).0)
        } else if let Some(problem) = &self.problem {
            format!("<p class=\"outcome bad\" role=\"status\">The service manager would not take this: {}</p>", escape(&problem.message))
        } else if self.changed() {
            "<p class=\"outcome\" role=\"status\">The service manager would take this.</p>".into()
        } else {
            "<p class=\"outcome\" role=\"status\">No changes.</p>".into()
        };
        let can_save = !self.fixed() && self.wrong.is_empty() && self.problem.is_none() && self.changed();
        let buttons = match self.asking {
            Some(Asking::Delete) => format!(
                "<p class=\"asking\">Delete the definition of {}? If it is running, it carries on until it stops, and is gone then.</p>\
                 <button type=\"button\" class=\"danger\" fx-click=\"delete-yes\" fx-autofocus>Delete</button><button type=\"button\" fx-click=\"keep\">Keep it</button>",
                escape(&self.found.name)
            ),
            Some(Asking::Close) => "<p class=\"asking\">Close without saving what has changed?</p>\
                 <button type=\"button\" class=\"danger\" fx-click=\"close-yes\" fx-autofocus>Close without saving</button><button type=\"button\" fx-click=\"keep\">Keep editing</button>"
                .into(),
            None => {
                let save = if self.fixed() && self.missing.is_none() && self.may_change.is_err() {
                    String::new()
                } else {
                    format!("<button type=\"button\" class=\"primary\" fx-click=\"save\" fx-key=\"Ctrl+S\"{}>{}</button>", if can_save { "" } else { " disabled" }, if self.made { "Save" } else { "Define" })
                };
                let revert = if self.made && self.may_change.is_ok() {
                    "<button type=\"button\" fx-click=\"revert\" title=\"Read it again from the registry\">Revert</button>"
                } else {
                    ""
                };
                let delete = match (&self.may_delete, self.made && self.missing.is_none()) {
                    (Ok(()), true) => "<button type=\"button\" fx-click=\"delete\">Delete…</button>".to_string(),
                    (Err(why), true) => format!("<button type=\"button\" disabled title=\"{}\">Delete…</button>", escape(&crate::words::sentence(why))),
                    _ => String::new(),
                };
                format!("{save}{revert}<span class=\"gap\"></span>{delete}<button type=\"button\" fx-click=\"close\">Close</button>")
            }
        };
        format!(
            "<div class=\"definition\">\
             <header><h1>{title}</h1>{name}</header>{banner}\
             <div class=\"form\">{sections}{others}</div>\
             <footer>{status}<div class=\"buttons\">{buttons}</div></footer>\
             </div>",
            title = escape(&self.title()),
        )
    }

    fn event(&mut self, name: &str, _: &Value, fields: &mut Fields) {
        match name {
            "save" => self.save(),
            "revert" => {
                let reread = Editor::open(&self.found.name);
                self.found = reread.found;
                self.edited = self.found.clone();
                self.missing = reread.missing;
                self.may_change = reread.may_change;
                self.may_delete = reread.may_delete;
                self.wrong.clear();
                self.said = None;
                self.check();
                self.fill(fields);
            }
            "delete" if self.may_delete.is_ok() => self.asking = Some(Asking::Delete),
            "delete-yes" if self.asking == Some(Asking::Delete) => self.delete(),
            "close" => {
                if self.changed() && self.may_change.is_ok() {
                    self.asking = Some(Asking::Close);
                } else {
                    self.close();
                }
            }
            "close-yes" => self.close(),
            "keep" => self.asking = None,
            _ => {}
        }
    }

    fn input(&mut self, name: &str, fields: &mut Fields) {
        if self.fixed() {
            return;
        }
        if name == "name" && !self.made {
            self.edited.name = fields.get("name").trim().to_string();
        } else if let Some(info) = name.strip_prefix("f-").and_then(service_field) {
            match self.edited.set(info.name, fields.get(name)) {
                Ok(()) => {
                    self.wrong.remove(info.name);
                }
                Err(why) => {
                    self.wrong.insert(info.name, why);
                }
            }
        } else {
            return;
        }
        self.said = None;
        self.check();
    }

    fn closing(&mut self, _: &mut Fields) -> bool {
        if self.changed() && self.may_change.is_ok() && self.missing.is_none() {
            self.asking = Some(Asking::Close);
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use peinit::client::{RawRegistryValue, RegistryValueType};

    use super::*;

    fn editor(values: &[(&str, &str)], may_change: Result<(), String>) -> Editor {
        let mut definition = Definition::new("web", Vec::new());
        for (field, text) in values {
            definition.set(field, text).unwrap();
        }
        Editor::with(true, definition, may_change, Ok(()), None)
    }

    fn shown(editor: &Editor) -> String {
        editor.render(&Facts { views: 1, fields: &Fields::default() })
    }

    fn typed(editor: &mut Editor, field: &str, text: &str) {
        let mut fields = Fields::default();
        let name = format!("f-{field}");
        fields.set(&name, text);
        editor.input(&name, &mut fields);
    }

    #[test]
    fn each_timer_says_when_its_schedule_comes_round_and_one_that_never_does_says_so() {
        let mut editor = editor(&[("ImagePath", "/usr/bin/web")], Ok(()));
        // Saturday 3 October 2026, 12:20 UTC, on the machine's own clock: a
        // schedule with no zone is in the machine's zone, which a build root
        // (often with no zone database) and a desk need not agree on, so the
        // clock is read in the same one.
        editor.clock = words::Clock::Fixed("2026-10-03T12:20:00Z".parse().unwrap(), jiff::tz::TimeZone::system());
        assert!(!shown(&editor).contains("class=\"schedules\""));
        typed(&mut editor, "Triggers", "boot\ntimer:*-*-* 02:00:00\ntimer:*-02-30");
        let html = shown(&editor);
        let said = html.split("<ul class=\"schedules\">").nth(1).unwrap_or(&html);
        assert!(said.starts_with("<li><code>*-*-* 02:00:00</code> comes round next ") && said.contains(" at 02:00.</li>"), "{said}");
        assert!(said.contains("<li class=\"bad\"><code>*-02-30</code> never comes round, so it never starts the service.</li></ul>"), "{said}");
        // It is said, not refused: peinit takes it, and arms the rest.
        assert!(html.contains("fx-click=\"save\" fx-key=\"Ctrl+S\">Save"));
    }

    #[test]
    fn every_field_is_shown_by_group_with_what_it_does_and_its_default() {
        let editor = editor(&[("ImagePath", "/usr/bin/web"), ("DisplayName", "Web")], Ok(()));
        let html = shown(&editor);
        assert_eq!(html.matches("<div class=\"field").count(), FIELDS.len());
        assert!(html.contains("<h1>Web definition</h1>") && html.contains("<p class=\"id\">web</p>"));
        assert!(html.contains("<label for=\"f-ImagePath\">Program <code>ImagePath</code></label>"));
        assert!(html.contains("The program to run, as a path from the root. A change takes effect when it is restarted."));
        assert!(html.contains("placeholder=\"30\""), "a number's default is its placeholder");
        assert!(html.contains("<option value=\"\">Default (Simple)</option><option value=\"Simple\">Simple</option><option value=\"Oneshot\">Oneshot</option>"));
        assert!(html.contains("<h2 id=\"g-Dependencies\">Dependencies</h2>"));
        assert!(html.contains("No changes.") && html.contains("fx-click=\"save\" fx-key=\"Ctrl+S\" disabled>Save"));
    }

    #[test]
    fn what_is_wrong_is_said_beside_its_field_and_it_cannot_be_saved() {
        let mut editor = editor(&[("ImagePath", "/usr/bin/web")], Ok(()));
        typed(&mut editor, "StopTimeout", "ten");
        let html = shown(&editor);
        assert!(html.contains("<p class=\"wrong\" id=\"w-StopTimeout\">StopTimeout is a whole number, from 0 to 4294967295.</p>"));
        assert!(html.contains("aria-describedby=\"h-StopTimeout w-StopTimeout\" aria-invalid=\"true\""));
        assert!(html.contains("Stop timeout is not right yet.") && html.contains(" disabled>Save"));
        typed(&mut editor, "StopTimeout", "20");
        typed(&mut editor, "ImagePath", "web");
        let html = shown(&editor);
        assert!(html.contains("<p class=\"wrong\" id=\"w-ImagePath\">ImagePath is a path from the root"));
        assert!(html.contains("The service manager would not take this: "));
        typed(&mut editor, "ImagePath", "/usr/bin/web");
        let html = shown(&editor);
        assert!(html.contains("The service manager would take this.") && html.contains("fx-key=\"Ctrl+S\">Save"));
        assert!(html.contains("<div class=\"field changed\"><label for=\"f-StopTimeout\">"));
        // Closing now asks first.
        assert!(!editor.closing(&mut Fields::default()));
        assert!(shown(&editor).contains("Close without saving what has changed?"));
    }

    #[test]
    fn a_definition_that_may_not_be_changed_is_shown_fixed_and_says_why() {
        let mut editor = editor(&[("ImagePath", "/usr/bin/web")], Err("you are not allowed to change its definition".into()));
        editor.edited.values.push(RawRegistryValue { name: "LastTimerRun".into(), value_type: RegistryValueType::Other(11), data: vec![0; 8] });
        let html = shown(&editor);
        assert!(html.contains("You may read this definition, not change it: you are not allowed to change its definition."));
        assert!(html.contains("<input id=\"f-ImagePath\" name=\"f-ImagePath\" autocomplete=\"off\" spellcheck=\"false\" placeholder=\"\" aria-describedby=\"h-ImagePath\" disabled>"));
        assert!(!html.contains(">Save<") && !html.contains(">Revert<"));
        assert!(html.contains("<code>LastTimerRun</code> when its timer last ran, kept by the service manager"));
        typed(&mut editor, "ImagePath", "/elsewhere");
        assert_eq!(editor.edited.text("ImagePath").as_deref(), Some("/usr/bin/web"), "nothing typed counts");
        assert!(editor.closing(&mut Fields::default()));
    }

    #[test]
    fn a_new_service_needs_a_name_and_a_program() {
        let mut editor = Editor::with(false, Definition::new("", Vec::new()), Ok(()), Err("it is not defined yet".into()), None);
        let html = shown(&editor);
        assert!(html.contains("<h1>New service</h1>") && html.contains("<input id=\"name\" name=\"name\""));
        assert!(html.contains(">Define</button>") && !html.contains("Delete…"));
        let mut fields = Fields::default();
        fields.set("name", "my web");
        editor.input("name", &mut fields);
        assert!(shown(&editor).contains("<p class=\"wrong\" id=\"w-name\">“my web” is not a service name"));
        fields.set("name", "web");
        editor.input("name", &mut fields);
        assert!(shown(&editor).contains("<p class=\"wrong\" id=\"w-ImagePath\">It needs a program to run: ImagePath is not set.</p>"));
        typed(&mut editor, "ImagePath", "/usr/bin/web");
        assert!(shown(&editor).contains("fx-key=\"Ctrl+S\">Define</button>"));
        assert_eq!(list(&["Runs as", "Privileges", "Program"]), "Runs as, Privileges and Program");
    }
}
