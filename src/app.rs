use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub enum ExitReason {
    Stopped,
    RestartRequested,
}

pub fn run() -> io::Result<ExitReason> {
    let (configuration_path, configuration) = crate::config::load()?;
    println!(
        "DNS service starting bind={} upstream={} records={}",
        configuration.bind,
        configuration
            .upstream
            .map_or_else(|| "disabled".to_owned(), |value| value.to_string()),
        configuration.records.len()
    );
    let restart_requested = Arc::new(AtomicBool::new(false));
    let control = crate::control::start(configuration_path, Arc::clone(&restart_requested))?;
    let dns_result = crate::dns::run(configuration, Arc::clone(&restart_requested));
    restart_requested.store(true, Ordering::Release);
    let control_result = control
        .join()
        .map_err(|_| io::Error::other("DNS control worker panicked"))?;
    dns_result?;
    control_result?;
    if restart_requested.load(Ordering::Acquire) {
        Ok(ExitReason::RestartRequested)
    } else {
        Ok(ExitReason::Stopped)
    }
}
