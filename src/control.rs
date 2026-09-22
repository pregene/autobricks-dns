use crate::config::DnsRecord;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEFAULT_SOCKET: &str = "/run/autobricks-dns/autobricks-dns.sock";
const MAX_FRAME: u64 = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "command", rename_all = "UPPERCASE")]
enum Request {
    List,
    Add {
        record: DnsRecord,
    },
    Delete {
        name: String,
    },
    Restart,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    ok: bool,
    result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    records: Option<Vec<DnsRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

pub fn list() -> io::Result<Vec<DnsRecord>> {
    let response = send(Request::List)?;
    if response.ok {
        response
            .records
            .ok_or_else(|| io::Error::other("DNS control response is missing records"))
    } else {
        Err(io::Error::other(response.detail.unwrap_or(response.result)))
    }
}

pub fn add(record: DnsRecord) -> io::Result<()> {
    ensure_success(send(Request::Add { record })?)
}

pub fn delete(name: String) -> io::Result<()> {
    ensure_success(send(Request::Delete { name })?)
}

pub fn restart() -> io::Result<()> {
    ensure_success(send(Request::Restart)?)
}

fn send(request: Request) -> io::Result<Response> {
    let socket_path = socket_path();
    let mut stream = UnixStream::connect(socket_path)?;
    serde_json::to_writer(&mut stream, &request).map_err(io::Error::other)?;
    stream.write_all(b"\n")?;
    stream.shutdown(Shutdown::Write)?;
    let mut response = String::new();
    BufReader::new(stream).read_to_string(&mut response)?;
    serde_json::from_str(response.trim_end()).map_err(io::Error::other)
}

fn ensure_success(response: Response) -> io::Result<()> {
    if response.ok {
        Ok(())
    } else {
        Err(io::Error::other(response.detail.unwrap_or(response.result)))
    }
}

pub fn start(
    config_path: PathBuf,
    stop: Arc<AtomicBool>,
) -> io::Result<JoinHandle<io::Result<()>>> {
    let socket_path = socket_path();
    prepare_socket(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o660))?;
    listener.set_nonblocking(true)?;
    thread::Builder::new()
        .name("autobricks-dns-control".to_owned())
        .spawn(move || serve(listener, &socket_path, &config_path, stop))
}

