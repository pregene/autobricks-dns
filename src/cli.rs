use crate::config::{DnsRecord, RecordType};
use std::env;
use std::io;

pub enum Mode {
    Server,
    Command(Command),
}

pub enum Command {
    Help,
    List,
    Add(DnsRecord),
    Delete { name: String },
    Restart,
}

pub fn parse() -> io::Result<Mode> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let Some(command) = arguments.first().map(String::as_str) else {
        return Ok(Mode::Server);
    };
    match command {
        "list" if arguments.len() == 1 => Ok(Mode::Command(Command::List)),
        "restart" if arguments.len() == 1 => Ok(Mode::Command(Command::Restart)),
        "add" => Ok(Mode::Command(Command::Add(DnsRecord {
            name: required(&arguments, "--name")?,
            ip: required(&arguments, "--ip")?.parse().map_err(|error| {
                io::Error::other(format!("invalid --ip value: {error}"))
            })?,
            record_type: record_type(&required(&arguments, "--type")?)?,
        }))),
        "delete" => Ok(Mode::Command(Command::Delete {
            name: required(&arguments, "--name")?,
        })),
        "help" | "--help" | "-h" => Ok(Mode::Command(Command::Help)),
        _ => Err(io::Error::other(usage())),
    }
}

fn required(arguments: &[String], flag: &str) -> io::Result<String> {
    let Some(position) = arguments.iter().position(|value| value == flag) else {
        return Err(io::Error::other(format!("missing required {flag}\n{}", usage())));
    };
    arguments
        .get(position + 1)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| io::Error::other(format!("missing value for {flag}\n{}", usage())))
}

fn record_type(value: &str) -> io::Result<RecordType> {
    match value {
        "A" => Ok(RecordType::A),
        "AAAA" => Ok(RecordType::Aaaa),
        _ => Err(io::Error::other("--type must be A or AAAA")),
    }
}

fn usage() -> &'static str {
    concat!(
        "Autobricks DNS ",
        env!("AUTOBRICKS_PRODUCT_VERSION"),
        r#"
Serve configured internal A/AAAA records and forward other names to upstream DNS.

Usage:
  autobricks-dns
  autobricks-dns <command> [options]

Commands:
  (no command)  Start the DNS service using the INI configuration.
  list          Print configured records as JSON.
  add           Add an A or AAAA record (requires --name, --ip, and --type).
  delete        Delete all A and AAAA records for a name (requires --name).
  restart       Request a clean service exit; a service manager must restart it.
  help          Show this help. Aliases: --help, -h.

Record options:
  --name <name>      Exact DNS name, matched case-insensitively; no wildcards.
  --ip <address>     IPv4 address for A, or IPv6 address for AAAA.
  --type <A|AAAA>    Record type. Other local record types are not supported.

Environment:
  AUTOBRICKS_DNS_CONFIG  INI file path used when starting the service.
                        Default: config/autobricks-dns.ini (relative to cwd).
  AUTOBRICKS_DNS_SOCKET  Unix control socket path for the service and commands.
                        Default: /run/autobricks-dns/autobricks-dns.sock

Examples:
  autobricks-dns
  autobricks-dns list
  autobricks-dns add --name api.example.internal --ip 10.10.0.10 --type A
  autobricks-dns add --name api.example.internal --ip fd00::10 --type AAAA
  autobricks-dns delete --name api.example.internal
  autobricks-dns restart

Notes:
  Management commands require a running service and access to its control socket.
  Add/delete save the configuration; restart the service to apply DNS changes.
  Adding the same name/type/IP is a no-op; a different IP is rejected.
  Without a service manager, start the service manually after restart exits.
  Clients must use this server as their DNS resolver to receive local overrides.
  See README.md for configuration and INSTALL.md for service installation."#,
    )
}

pub fn execute(command: Command) -> io::Result<()> {
    match command {
        Command::Help => println!("{}", usage()),
        Command::List => {
            let records = crate::control::list()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&records).map_err(io::Error::other)?
            );
        }
        Command::Add(record) => crate::control::add(record)?,
        Command::Delete { name } => crate::control::delete(name)?,
        Command::Restart => crate::control::restart()?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_record_type() -> io::Result<()> {
        assert_eq!(record_type("A")?, RecordType::A);
        assert!(record_type("MX").is_err());
        Ok(())
    }
}
