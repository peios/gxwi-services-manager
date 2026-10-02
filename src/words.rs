//! What peinit's states, causes and commands are called where a person reads
//! them.

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
    fn every_command_is_named_by_what_its_button_sends() {
        for command in [Command::Start, Command::Stop, Command::Restart, Command::Reload, Command::Reset] {
            assert_eq!(named(verb(command)), Some(command));
        }
    }
}