fn socket_path() -> PathBuf {
    std::env::var_os("AUTOBRICKS_DNS_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET))
}

fn prepare_socket(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("DNS control socket parent is missing"))?;
    fs::create_dir_all(parent)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn serve(
    listener: UnixListener,
    socket_path: &Path,
    config_path: &Path,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = handle(stream, config_path, &stop) {
                    eprintln!("DNS_CONTROL_FAILURE reason={error}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    drop(listener);
    match fs::remove_file(socket_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn handle(mut stream: UnixStream, config_path: &Path, stop: &AtomicBool) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut frame = String::new();
    let mut reader = BufReader::new(stream.try_clone()?.take(MAX_FRAME + 1));
    let size = reader.read_line(&mut frame)?;
    if size == 0
        || u64::try_from(size).map_err(io::Error::other)? > MAX_FRAME
        || !frame.ends_with('\n')
    {
        return write_response(
            &mut stream,
            error_response(
                "INVALID_FRAME",
                "request must be one bounded newline-terminated JSON frame",
            ),
        );
    }
    let request = serde_json::from_str::<Request>(frame.trim_end()).map_err(io::Error::other)?;
    let response = apply(config_path, request, stop);
    write_response(&mut stream, response)?;
    stream.shutdown(Shutdown::Both)
}

fn apply(config_path: &Path, request: Request, stop: &AtomicBool) -> Response {
    match apply_inner(config_path, request, stop) {
        Ok(response) => response,
        Err(error) => error_response("FAILED", &error.to_string()),
    }
}

fn apply_inner(config_path: &Path, request: Request, stop: &AtomicBool) -> io::Result<Response> {
    let mut configuration = crate::config::load_path(config_path)?;
    match request {
        Request::List => Ok(success("LISTED", Some(configuration.records))),
        Request::Add { mut record } => {
            record.name.make_ascii_lowercase();
            let existing = configuration.records.iter().find(|value| {
                value.name.eq_ignore_ascii_case(&record.name)
                    && value.record_type == record.record_type
            });
            if let Some(existing) = existing {
                if existing.ip == record.ip {
                    return Ok(success("UNCHANGED", None));
                }
                return Ok(error_response(
                    "CONFLICT",
                    "record name and type already exist with a different IP",
                ));
            }
            configuration.records.push(record);
            crate::config::save(config_path, &mut configuration)?;
            Ok(success("ADDED", None))
        }
        Request::Delete {
            mut name,
        } => {
            name.make_ascii_lowercase();
            let before = configuration.records.len();
            configuration
                .records
                .retain(|record| record.name != name);
            if configuration.records.len() == before {
                return Ok(error_response("NOT_FOUND", "record does not exist"));
            }
            crate::config::save(config_path, &mut configuration)?;
            Ok(success("DELETED", None))
        }
        Request::Restart => {
            stop.store(true, Ordering::Release);
            Ok(success("RESTARTING", None))
        }
    }
}

fn success(result: &'static str, records: Option<Vec<DnsRecord>>) -> Response {
    Response {
        ok: true,
        result: result.to_owned(),
        records,
        detail: None,
    }
}

fn error_response(result: &'static str, detail: &str) -> Response {
    Response {
        ok: false,
        result: result.to_owned(),
        records: None,
        detail: Some(detail.to_owned()),
    }
}

fn write_response(stream: &mut UnixStream, response: Response) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, &response).map_err(io::Error::other)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DnsConfig, RecordType};
    use std::net::{IpAddr, Ipv4Addr};

    fn fixture() -> io::Result<(PathBuf, DnsConfig)> {
        let directory = std::env::temp_dir().join(format!(
            "autobricks-dns-control-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        fs::create_dir_all(&directory)?;
        let path = directory.join("autobricks-dns.json");
        let mut configuration = DnsConfig {
            bind: "127.0.0.1:5353".parse().map_err(io::Error::other)?,
            upstream: None,
            records: vec![DnsRecord {
                name: "pki.autobricks.internal".to_owned(),
                record_type: RecordType::A,
                ip: IpAddr::V4(Ipv4Addr::new(10, 10, 254, 1)),
            }],
        };
        crate::config::save(&path, &mut configuration)?;
        Ok((path, configuration))
    }

    #[test]
    fn add_and_delete_persist_configuration() -> io::Result<()> {
        let (path, _) = fixture()?;
        let stop = AtomicBool::new(false);
        let record = DnsRecord {
            name: "CADDY.AUTOBRICKS.INTERNAL".to_owned(),
            record_type: RecordType::A,
            ip: IpAddr::V4(Ipv4Addr::new(10, 10, 254, 20)),
        };
        assert_eq!(
            apply_inner(&path, Request::Add { record }, &stop)?.result,
            "ADDED"
        );
        assert!(crate::config::load_path(&path)?
            .records
            .iter()
            .any(|value| value.name == "caddy.autobricks.internal"));
        assert_eq!(
            apply_inner(
                &path,
                Request::Delete {
                    name: "caddy.autobricks.internal".to_owned(),
                },
                &stop
            )?
            .result,
            "DELETED"
        );
        assert!(!crate::config::load_path(&path)?
            .records
            .iter()
            .any(|value| value.name == "caddy.autobricks.internal"));
        fs::remove_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("fixture parent missing"))?,
        )
    }

    #[test]
    fn restart_sets_stop_after_response_is_prepared() -> io::Result<()> {
        let (path, _) = fixture()?;
        let stop = AtomicBool::new(false);
        assert_eq!(
            apply_inner(&path, Request::Restart, &stop)?.result,
            "RESTARTING"
        );
        assert!(stop.load(Ordering::Acquire));
        fs::remove_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("fixture parent missing"))?,
        )
    }
}
