//! What peinit's states, causes and commands are called where a person reads
//! them.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use peinit::client::{Command, Health, State};

/// A state, as the State column has it.
pub fn state(state: State) -> &'static str {
    match state {
        State::Inactive => "Stopped",
        State::Starting => "Starting",
        State::Active => "Running",
        State::Reloading => "Reloading",
        State::Stopping => "Stopping",
        State::Completed => "Finished",
        State::Backoff => "Waiting to restart",
        State::Failed => "Failed",
        State::Abandoned => "Abandoned",
        State::Skipped => "Skipped",
    }
}

/// What a state looks like: the class the dot beside it is drawn with.
pub fn tone(state: State) -> &'static str {
    match state {
        State::Active | State::Completed => "good",
        State::Starting | State::Reloading | State::Stopping | State::Backoff => "busy",
        State::Failed | State::Abandoned => "bad",
        State::Inactive | State::Skipped => "idle",
    }
}

pub fn health(health: Health) -> &'static str {
    match health {
        Health::Unknown => "Not checked yet",
        Health::Healthy => "Healthy",
        Health::Unhealthy => "Unhealthy",
    }
}

/// Why a service is in its state, by the wire's name for the cause. A cause
/// this does not know is shown as peinit names it.
pub fn cause(cause: &str) -> String {
    match cause {
        "explicit_start" => "started when asked",
        "dependency_start" => "started because another service needs it",
        "restart_policy" => "restarted after it stopped",
        "binds_to_recovery" => "started again with a service it is bound to",
        "explicit_stop" => "stopped when asked",
        "explicit_reload" => "reloaded when asked",
        "explicit_reset" => "reset when asked",
        "conflict_eviction" => "stopped because a service it conflicts with started",
        "binds_to_propagation" => "stopped because a service it is bound to stopped",
        "timer" => "started by its timer",
        "shutdown_wave" => "stopped for the shutdown",
        "process_crash" => "its process crashed",
        "clean_exit" => "its process ended",
        "clean_exit_restart" => "its process ended and is started again",
        "readiness_timeout" => "it did not become ready in time",
        "watchdog_timeout" => "it stopped answering its watchdog",
        "health_check_failure" => "its health check failed",
        "pre_hook_failure" => "a command run before it failed",
        "parent_setup_failure" => "what it needs could not be set up",
        "pre_exec_failure" => "its program could not be started",
        "dependency_failure" => "a service it needs failed",
        "restart_budget_exhausted" => "it failed too many times in a row",
        "cycle_detected" => "its dependencies go round in a circle",
        "validation_error" => "its definition is not valid",
        "assertion_error" => "something it asserts was not so",
        "condition_skipped" => "a condition for starting it was not met",
        "tty_unavailable" => "its terminal was not free",
        "process_unkillable" => "its process could not be killed",
        "internal_error" => "the service manager went wrong",
        other => return other.replace('_', " "),
    }
    .into()
}

/// Why an operation failed, as peinit says it: a cause by its type's name
/// and what more there is to say (`ProcessCrash: exit code 1`). The cause is
/// said in words, as a state's is; anything else is left as it is.
pub fn failure(error: &str) -> String {
    let (name, more) = error.split_once(": ").map_or((error, None), |(name, more)| (name, Some(more)));
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return error.to_string();
    }
    // ProcessCrash is process_crash on the wire.
    let wire: String = name
        .chars()
        .enumerate()
        .flat_map(|(at, c)| (c.is_ascii_uppercase() && at > 0).then_some('_').into_iter().chain(std::iter::once(c.to_ascii_lowercase())))
        .collect();
    let said = cause(&wire);
    if said == wire.replace('_', " ") {
        return error.to_string();
    }
    match more {
        Some(more) => format!("{said} ({more})"),
        None => said,
    }
}

