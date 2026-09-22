use crate::config::{DnsRecord, RecordType};
use std::env;
use std::io;

pub enum Mode {
    Server,
    Command(Command),
}

pub enum Command {
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
        "help" | "--help" | "-h" => Err(usage()),
        _ => Err(usage()),
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

fn usage() -> io::Error {
    io::Error::other(
        "usage:\n  autobricks-dns\n  autobricks-dns list\n  autobricks-dns add --name <name> --ip <address> --type <A|AAAA>\n  autobricks-dns delete --name <name>\n  autobricks-dns restart",
    )
}

pub fn execute(command: Command) -> io::Result<()> {
    match command {
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