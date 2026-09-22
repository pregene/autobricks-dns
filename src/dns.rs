use crate::config::{DnsConfig, DnsRecord};
use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PACKET: usize = 4096;

pub fn run(configuration: DnsConfig, stop: Arc<AtomicBool>) -> io::Result<()> {
    let socket = bind(configuration.bind)?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut packet = [0_u8; MAX_PACKET];
    while !stop.load(Ordering::Acquire) {
        let (size, client) = match socket.recv_from(&mut packet) {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let request = packet
            .get(..size)
            .ok_or_else(|| io::Error::other("DNS request size is invalid"))?;
        let response = match local_response(request, &configuration.records) {
            Err(error) => {
                eprintln!("DNS_REQUEST_FAILURE reason={error}");
                continue;
            }
            Ok(Some(response)) => response,
            Ok(None) => match configuration.upstream {
                Some(upstream) => match forward(request, upstream) {
                    Ok(response) => response,
                    Err(error) => {
                        eprintln!("DNS_FORWARD_FAILURE reason={error}");
                        continue;
                    }
                },
                None => negative_response(request)?,
            },
        };
        if let Err(error) = socket.send_to(&response, client) {
            eprintln!("DNS_RESPONSE_FAILURE reason={error}");
        }
    }
    Ok(())
}

fn bind(address: SocketAddr) -> io::Result<UdpSocket> {
    let domain = if address.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.bind(&address.into())?;
    Ok(socket.into())
}

fn local_response(request: &[u8], records: &[DnsRecord]) -> io::Result<Option<Vec<u8>>> {
    let (name, question_end, query_type, query_class) = question(request)?;
    let Some(record) = records
        .iter()
        .find(|record| record.name == name && record.record_type.query_type() == query_type)
    else {
        if records.iter().any(|record| record.name == name) {
            let mut response = request
                .get(..question_end)
                .ok_or_else(|| io::Error::other("DNS question is incomplete"))?
                .to_vec();
            set_header(&mut response, false)?;
            return Ok(Some(response));
        }
        return Ok(None);
    };
    let mut response = request
        .get(..question_end)
        .ok_or_else(|| io::Error::other("DNS question is incomplete"))?
        .to_vec();
    let has_answer = query_class == 1;
    set_header(&mut response, has_answer)?;
    if has_answer {
        response.extend_from_slice(&[0xc0, 0x0c]);
        response.extend_from_slice(&record.record_type.query_type().to_be_bytes());
        response.extend_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&60_u32.to_be_bytes());
        match record.ip {
            std::net::IpAddr::V4(ip) => {
                response.extend_from_slice(&4_u16.to_be_bytes());
                response.extend_from_slice(&ip.octets());
            }
            std::net::IpAddr::V6(ip) => {
                response.extend_from_slice(&16_u16.to_be_bytes());
                response.extend_from_slice(&ip.octets());
            }
        }
    }
    Ok(Some(response))
}

fn question(request: &[u8]) -> io::Result<(String, usize, u16, u16)> {
    if request.len() < 12 || read_u16(request, 4)? != 1 {
        return Err(io::Error::other("DNS request must contain one question"));
    }
    let mut position = 12_usize;
    let mut labels = Vec::new();
    loop {
        let length = usize::from(
            *request
                .get(position)
                .ok_or_else(|| io::Error::other("DNS name is incomplete"))?,
        );
        position = position
            .checked_add(1)
            .ok_or_else(|| io::Error::other("DNS name position overflow"))?;
        if length == 0 {
            break;
        }
        if length > 63 {
            return Err(io::Error::other(
                "compressed or invalid DNS question is unsupported",
            ));
        }
        let end = position
            .checked_add(length)
            .ok_or_else(|| io::Error::other("DNS label position overflow"))?;
        let label = request
            .get(position..end)
            .ok_or_else(|| io::Error::other("DNS label is incomplete"))?;
        labels.push(
            std::str::from_utf8(label)
                .map_err(io::Error::other)?
                .to_ascii_lowercase(),
        );
        position = end;
    }
    let query_type = read_u16(request, position)?;
    let class_position = position
        .checked_add(2)
        .ok_or_else(|| io::Error::other("DNS question position overflow"))?;
    let query_class = read_u16(request, class_position)?;
    let question_end = class_position
        .checked_add(2)
        .ok_or_else(|| io::Error::other("DNS question position overflow"))?;
    Ok((labels.join("."), question_end, query_type, query_class))
}

fn read_u16(bytes: &[u8], position: usize) -> io::Result<u16> {
    let high = u16::from(
        *bytes
            .get(position)
            .ok_or_else(|| io::Error::other("DNS field is incomplete"))?,
    );
    let low_position = position
        .checked_add(1)
        .ok_or_else(|| io::Error::other("DNS field position overflow"))?;
    let low = u16::from(
        *bytes
            .get(low_position)
            .ok_or_else(|| io::Error::other("DNS field is incomplete"))?,
    );
    Ok((high << 8) | low)
}

