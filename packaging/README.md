# Ubuntu 22.04/24.04 amd64 and arm64 packages

## Build

From the repository root, with Docker running:

```sh
./scripts/build-package-matrix.sh
```

To build one target, pass the Ubuntu release and architecture explicitly, for
example `./scripts/build-deb.sh 24.04 arm64`. The matrix command builds and tests
the Linux executable inside Ubuntu 22.04 and 24.04,
using Docker emulation when the target differs from the host. The first build downloads
Ubuntu packages and Rust 1.85.1. Docker and internet access are required.
Cargo.lock pins Rust dependencies. The Ubuntu base image and apt repositories
receive updates, so this is a repeatable build procedure, not a guarantee of
byte-identical output. The output includes:

```text
build/autobricks-dns-1.0.3-ubuntu-22.04-amd64.deb
build/autobricks-dns-1.0.3-ubuntu-22.04-arm64.deb
build/autobricks-dns-1.0.3-ubuntu-24.04-amd64.deb
build/autobricks-dns-1.0.3-ubuntu-24.04-arm64.deb
build/SHA256SUMS
```

All build output and temporary contexts are under the Git-ignored `build/`
directory. Packaging sources and the build script are tracked. Docker retains
its own build cache outside the repository. The image build checks installation,
real UDP DNS responses and CLI commands, configuration preservation on reinstall,
remove, purge, and fresh installation after purge. Maintainer-script service
reload/restart/stop calls are checked using simulated service-manager commands.
The container does not boot
systemd; actual systemd start/stop behavior must also be checked on a Ubuntu VM
or host. The unit is checked with `systemd-analyze verify`.

## Install and start

On an Ubuntu host, select the package matching the OS release and
`dpkg --print-architecture`:

```sh
sudo apt install ./build/autobricks-dns-1.0.3-ubuntu-22.04-$(dpkg --print-architecture).deb
sudoedit /etc/autobricks/dns/autobricks-dns.ini
sudo ss -ulpn 'sport = :53'
sudo systemctl enable --now autobricks-dns.service
sudo systemctl status autobricks-dns.service
```

The package installs `/usr/bin/autobricks/autobricks-dns`, the compatible
`/usr/bin/autobricks-dns` symbolic link, a systemd unit, and a dedicated
non-login `autobricks-dns` account. It registers the service but does not enable
or start it on first installation. Configure the listener, upstream, and local
records before starting. Ubuntu's existing DNS resolver may already use port
53; choose an available listener address or explicitly reconfigure that service.
The package does not alter OS DNS, DHCP, firewall, or other resolver services.

The service configuration is installed under `/etc/autobricks/dns/` and is not
modified by the CLI. Mutable records are stored in
`/var/lib/autobricks/dns/records.ini`; upgrades, removal, and purge preserve
that state. The service account owns the state directory and file.

```sh
sudo /usr/bin/autobricks/autobricks-dns list
sudo /usr/bin/autobricks/autobricks-dns add --name cache.example.internal --ip 10.10.0.20 --type A
sudo /usr/bin/autobricks/autobricks-dns restart
```

The public command name remains available through the package-owned symbolic
link. Package scripts never overwrite an unrelated existing path.

## Upgrade

Install the new `.deb` with `sudo apt install ./path/to/package.deb`. Existing
configuration and records are preserved. An active service is restarted after
upgrade; an inactive service stays inactive. Back up configuration before
upgrading. If startup fails, inspect `journalctl -u autobricks-dns.service`.

## Uninstall

Stop and remove the executable and service, keeping settings and records:

```sh
sudo apt remove autobricks-dns
```

Remove the package and its live configuration, including CLI-created records:

```sh
sudo apt purge autobricks-dns
```

Purge also cleans systemd enablement and the known control socket. Unrelated
files, administrator backups, custom systemd overrides, and journal history
are not recursively deleted. The non-login system account/group are retained
to avoid reassigning their IDs while other files may still refer to them.
After checking for remaining owned files and group members, administrators may
remove these identities explicitly with `sudo deluser autobricks-dns` and,
if the group still exists, `sudo delgroup autobricks-dns`.

Clients that use this server for DNS must be moved to another resolver before
the service is removed.

The remove/purge and systemd integration follow [Debian configuration-file
policy](https://www.debian.org/doc/debian-policy/ch-files.html#configuration-files)
and [dh_installsystemd](https://manpages.debian.org/bookworm/debhelper/dh_installsystemd.1.en.html).
