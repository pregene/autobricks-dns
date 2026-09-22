use ini::Ini;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const DEFAULT_CONFIG: &str = "config/autobricks-dns.ini";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DnsConfig {
    pub bind: SocketAddr,
    pub upstream: Option<SocketAddr>,
    pub records: Vec<DnsRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DnsRecord {
    pub name: String,
    #[serde(rename = "type")]
    pub record_type: RecordType,
    pub ip: IpAddr,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum RecordType {
    A,
    #[serde(rename = "AAAA")]
    Aaaa,
}

impl RecordType {
    pub const fn query_type(self) -> u16 {
        match self {
            Self::A => 1,
            Self::Aaaa => 28,
        }
    }

    const fn ini_name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
        }
    }
}

pub fn load() -> io::Result<(PathBuf, DnsConfig)> {
    let path = std::env::var_os("AUTOBRICKS_DNS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    let configuration = load_path(&path)?;
    Ok((path, configuration))
}

pub fn load_path(path: &Path) -> io::Result<DnsConfig> {
    let mut configuration = parse_ini(&fs::read_to_string(path)?)?;
    validate(&mut configuration)?;
    Ok(configuration)
}

pub fn save(path: &Path, configuration: &mut DnsConfig) -> io::Result<()> {
    validate(configuration)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("DNS configuration parent is missing"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::other("DNS configuration filename is invalid"))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = write_atomic(&temporary, path, encode_ini(configuration).as_bytes());
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn parse_ini(source: &str) -> io::Result<DnsConfig> {
    let document = Ini::load_from_str(source).map_err(io::Error::other)?;
    let server = document
        .section(Some("server"))
        .ok_or_else(|| io::Error::other("DNS INI configuration requires a [server] section"))?;
    if server.iter().any(|(key, _)| key != "bind" && key != "upstream") {
        return Err(io::Error::other(
            "DNS [server] section contains an unknown key",
        ));
    }
    let bind = server
        .get("bind")
        .ok_or_else(|| io::Error::other("DNS [server] section requires bind"))?
        .parse()
        .map_err(|error| io::Error::other(format!("invalid DNS bind address: {error}")))?;
    let upstream = server
        .get("upstream")
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse()
                .map_err(|error| io::Error::other(format!("invalid DNS upstream address: {error}")))
        })
        .transpose()?;
    let mut records = Vec::new();
    for (section, properties) in document.iter() {
        let Some(section) = section else {
            if properties.is_empty() {
                continue;
            }
            return Err(io::Error::other(
                "DNS INI configuration does not allow top-level keys",
            ));
        };
        if section == "server" {
            continue;
        }
        for (record_type, value) in properties {
            let record_type = match record_type {
                "A" => RecordType::A,
                "AAAA" => RecordType::Aaaa,
                _ => {
                    return Err(io::Error::other(format!(
                        "DNS INI section [{section}] only accepts A and AAAA keys"
                    )));
                }
            };
            let ip = value
                .parse()
                .map_err(|error| io::Error::other(format!("invalid DNS record IP: {error}")))?;
            records.push(DnsRecord {
                name: section.to_owned(),
                record_type,
                ip,
            });
        }
    }
    Ok(DnsConfig {
        bind,
        upstream,
        records,
    })
}

fn encode_ini(configuration: &DnsConfig) -> String {
    let mut output = format!("[server]\nbind = {}\n", configuration.bind);
    if let Some(upstream) = configuration.upstream {
        output.push_str(&format!("upstream = {upstream}\n"));
    }
    let mut records_by_name = BTreeMap::new();
    for record in &configuration.records {
        records_by_name
            .entry(record.name.as_str())
            .or_insert_with(Vec::new)
            .push(record);
    }
    for (name, records) in records_by_name {
        output.push_str(&format!("\n[{name}]\n"));
        for record in records {
            output.push_str(&format!("{} = {}\n", record.record_type.ini_name(), record.ip));
        }
    }
    output
}

#[cfg(unix)]
fn write_atomic(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(temporary)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temporary, destination)?;
    fs::File::open(
        destination
            .parent()
            .ok_or_else(|| io::Error::other("DNS configuration parent is missing"))?,
    )?
    .sync_all()
}

#[cfg(not(unix))]
fn write_atomic(_temporary: &Path, _destination: &Path, _bytes: &[u8]) -> io::Result<()> {
    Err(io::Error::other(
        "atomic DNS configuration storage is unavailable on this platform",
    ))
}

