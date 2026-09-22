# Installation and Operations

For an Ubuntu 22.04 amd64 or arm64 `.deb`, including build, install, upgrade, remove,
and purge instructions, see [packaging/README.md](packaging/README.md).
The sections below describe manual installation without a package manager.

This guide installs Autobricks DNS on a Linux host running `systemd`. The
service provides local A and AAAA overrides for intranet or isolated-network
names and forwards all unregistered names to the configured upstream DNS
server.

The commands use Debian or Ubuntu package names. Use equivalent packages on
other Linux distributions.

## 1. Install Build Requirements

Install Git, a C build toolchain, and Rust:

```sh
sudo apt update
sudo apt install --yes build-essential curl git pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
rustc --version
cargo --version
```

For a non-interactive Rust installation, use `sh -s -- -y` at the end of the
`rustup.rs` command.

## 2. Download and Build

Clone the public repository and build the locked release dependencies:

```sh
git clone https://github.com/pregene/autobricks-dns.git
cd autobricks-dns
cargo test --locked
cargo build --release --locked
```

The release executable is `target/release/autobricks-dns`.

## 3. Create the Service Account and Files

Create a dedicated system account and the configuration directory. The service
must be able to modify its INI file when `add` or `delete` is used.

```sh
sudo useradd --system --user-group --no-create-home --shell /usr/sbin/nologin autobricks-dns
sudo install -d -o autobricks-dns -g autobricks-dns -m 0750 /etc/autobricks-dns
sudo install -o root -g root -m 0755 target/release/autobricks-dns /usr/local/bin/autobricks-dns
sudo install -o autobricks-dns -g autobricks-dns -m 0600 \
  config/autobricks-dns.ini /etc/autobricks-dns/autobricks-dns.ini
```

Edit `/etc/autobricks-dns/autobricks-dns.ini` before starting the service.

```ini
[server]
bind = 0.0.0.0:53
upstream = 8.8.8.8:53

[api.example.internal]
A = 10.10.0.10
AAAA = fd00::10
```

Use an upstream resolver reachable from the DNS server. Do not set `upstream`
to the same address and port as `bind`, or the service will reject its
configuration to prevent a forwarding loop.

## 4. Register the systemd Service

Create `/etc/systemd/system/autobricks-dns.service`:

```ini
[Unit]
Description=Autobricks DNS split-DNS service
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=30s
StartLimitBurst=5

[Service]
Type=simple
User=autobricks-dns
Group=autobricks-dns
Environment=AUTOBRICKS_DNS_CONFIG=/etc/autobricks-dns/autobricks-dns.ini
Environment=AUTOBRICKS_DNS_SOCKET=/run/autobricks-dns/autobricks-dns.sock
ExecStart=/usr/local/bin/autobricks-dns
Restart=always
RestartSec=0

# Allow the non-root service account to bind UDP port 53.
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true

RuntimeDirectory=autobricks-dns
RuntimeDirectoryMode=0750
ReadWritePaths=/etc/autobricks-dns
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Enable and start it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now autobricks-dns.service
sudo systemctl status autobricks-dns.service
```

`Restart=always` starts the service again immediately after either a clean exit
or a failure. This also makes `autobricks-dns restart` work: it asks the service
to exit cleanly, then systemd starts it again.

`StartLimitBurst=5` and `StartLimitIntervalSec=30s` stop the unit after five
start attempts within 30 seconds. Fix the underlying error, then clear the
failed state and start the service again:

```sh
sudo systemctl reset-failed autobricks-dns.service
sudo systemctl start autobricks-dns.service
```

If another DNS service already owns UDP port 53, stop or reconfigure it before
starting Autobricks DNS. Check the active listener with:

```sh
sudo ss -ulpn 'sport = :53'
```

Allow UDP port 53 through the host firewall for the networks that should use
this DNS server. Do not expose a resolver to untrusted networks without an
appropriate access-control policy.

## 5. Configure DNS Clients

For client machines to receive local overrides, they must query the Autobricks
DNS server. Configure their DNS server address through DHCP, the operating
system network settings, or a network manager.

For a temporary test on a Linux client using `systemd-resolved`:

```sh
sudo resolvectl dns <interface> <dns-server-address>
resolvectl status <interface>
```

Replace `<interface>` with an interface such as `eth0`, and replace
`<dns-server-address>` with the address of the host running Autobricks DNS.
Because unregistered names are forwarded upstream, this one DNS server can
resolve both internal override names and ordinary public names.

## 6. Verify DNS Responses

Run these commands from the DNS server or a reachable client. Replace
`<dns-server-address>` with the server address.

```sh
dig @<dns-server-address> api.example.internal A
dig @<dns-server-address> api.example.internal AAAA
dig @<dns-server-address> example.com A
```

The first two queries should return the configured private addresses. The
`example.com` query should return the upstream resolver's answer.

## 7. Manage Local Records

The service exposes record management through a local Unix socket. Add an
administrator to the `autobricks-dns` group, then start a new login session so
the group membership takes effect:

```sh
sudo usermod -aG autobricks-dns <administrator-user>
```

Run these commands as that administrator, or with `sudo`:

```sh
autobricks-dns list
autobricks-dns add --name cache.example.internal --ip 10.10.0.20 --type A
autobricks-dns add --name cache.example.internal --ip fd00::20 --type AAAA
autobricks-dns delete --name cache.example.internal
autobricks-dns restart
```

`list` writes the configured records as JSON. `add` accepts only `A` and
`AAAA`. `delete` removes every local A and AAAA record for the supplied name.
The service writes changes atomically to
`/etc/autobricks-dns/autobricks-dns.ini`.

## 8. Monitor and Troubleshoot

Inspect service state and follow logs:

```sh
sudo systemctl status autobricks-dns.service
sudo journalctl -u autobricks-dns.service -f
```

Common checks:

```sh
sudo ss -ulpn 'sport = :53'
sudo ls -l /run/autobricks-dns/autobricks-dns.sock
sudo cat /etc/autobricks-dns/autobricks-dns.ini
```

If a management command reports a socket connection error, confirm that the
service is running, that `AUTOBRICKS_DNS_SOCKET` has the same value for the
service and command, and that the caller has permission to access the socket.

## 9. Upgrade

Build the new release, install it over the current binary, then restart the
service:

```sh
git pull --ff-only
cargo test --locked
cargo build --release --locked
sudo install -o root -g root -m 0755 target/release/autobricks-dns /usr/local/bin/autobricks-dns
sudo systemctl restart autobricks-dns.service
```
