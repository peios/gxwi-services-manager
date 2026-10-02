//! What each field of a definition is called in the window, and a line
//! saying what it does. The fields themselves, their kinds and defaults,
//! are peinit's (`peinit::client::FIELDS`); these are the words.

/// The words for `field`: what it is called, and what it does.
pub fn words(field: &str) -> (&'static str, &'static str) {
    match field {
        "DisplayName" => ("Display name", "What it is called in lists, such as Services Manager's."),
        "Description" => ("Description", "What it does, in a sentence."),
        "ImagePath" => ("Program", "The program to run, as a path from the root."),
        "Arguments" => ("Arguments", "Given to the program, one a line."),
        "WorkingDirectory" => ("Working directory", "Where the program starts."),
        "Environment" => ("Environment", "Variables added to its environment, as NAME=value, one a line."),
        "RuntimeDirectories" => ("Runtime directories", "Made under /run before it starts, for it, SYSTEM and Administrators; one a line."),
        "LimitNOFILE" => ("Open files", "The most files it may have open at once."),
        "LimitCORE" => ("Core dump size", "The largest core dump it may leave."),
        "TTYPath" => ("Terminal", "A terminal to run on, such as /dev/tty1. What it writes there is not logged."),
        "TTYPrecedence" => ("Terminal precedence", "Which service has the terminal when several want it at once: the highest."),
        "Type" => ("Type", "Simple runs until it is stopped. Oneshot runs to its end, and is done."),
        "Readiness" => ("Ready when", "Notify: when it says READY=1. Alive: as soon as it is running."),
        "RemainAfterExit" => ("Stays completed", "A Oneshot that ended well stays Completed rather than Inactive."),
        "SuccessExitCodes" => ("Successful exit codes", "Exit codes besides 0 that mean it ended well, one a line."),
        "Triggers" => ("Started by", "One a line: boot, boot:settled, tty:released, or timer: and a schedule. None: only when asked."),
        "Disabled" => ("Disabled", "Its triggers do not start it. It can still be started by hand."),
        "SafeMode" => ("Starts in Safe mode", "Started when the machine starts in Safe mode."),
        "Conditions" => ("Conditions", "Checked before it starts, one a line; one not met skips it."),
        "Asserts" => ("Asserts", "Checked before it starts, one a line; one not met fails it."),
        "PreStartCheckTimeout" => ("Check timeout", "How long checking its conditions and asserts may take."),
        "TimerPersistent" => ("Catches up", "A timer missed while the machine was off runs when it next starts."),
        "TimerJitter" => ("Timer jitter", "Up to this much is added to each timer, at random."),
        "Identity" => ("Runs as", "A principal, by name or SID."),
        "RequiredPrivileges" => ("Privileges", "The only privileges it keeps, one a line. None: all it would have."),
        "HookIdentity" => ("Commands run as", "Who the commands run before and after it starts run as."),
        "Requires" => ("Requires", "Services it cannot run without, one a line, with a readiness level after a colon if it needs one (netd:routed)."),
        "Wants" => ("Wants", "Services started with it, which it does without if they fail."),
        "BindsTo" => ("Bound to", "Services it stops with."),
        "Conflicts" => ("Conflicts with", "Services that cannot run while it does."),
        "Provides" => ("Provides", "Roles it fills, so that others can depend on the role."),
        "OnFailure" => ("When it fails, start", "A service started when it fails."),
        "ErrorControl" => ("Error control", "Critical: if it cannot be kept running, the machine restarts."),
        "RestartPolicy" => ("Restarted", "When it is started again after it stops by itself."),
        "RestartMaxRetries" => ("Restart attempts", "Restarts in a row before it is left Failed."),
        "RestartWindow" => ("Restart window", "Running this long resets the count of restarts."),
        "RestartDelay" => ("Restart delay", "The first wait before a restart. It doubles each time, up to 60 seconds."),
        "HealthCheck" => ("Health check", "A command run every so often to see that it is well."),
        "HealthCheckInterval" => ("Health check interval", "How often the health check runs."),
        "HealthCheckTimeout" => ("Health check timeout", "How long the health check may take before it counts as failed."),
        "HealthCheckRetries" => ("Health check retries", "Failed checks in a row before it is unhealthy."),
        "WatchdogTimeout" => ("Watchdog", "How often it must say WATCHDOG=1, or it is restarted. 0: it need not."),
        "ExecStartPre" => ("Before it starts", "Commands run in turn before it starts, one a line. One that fails stops it starting."),
        "ExecStartPost" => ("Once it has started", "Commands run once it is ready, one a line. One that fails is logged."),
        "ExecReload" => ("Reloaded with", "A command, or signal: and a signal's name."),
        "StartTimeout" => ("Start timeout", "How long starting may take, from the first command before it to its being ready."),
        "StopTimeout" => ("Stop timeout", "How long it has to stop before it is killed."),
        "NotifyAccess" => ("Notifications from", "Which of its processes may tell the service manager things."),
        "FdStoreMax" => ("Stored descriptors", "How many file descriptors the service manager keeps for it across restarts."),
        "ServiceSecurity" => ("Who may control it", "Changed with Who may control it… in Services Manager."),
        _ => ("", ""),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_field_has_words() {
        for field in peinit::client::FIELDS {
            let (label, says) = super::words(field.name);
            assert!(!label.is_empty() && says.ends_with('.'), "{}", field.name);
        }
    }
}
