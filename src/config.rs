use ini::Ini;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const DEFAULT_CONFIG: &str = "/etc/autobricks/dns/autobricks-dns.ini";
const DEFAULT_STATE: &str = "/var/lib/autobricks/dns/records.ini";

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
    let config_path = std::env::var_os("AUTOBRICKS_DNS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    let state_path = std::env::var_os("AUTOBRICKS_DNS_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE));
    let (bind, upstream) = load_server(&config_path)?;
    let records = load_records(&state_path)?;
    Ok((
        state_path,
        DnsConfig {
            bind,
            upstream,
            records,
        },
    ))
}

fn load_server(path: &Path) -> io::Result<(SocketAddr, Option<SocketAddr>)> {
    let document = Ini::load_from_file(path).map_err(io::Error::other)?;
    let server = document
        .section(Some("server"))
        .ok_or_else(|| io::Error::other("DNS INI configuration requires a [server] section"))?;
    if document
        .iter()
        .any(|(section, properties)| section != Some("server") && !properties.is_empty())
        || server
            .iter()
            .any(|(key, _)| key != "bind" && key != "upstream")
    {
        return Err(io::Error::other(
            "DNS service configuration only accepts bind and upstream in [server]",
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
    if upstream == Some(bind) {
        return Err(io::Error::other(
            "DNS upstream must not point to the DNS service bind",
        ));
    }
    Ok((bind, upstream))
}

pub fn load_records(path: &Path) -> io::Result<Vec<DnsRecord>> {
    let document = Ini::load_from_file(path).map_err(io::Error::other)?;
    let mut records = Vec::new();
    for (section, properties) in &document {
        let Some(name) = section else {
            if properties.is_empty() {
                continue;
            }
            return Err(io::Error::other(
                "DNS record state does not allow top-level keys",
            ));
        };
        for (record_type, value) in properties {
            let record_type = match record_type {
                "A" => RecordType::A,
                "AAAA" => RecordType::Aaaa,
                _ => {
                    return Err(io::Error::other(format!(
                        "DNS record section [{name}] only accepts A and AAAA keys"
                    )));
                }
            };
            records.push(DnsRecord {
                name: name.to_owned(),
                record_type,
                ip: value
                    .parse()
                    .map_err(|error| io::Error::other(format!("invalid DNS record IP: {error}")))?,
            });
        }
    }
    validate_records(&mut records)?;
    Ok(records)
}

pub fn save_records(path: &Path, records: &mut [DnsRecord]) -> io::Result<()> {
    validate_records(records)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("DNS state parent is missing"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::other("DNS state filename is invalid"))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let mut ordered = records.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (left.name.as_str(), left.record_type).cmp(&(right.name.as_str(), right.record_type))
    });
    let mut output = String::new();
    let mut current = None;
    for record in ordered {
        if current != Some(record.name.as_str()) {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("[{}]\n", record.name));
            current = Some(record.name.as_str());
        }
        output.push_str(&format!(
            "{} = {}\n",
            record.record_type.ini_name(),
            record.ip
        ));
    }
    let result = write_atomic(&temporary, path, output.as_bytes());
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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
    file.sync_all()?;
    fs::rename(temporary, destination)?;
    fs::File::open(
        destination
            .parent()
            .ok_or_else(|| io::Error::other("DNS state parent is missing"))?,
    )?
    .sync_all()
}

#[cfg(not(unix))]
fn write_atomic(_temporary: &Path, _destination: &Path, _bytes: &[u8]) -> io::Result<()> {
    Err(io::Error::other(
        "atomic DNS record storage is unavailable on this platform",
    ))
}

pub(crate) fn validate_records(records: &mut [DnsRecord]) -> io::Result<()> {
    let mut names = BTreeSet::new();
    for record in records {
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

    fn record() -> DnsRecord {
        DnsRecord {
            name: "PKI.AUTOBRICKS.INTERNAL".to_owned(),
            record_type: RecordType::A,
            ip: IpAddr::V4(Ipv4Addr::new(10, 10, 254, 2)),
        }
    }

    fn fixture(name: &str, value: &str) -> io::Result<PathBuf> {
        let directory = std::env::temp_dir().join(format!(
            "autobricks-dns-config-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory)?;
        let path = directory.join(name);
        fs::write(&path, value)?;
        Ok(path)
    }

    #[test]
    fn server_configuration_and_record_state_are_separate() -> io::Result<()> {
        let config = fixture(
            "autobricks-dns.ini",
            "[server]\nbind = 127.0.0.1:5353\nupstream = 8.8.8.8:53\n",
        )?;
        let state = fixture(
            "records.ini",
            "[PKI.AUTOBRICKS.INTERNAL]\nA = 10.10.254.2\nAAAA = fd00::2\n",
        )?;
        let (bind, upstream) = load_server(&config)?;
        let records = load_records(&state)?;
        assert_eq!(bind, "127.0.0.1:5353".parse().map_err(io::Error::other)?);
        assert_eq!(
            upstream,
            Some("8.8.8.8:53".parse().map_err(io::Error::other)?)
        );
        assert_eq!(records.len(), 2);
        assert_eq!(
            records.first().map(|record| record.name.as_str()),
            Some("pki.autobricks.internal")
        );
        fs::remove_dir_all(
            config
                .parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )
    }

    #[test]
    fn rejects_records_in_service_configuration() -> io::Result<()> {
        let path = fixture(
            "invalid.ini",
            "[server]\nbind = 127.0.0.1:5353\n\n[pki.internal]\nA = 10.0.0.1\n",
        )?;
        assert!(load_server(&path).is_err());
        fs::remove_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )
    }

    #[test]
    fn normalizes_and_rejects_duplicate_records() {
        let mut records = vec![record()];
        assert!(validate_records(&mut records).is_ok());
        assert_eq!(
            records.first().map(|record| record.name.as_str()),
            Some("pki.autobricks.internal")
        );
        records.push(record());
        assert!(validate_records(&mut records).is_err());
    }

    #[test]
    fn rejects_record_type_and_ip_mismatch() {
        let mut invalid = record();
        invalid.record_type = RecordType::Aaaa;
        let mut records = vec![invalid];
        assert!(validate_records(&mut records).is_err());
    }
}
