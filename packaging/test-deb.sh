#!/bin/sh
# Runs only inside the disposable Ubuntu build container.
set -eu
package=$1
config=/etc/autobricks/dns/autobricks-dns.ini
state=/var/lib/autobricks/dns/records.ini
service=/lib/systemd/system/autobricks-dns.service

test "$(dpkg-deb -f "$package" Architecture)" = "$(dpkg --print-architecture)"
dpkg -i "$package"
test -x /usr/bin/autobricks/autobricks-dns
test -L /usr/bin/autobricks-dns
test "$(readlink /usr/bin/autobricks-dns)" = autobricks/autobricks-dns
test -f "$service"
test ! -e /etc/systemd/system/multi-user.target.wants/autobricks-dns.service
test "$(stat -c '%U:%G:%a' "$config")" = root:autobricks-dns:640
test "$(stat -c '%U:%G:%a' "$state")" = autobricks-dns:autobricks-dns:600
systemd-analyze verify "$service"
autobricks-dns --help

# Exercise the actual Linux executable as the service user without external DNS.
cat > "$config" <<'EOF'
[server]
bind = 127.0.0.1:15353
EOF
cat > "$state" <<'EOF'
[package-test.internal]
A = 10.20.30.40
EOF
chown root:autobricks-dns "$config"
chmod 0640 "$config"
chown autobricks-dns:autobricks-dns "$state"
chmod 0600 "$state"
python3 - <<'PY'
import json
import os
import socket
import struct
import subprocess
import time

env = dict(os.environ,
           AUTOBRICKS_DNS_CONFIG='/etc/autobricks/dns/autobricks-dns.ini',
           AUTOBRICKS_DNS_STATE='/var/lib/autobricks/dns/records.ini',
           AUTOBRICKS_DNS_SOCKET='/var/run/autobricks/dns.sock')
server = subprocess.Popen(['runuser', '-u', 'autobricks-dns', '--', '/usr/bin/autobricks/autobricks-dns'], env=env)
try:
    for _ in range(100):
        if os.path.exists('/var/run/autobricks/dns.sock'):
            break
        if server.poll() is not None:
            raise RuntimeError('DNS server exited during startup')
        time.sleep(0.05)
    else:
        raise RuntimeError('Control socket did not become ready')
    question = b''.join(bytes([len(label)]) + label for label in b'package-test.internal'.split(b'.')) + b'\0'
    request = struct.pack('!6H', 0x1234, 0x0100, 1, 0, 0, 0) + question + struct.pack('!2H', 1, 1)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
        client.settimeout(3)
        client.sendto(request, ('127.0.0.1', 15353))
        response = client.recv(4096)
        assert response[:2] == request[:2] and response[-4:] == bytes([10, 20, 30, 40])
    subprocess.run(['autobricks-dns', 'add', '--name', 'persist.internal', '--ip', '10.20.30.41', '--type', 'A'], check=True)
    records = json.loads(subprocess.check_output(['autobricks-dns', 'list']))
    assert any(record['name'] == 'persist.internal' for record in records)
    subprocess.run(['autobricks-dns', 'restart'], check=True)
    assert server.wait(timeout=10) == 0
finally:
    if server.poll() is None:
        server.terminate()
        server.wait(timeout=10)
PY

# An upgrade/reinstall, remove, and purge must preserve mutable record state.
# Simulate systemd availability to verify maintainer-script action dispatch.
# Real service-manager execution still requires a booted Ubuntu host.
mkdir -p /tmp/mock-systemd /run/systemd/system
cat > /tmp/mock-systemd/deb-systemd-invoke <<'EOF'
#!/bin/sh
echo "invoke $*" >> /tmp/service-actions.log
EOF
cat > /tmp/mock-systemd/systemctl <<'EOF'
#!/bin/sh
case "$*" in
    '--system daemon-reload')
        echo reload >> /tmp/service-actions.log ;;
    *) exec /usr/bin/systemctl "$@" ;;
esac
EOF
chmod 0755 /tmp/mock-systemd/*
export PATH="/tmp/mock-systemd:$PATH"
: > /tmp/service-actions.log
cp "$config" /tmp/autobricks-dns-expected.ini
cp "$state" /tmp/autobricks-dns-expected-records.ini
dpkg -i "$package"
cmp "$config" /tmp/autobricks-dns-expected.ini
cmp "$state" /tmp/autobricks-dns-expected-records.ini
grep -Fxq 'reload' /tmp/service-actions.log
grep -Fxq 'invoke try-restart autobricks-dns.service' /tmp/service-actions.log
if grep -Fq 'invoke stop' /tmp/service-actions.log; then
    echo 'Upgrade unexpectedly stopped the service before replacement.' >&2
    exit 1
fi
systemctl --root=/ enable autobricks-dns.service
: > /tmp/service-actions.log
dpkg --remove autobricks-dns
grep -Fxq 'invoke stop autobricks-dns.service' /tmp/service-actions.log
test ! -e /usr/bin/autobricks-dns
test ! -e /usr/bin/autobricks/autobricks-dns
test ! -e "$service"
cmp "$config" /tmp/autobricks-dns-expected.ini
cmp "$state" /tmp/autobricks-dns-expected-records.ini

# Reinstall after remove keeps records too; purge removes only owned config.
dpkg -i "$package"
cmp "$config" /tmp/autobricks-dns-expected.ini
cmp "$state" /tmp/autobricks-dns-expected-records.ini
touch /etc/autobricks/dns/operator-backup.ini
dpkg --purge autobricks-dns
test ! -e "$config"
test -e /etc/autobricks/dns/operator-backup.ini
cmp "$state" /tmp/autobricks-dns-expected-records.ini
test ! -e /etc/systemd/system/multi-user.target.wants/autobricks-dns.service
test ! -L /etc/systemd/system/multi-user.target.wants/autobricks-dns.service
getent passwd autobricks-dns >/dev/null

# Purge/reinstall creates fresh defaults and can be purged again.
rm /etc/autobricks/dns/operator-backup.ini
: > /tmp/service-actions.log
dpkg -i "$package"
grep -Fxq 'reload' /tmp/service-actions.log
if grep -Fq 'invoke ' /tmp/service-actions.log; then
    echo 'Fresh installation unexpectedly started or restarted the service.' >&2
    exit 1
fi
cmp "$config" /usr/share/autobricks-dns/default.ini
cmp "$state" /tmp/autobricks-dns-expected-records.ini
dpkg --purge autobricks-dns
test ! -d /etc/autobricks/dns
cmp "$state" /tmp/autobricks-dns-expected-records.ini
echo 'PASS: canonical layout, Linux DNS/CLI, reinstall, remove, purge, and state preservation'
