mod app;
mod cli;
mod config;
mod control;
mod dns;

const PRODUCT_BANNER: &str = concat!(
    "Autobricks DNS ",
    env!("AUTOBRICKS_PRODUCT_VERSION"),
    " (C) Copyright 2026 Autobricks, Co. All Rights Reserved."
);

fn main() {
    let mode = match cli::parse() {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if let cli::Mode::Command(command) = mode {
        if let Err(error) = cli::execute(command) {
            eprintln!("level=ERROR, component=autobricks-dns, result=failure, detail={error}");
            std::process::exit(1);
        }
        return;
    }
    println!("{PRODUCT_BANNER}");
    match app::run() {
        Ok(app::ExitReason::Stopped) => {}
        Ok(app::ExitReason::RestartRequested) => {
            println!("DNS_CONTROL_RESTART_REQUESTED");
        }
        Err(error) => {
            eprintln!("level=ERROR, component=autobricks-dns, result=failure, detail={error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_banner_uses_global_product_version() {
        assert!(PRODUCT_BANNER.starts_with(concat!(
            "Autobricks DNS ",
            env!("AUTOBRICKS_PRODUCT_VERSION")
        )));
    }
}