fn set_header(response: &mut [u8], has_answer: bool) -> io::Result<()> {
    let request_flags = *response
        .get(2)
        .ok_or_else(|| io::Error::other("DNS header is incomplete"))?;
    *response
        .get_mut(2)
        .ok_or_else(|| io::Error::other("DNS header is incomplete"))? =
        0x84 | (request_flags & 0x01);
    *response
        .get_mut(3)
        .ok_or_else(|| io::Error::other("DNS header is incomplete"))? = 0;
    let count = if has_answer { 1_u16 } else { 0_u16 }.to_be_bytes();
    *response
        .get_mut(6)
        .ok_or_else(|| io::Error::other("DNS header is incomplete"))? = count[0];
    *response
        .get_mut(7)
        .ok_or_else(|| io::Error::other("DNS header is incomplete"))? = count[1];
    for position in 8..12 {
        *response
            .get_mut(position)
            .ok_or_else(|| io::Error::other("DNS header is incomplete"))? = 0;
    }
    Ok(())
}

fn negative_response(request: &[u8]) -> io::Result<Vec<u8>> {
    let (_, question_end, _, _) = question(request)?;
    let mut response = request
        .get(..question_end)
        .ok_or_else(|| io::Error::other("DNS question is incomplete"))?
        .to_vec();
    set_header(&mut response, false)?;
    let flags = response
        .get_mut(3)
        .ok_or_else(|| io::Error::other("DNS header is incomplete"))?;
    *flags = (*flags & 0xf0) | 0x03;
    Ok(response)
}

fn forward(request: &[u8], upstream: SocketAddr) -> io::Result<Vec<u8>> {
    let bind_address = if upstream.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind_address)?;
    socket.connect(upstream)?;
    socket.set_read_timeout(Some(DNS_TIMEOUT))?;
    socket.send(request)?;
    let mut response = [0_u8; MAX_PACKET];
    let size = socket.recv(&mut response)?;
    response
        .get(..size)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| io::Error::other("DNS upstream response size is invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RecordType;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn query(name: &str, query_type: u16) -> io::Result<Vec<u8>> {
        let mut value = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            value.push(u8::try_from(label.len()).map_err(io::Error::other)?);
            value.extend_from_slice(label.as_bytes());
        }
        value.push(0);
        value.extend_from_slice(&query_type.to_be_bytes());
        value.extend_from_slice(&1_u16.to_be_bytes());
        Ok(value)
    }

    fn records() -> Vec<DnsRecord> {
        vec![DnsRecord {
            name: "pki.autobricks.internal".to_owned(),
            record_type: RecordType::A,
            ip: IpAddr::V4(Ipv4Addr::new(10, 10, 254, 2)),
        }]
    }

    #[test]
    fn returns_configured_ipv4_for_internal_name() -> io::Result<()> {
        let response = local_response(&query("pki.autobricks.internal", 1)?, &records())?
            .ok_or_else(|| io::Error::other("configured response is missing"))?;
        assert_eq!(read_u16(&response, 6)?, 1);
        assert!(response.ends_with(&[10, 10, 254, 2]));
        Ok(())
    }

    #[test]
    fn forwards_name_outside_internal_registry() -> io::Result<()> {
        assert!(local_response(&query("example.com", 1)?, &records())?.is_none());
        Ok(())
    }

    #[test]
    fn returns_empty_ipv6_answer_for_known_internal_name() -> io::Result<()> {
        let response = local_response(&query("pki.autobricks.internal", 28)?, &records())?
            .ok_or_else(|| io::Error::other("configured response is missing"))?;
        assert_eq!(read_u16(&response, 6)?, 0);
        Ok(())
    }

    #[test]
    fn returns_configured_ipv6_for_aaaa_record() -> io::Result<()> {
        let ipv6 = Ipv6Addr::LOCALHOST;
        let records = vec![DnsRecord {
            name: "ipv6.autobricks.internal".to_owned(),
            record_type: RecordType::Aaaa,
            ip: IpAddr::V6(ipv6),
        }];
        let response = local_response(&query("ipv6.autobricks.internal", 28)?, &records)?
            .ok_or_else(|| io::Error::other("configured response is missing"))?;
        assert_eq!(read_u16(&response, 6)?, 1);
        assert!(response.ends_with(&ipv6.octets()));
        Ok(())
    }

    #[test]
    fn returns_nxdomain_when_upstream_is_not_configured() -> io::Result<()> {
        let response = negative_response(&query("unregistered.example", 1)?)?;
        let flags = *response
            .get(3)
            .ok_or_else(|| io::Error::other("DNS response flags are missing"))?;
        assert_eq!(flags & 0x0f, 3);
        assert_eq!(read_u16(&response, 6)?, 0);
        Ok(())
    }
}