/// What asked for an operation, by the wire's name for its source.
pub fn source(source: &str) -> String {
    match source {
        "admin" => "asked for by someone",
        "boot" => "for the boot",
        "shutdown" => "for the shutdown",
        "dependency_propagation" => "for a service that depends on it",
        "restart_policy" => "by its restart policy",
        "timer" => "by its timer",
        "binds_to_recovery" | "binds_to_propagation" => "with a service it is bound to",
        "conflict_resolution" => "for a service it conflicts with",
        "on_failure" => "because another service failed",
        "tty_release" => "when its terminal was let go",
        other => return other.replace('_', " "),
    }
    .into()
}

/// A command as a button names it.
pub fn command(command: Command) -> &'static str {
    match command {
        Command::Start => "Start",
        Command::Stop => "Stop",
        Command::Restart => "Restart",
        Command::Reload => "Reload",
        Command::Reset => "Reset",
    }
}

/// A command as a sentence has it: "may not *start* it".
pub fn verb(command: Command) -> &'static str {
    match command {
        Command::Start => "start",
        Command::Stop => "stop",
        Command::Restart => "restart",
        Command::Reload => "reload",
        Command::Reset => "reset",
    }
}

/// What a command has done: "it was *started*".
pub fn done(command: Command) -> &'static str {
    match command {
        Command::Start => "started",
        Command::Stop => "stopped",
        Command::Restart => "restarted",
        Command::Reload => "reloaded",
        Command::Reset => "reset",
    }
}

/// What a command is doing: "*Starting* timed".
pub fn doing(command: Command) -> &'static str {
    match command {
        Command::Start => "Starting",
        Command::Stop => "Stopping",
        Command::Restart => "Restarting",
        Command::Reload => "Reloading",
        Command::Reset => "Resetting",
    }
}

/// A command by the name its button sends.
pub fn named(name: &str) -> Option<Command> {
    match name {
        "start" => Some(Command::Start),
        "stop" => Some(Command::Stop),
        "restart" => Some(Command::Restart),
        "reload" => Some(Command::Reload),
        "reset" => Some(Command::Reset),
        _ => None,
    }
}

/// A reason, as a sentence of its own: "you may not change it" is "You may
/// not change it."
pub fn sentence(reason: &str) -> String {
    let mut chars = reason.chars();
    let first: String = chars.next().map(|first| first.to_uppercase().collect()).unwrap_or_default();
    format!("{first}{}.", chars.as_str())
}

/// A length of time, roughly, as a person says it.
pub fn duration(seconds: u64) -> String {
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86_400);
    match () {
        _ if seconds < 60 => format!("{seconds} s"),
        _ if minutes < 60 => format!("{minutes} min"),
        _ if hours < 24 => format!("{hours} h {} min", minutes % 60),
        _ => format!("{days} {} {} h", if days == 1 { "day" } else { "days" }, hours % 24),
    }
}

/// The time, and the zone, that times are said against: the machine's, or
/// in a test, fixed ones.
#[derive(Clone)]
pub enum Clock {
    Machine,
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed(Timestamp, TimeZone),
}

impl Clock {
    pub fn now(&self) -> (Timestamp, TimeZone) {
        match self {
            Clock::Machine => (Timestamp::now(), TimeZone::system()),
            Clock::Fixed(now, zone) => (*now, zone.clone()),
        }
    }
}

/// A time peinit gives (RFC 3339, in UTC) as a person says it on this
/// machine's clock, against now: "today at 14:00", "tomorrow at 02:00",
/// "Monday at 02:00", "5 Oct at 02:00", "5 Oct 2027 at 02:00". Seconds only
/// where there are some. A time that will not read is left as it is.
pub fn when(at: &str, now: Timestamp, zone: &TimeZone) -> String {
    let Ok(at) = at.parse::<Timestamp>() else { return at.to_string() };
    let (at, now) = (at.to_zoned(zone.clone()), now.to_zoned(zone.clone()));
    let clock = if at.second() == 0 { at.strftime("%H:%M") } else { at.strftime("%H:%M:%S") };
    let days = (at.date() - now.date()).get_days();
    let day = match days {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        2..=6 => at.strftime("%A").to_string(),
        _ if at.year() == now.year() => at.strftime("%-d %b").to_string(),
        _ => at.strftime("%-d %b %Y").to_string(),
    };
    format!("{day} at {clock}")
}