pub(crate) fn validate(configuration: &mut DnsConfig) -> io::Result<()> {
    if configuration.upstream == Some(configuration.bind) {
        return Err(io::Error::other(
            "DNS upstream must not point to the DNS service bind",
        ));
    }
    let mut names = BTreeSet::new();
    for record in &mut configuration.records {
        record.name.make_ascii_lowercase();
        if !valid_name(&record.name) {
            return Err(io::Error::other(format!(
                "invalid DNS record name {}",
                record.name
            )));
        }
        if !names.insert((record.name.clone(), record.record_type)) {
            return Err(io::Error::other(format!(
                "duplicate DNS record name and type {} {:?}",
                record.name, record.record_type
            )));
        }
        if !matches!(
            (record.record_type, record.ip),
            (RecordType::A, IpAddr::V4(_)) | (RecordType::Aaaa, IpAddr::V6(_))
        ) {
            return Err(io::Error::other(format!(
                "DNS record {} type does not match its IP address",
                record.name
            )));
        }
    }
    Ok(())
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn configuration() -> Result<DnsConfig, String> {
        Ok(DnsConfig {
            bind: "127.0.0.1:5353"
                .parse()
                .map_err(|error| format!("invalid test bind: {error}"))?,
            upstream: Some(
                "8.8.8.8:53"
                    .parse()
                    .map_err(|error| format!("invalid test upstream: {error}"))?,
            ),
            records: vec![DnsRecord {
                name: "PKI.AUTOBRICKS.INTERNAL".to_owned(),
                record_type: RecordType::A,
                ip: IpAddr::V4(Ipv4Addr::new(10, 10, 254, 2)),
            }],
        })
    }

    #[test]
    fn normalizes_and_accepts_exact_records() -> Result<(), String> {
        let mut value = configuration()?;
        validate(&mut value).map_err(|error| error.to_string())?;
        let name = value
            .records
            .first()
            .map(|record| record.name.as_str())
            .ok_or_else(|| "normalized test record is missing".to_owned())?;
        assert_eq!(name, "pki.autobricks.internal");
        Ok(())
    }

    #[test]
    fn rejects_duplicate_records() -> Result<(), String> {
        let mut value = configuration()?;
        value.records.push(DnsRecord {
            name: "pki.autobricks.internal".to_owned(),
            record_type: RecordType::A,
            ip: IpAddr::V4(Ipv4Addr::new(10, 10, 254, 3)),
        });
        assert!(validate(&mut value).is_err());
        Ok(())
    }

    #[test]
    fn parses_ini_configuration() -> Result<(), String> {
        let mut value = parse_ini(
            "[server]\nbind = 127.0.0.1:5353\nupstream = 8.8.8.8:53\n\
             \n[PKI.AUTOBRICKS.INTERNAL]\nA = 10.10.254.2\nAAAA = fd00::2\n",
        )
        .map_err(|error| error.to_string())?;
        validate(&mut value).map_err(|error| error.to_string())?;
        assert_eq!(value.records.len(), 2);
        assert_eq!(value.records[0].name, "pki.autobricks.internal");
        assert_eq!(value.records[1].record_type, RecordType::Aaaa);
        Ok(())
    }

    #[test]
    fn rejects_unknown_ini_configuration_keys() {
        let ini = "[server]\nbind = 127.0.0.1:5353\nunexpected = true\n\
                   \n[pki.autobricks.internal]\nCNAME = target.autobricks.internal\n";
        assert!(parse_ini(ini).is_err());
    }

    #[test]
    fn rejects_record_type_and_ip_mismatch() -> Result<(), String> {
        let mut value = configuration()?;
        let record = value
            .records
            .first_mut()
            .ok_or_else(|| "test record is missing".to_owned())?;
        record.record_type = RecordType::Aaaa;
        assert!(validate(&mut value).is_err());
        Ok(())
    }

    #[test]
    fn accepts_configuration_without_upstream() -> Result<(), String> {
        let mut value = configuration()?;
        value.upstream = None;
        validate(&mut value).map_err(|error| error.to_string())
    }

    #[test]
    fn accepts_configuration_without_local_records() -> Result<(), String> {
        let mut value = configuration()?;
        value.records.clear();
        validate(&mut value).map_err(|error| error.to_string())
    }
}
