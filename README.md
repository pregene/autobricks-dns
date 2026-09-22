# Autobricks DNS

Autobricks DNS is a lightweight split-DNS service for intranets and isolated
networks. It returns configured private addresses for explicitly registered
internal host names, taking precedence over normal DNS resolution for those
names. Queries for every other name are forwarded unchanged to an upstream DNS
server, preserving normal DNS behavior for public or otherwise unregistered
names.

Clients must use Autobricks DNS as their DNS resolver for this local-record
override to apply. It does not modify a client's existing operating-system DNS
configuration by itself.

## Features

- Exact, case-insensitive A and AAAA record matching
- Local records support A and AAAA only
- Local records take precedence over upstream DNS answers for matching names
- UDP forwarding for unregistered names, or NXDOMAIN replies when no upstream is configured
- INI configuration with validation and atomic updates
- Local Unix-socket control interface for listing and changing records
- IPv4 and IPv6 listener support

## Requirements

- A current stable Rust toolchain
- Permission to bind the configured UDP port. Binding port 53 generally
  requires elevated privileges or an operating-system-specific capability.

## Build

```sh
cargo build --release --locked
```

The executable is written to `target/release/autobricks-dns`.

For Linux systemd installation, service registration, operations, and
verification, see [INSTALL.md](INSTALL.md).

## Configuration

By default, Autobricks DNS reads `config/autobricks-dns.ini` relative to the
current working directory. Set `AUTOBRICKS_DNS_CONFIG` to use another file.

```ini
[server]
bind = 127.0.0.1:5353
upstream = 1.1.1.1:53

[api.example.internal]
A = 10.10.0.10
AAAA = fd00::10
```

`bind` and `upstream` in `[server]` are socket addresses. `upstream` is
optional. Define each DNS name in its own section, using `A = <IPv4>` and/or
`AAAA = <IPv6>`. A and AAAA are the only supported local record types; MX,
CNAME, TXT, and other record types are not accepted in the configuration. Each
name and record-type pair must be unique. Record names are normalized to
lowercase, and only valid DNS labels are accepted. An empty local record set is
valid and makes the service an upstream DNS forwarder.

## Run

Create the default configuration, then run the release binary:

```sh
cargo run --release --locked
```

For a custom configuration or control socket path:

```sh
AUTOBRICKS_DNS_CONFIG=/etc/autobricks-dns/autobricks-dns.ini \
AUTOBRICKS_DNS_SOCKET=/run/autobricks-dns/autobricks-dns.sock \
target/release/autobricks-dns
```

## DNS Behavior

Configured records answer only matching A or AAAA queries, with a 60-second
TTL. This local response takes precedence over an answer that an upstream DNS
server might otherwise return for the same name. A query for a configured name
but an unconfigured record type returns an empty successful response. A name
absent from the local records is forwarded unchanged to `upstream`; when no
upstream is configured, it returns NXDOMAIN.

The server accepts one uncompressed DNS question per UDP packet. It does not
provide TCP DNS, recursive resolution, zone transfers, or DNSSEC.

## Record Management

Start `autobricks-dns` with no arguments to run the DNS service. Once it is
running, use the same executable to manage local records through its Unix-domain
control socket:

```sh
autobricks-dns list
autobricks-dns add --name cache.example.internal --ip 10.10.0.20 --type A
autobricks-dns add --name cache.example.internal --ip fd00::20 --type AAAA
autobricks-dns delete --name cache.example.internal
autobricks-dns restart
```

`list` prints the configured records as JSON. `add` creates a single A or AAAA
record. `delete` removes every A and AAAA record for the supplied name. `restart`
requests a clean service exit so a service manager can start it again.

The service creates its Unix-domain control socket at
`/run/autobricks-dns/autobricks-dns.sock` by default. Use
`AUTOBRICKS_DNS_SOCKET` to override it. The socket permissions are `0660`.
Configuration changes are atomically written with owner-only file permissions on
Unix platforms.

## Development

```sh
cargo test --locked
cargo clippy --all-targets -- -D warnings
```

## License

Copyright 2026 Autobricks, Co.

This project is licensed under the GNU General Public License v3.0 only. See
[COPYING](COPYING) for the full license text.