/// How long until `at`, roughly, where it is still to come.
pub fn until(at: &str, now: Timestamp) -> Option<String> {
    let at = at.parse::<Timestamp>().ok()?;
    let seconds = at.as_second() - now.as_second();
    (seconds > 0).then(|| duration(seconds as u64))
}

/// `said` with its first letter a capital, to start a line with.
pub fn capital(said: &str) -> String {
    let mut chars = said.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Who a SID is, where it is one of the principals services run as. Anyone
/// else is shown as their SID.
pub fn principal(sid: &str) -> String {
    match sid {
        "S-1-5-18" => "Local System".into(),
        "S-1-5-19" => "Local Service".into(),
        "S-1-5-20" => "Network Service".into(),
        sid if sid.starts_with("S-1-5-80-") => format!("its own account ({sid})"),
        sid => sid.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_said_roughly() {
        assert_eq!(duration(5), "5 s");
        assert_eq!(duration(125), "2 min");
        assert_eq!(duration(3 * 3600 + 7 * 60), "3 h 7 min");
        assert_eq!(duration(86_400 + 3600), "1 day 1 h");
        assert_eq!(duration(3 * 86_400), "3 days 0 h");
    }

    #[test]
    fn a_cause_is_said_in_words_and_an_unknown_one_as_peinit_names_it() {
        assert_eq!(cause("process_crash"), "its process crashed");
        assert_eq!(cause("some_new_cause"), "some new cause");
    }

    #[test]
    fn a_failure_is_said_by_its_cause_with_the_rest_as_peinit_says_it() {
        assert_eq!(failure("ProcessCrash: exit code 1"), "its process crashed (exit code 1)");
        assert_eq!(failure("ReadinessTimeout"), "it did not become ready in time");
        assert_eq!(failure("SomethingNew: detail"), "SomethingNew: detail");
        assert_eq!(failure("the socket went away"), "the socket went away");
    }

    #[test]
    fn a_time_is_said_on_the_local_clock_by_how_far_off_its_day_is() {
        // Saturday 3 October 2026, 13:20 in UTC; 14:20 an hour east of it.
        let now: Timestamp = "2026-10-03T13:20:00Z".parse().unwrap();
        let utc = TimeZone::UTC;
        let east = TimeZone::fixed(jiff::tz::offset(1));
        assert_eq!(when("2026-10-03T14:00:00.000000000Z", now, &utc), "today at 14:00");
        assert_eq!(when("2026-10-03T14:00:00.000000000Z", now, &east), "today at 15:00");
        assert_eq!(when("2026-10-03T23:30:00.000000000Z", now, &east), "tomorrow at 00:30");
        assert_eq!(when("2026-10-04T02:07:12.000000000Z", now, &utc), "tomorrow at 02:07:12");
        assert_eq!(when("2026-10-02T02:00:00.000000000Z", now, &utc), "yesterday at 02:00");
        assert_eq!(when("2026-10-05T02:00:00.000000000Z", now, &utc), "Monday at 02:00");
        assert_eq!(when("2026-09-01T00:00:00.000000000Z", now, &utc), "1 Sep at 00:00");
        assert_eq!(when("2027-01-01T00:00:00.000000000Z", now, &utc), "1 Jan 2027 at 00:00");
        assert_eq!(when("whenever", now, &utc), "whenever");
        assert_eq!(until("2026-10-03T14:00:00.000000000Z", now).as_deref(), Some("40 min"));
        assert_eq!(until("2026-10-03T13:00:00.000000000Z", now), None);
        assert_eq!(capital("tomorrow at 02:00"), "Tomorrow at 02:00");
    }

    #[test]
    fn every_command_is_named_by_what_its_button_sends() {
        for command in [Command::Start, Command::Stop, Command::Restart, Command::Reload, Command::Reset] {
            assert_eq!(named(verb(command)), Some(command));
        }
    }
}
