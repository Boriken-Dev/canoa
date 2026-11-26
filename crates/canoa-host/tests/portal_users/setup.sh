#!/bin/sh
# Makes what tests/portal_users.rs needs on a Linux machine with systemd, as root: three throwaway accounts (canoatestportal for
# the portal, canoatesta and canoatestb for the two people), a PAM service that checks a password file of its own, a slice and
# the template unit, the root helper and a sudoers rule that lets the portal run only it, and a copy of the built programs.
#
#   sudo sh setup.sh TARGET_DIR [MTLS_PLUGIN]   # TARGET_DIR holds canoa and examples/stdio_agent, such as target/debug; the plugin is
#                                               # found under TARGET_DIR/deps when it is not named
#   sudo sh teardown.sh              # removes all of it
#
# Every name starts with canoatest. Nothing here touches another account, sssd, or the real PAM services. The proxy's socket is
# for the group of the user who ran sudo, so the test can stand in for the proxy.
set -eu

target=${1:?"usage: setup.sh TARGET_DIR"}
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
tester=${SUDO_USER:?"run through sudo, so the test can reach the proxy's socket"}
tester_group=$(id -gn "$tester")
here=$(dirname "$0")

for name in canoatestportal canoatesta canoatestb canoatestc; do
    if id "$name" >/dev/null 2>&1; then
        echo "$name exists already; run teardown.sh first" >&2
        exit 1
    fi
done

useradd --system --no-create-home --shell /usr/sbin/nologin canoatestportal
useradd --create-home --shell /usr/sbin/nologin canoatesta
useradd --create-home --shell /usr/sbin/nologin canoatestb
# Outside the sign-in group: an account like any other on the machine, which must never be given a host.
useradd --create-home --shell /usr/sbin/nologin canoatestc
chmod 0700 /home/canoatesta /home/canoatestb /home/canoatestc
groupadd canoatestusers
usermod -aG canoatestusers canoatesta
usermod -aG canoatestusers canoatestb
# The portal may make the proxy's socket for the tester's group.
usermod -aG "$tester_group" canoatestportal

install -d -m 0755 /opt/canoatest /opt/canoatest/site /etc/canoatest
install -m 0755 "$target/canoa" /opt/canoatest/canoa
install -m 0755 "$target/examples/stdio_agent" /opt/canoatest/stdio_agent
echo '<!doctype html>the app' > /opt/canoatest/site/index.html
# What the test runs as one account to reach another's sockets, from where every account can read it.
install -m 0644 "$here/uds.py" /opt/canoatest/uds.py
# The root helper, for units named canoatest@NAME.service.
sed 's/^PREFIX=canoa$/PREFIX=canoatest/; s/^ALLOWED_GROUPS=canoa-users$/ALLOWED_GROUPS=canoatestusers/' "$here/../../dist/canoa-instance" > /opt/canoatest/canoa-instance
chmod 0755 /opt/canoatest/canoa-instance
chown root:root /opt/canoatest/canoa-instance

# The host's entries, in the layer beside the program; each person's own canoa.cfg goes over it.
cat > /opt/canoatest/canoa.cfg <<'EOF'
[host.agents.fixture]
program = "/opt/canoatest/stdio_agent"
EOF

cat > /opt/canoatest/portal.toml <<EOF
[portal]
socket = "/run/canoatest-portal/portal.sock"
socket_group = "$tester_group"
site = "/opt/canoatest/site"
socket_dir = "/run/canoatest"
unit = "canoatest@{user}.service"
elevate = ["sudo", "-n"]
systemctl = "/opt/canoatest/canoa-instance"
pam_service = "canoatest"
groups = ["canoatestusers"]
min_uid = 1000
idle_seconds = 4
start_seconds = 20
pair_seconds = 8
max_failures_per_pair = 3
failure_floor_seconds = 1
EOF

# The same portal with the network rule: a password only from 10.0.0.0/8, a certificate (pinned through the portal) from anywhere.
# The certificate provider is a library the portal loads, so it is root's: built beside the tests, or named by the second argument.
mtls=${2:-$(ls -t "$target"/deps/libcoqui_access_mtls_plugin*.so "$target"/libcoqui_access_mtls_plugin.so 2>/dev/null | head -n 1)}
[ -f "$mtls" ] || { echo "no coqui_access_mtls plugin library: cargo build -p coqui-access-mtls-plugin, or name it as the second argument" >&2; exit 1; }
install -m 0755 "$mtls" /opt/canoatest/libcoqui_access_mtls_plugin.so
install -d -o canoatestportal -m 0700 /var/lib/canoatest-portal
{
    cat /opt/canoatest/portal.toml
    cat <<'EOF'
authenticators = [
  { provider = "coqui_access_system" },
  { provider = "coqui_access_mtls", library = "/opt/canoatest/libcoqui_access_mtls_plugin.so", enrol = true,
    config = { trust = "pinned", source = { file = { path = "/var/lib/canoatest-portal/certificates.toml", format = "toml" } } } },
]

[[portal.sign_in_policy.config.rule]]
networks = ["10.0.0.0/8"]
allow = [["password"], ["certificate"]]

[[portal.sign_in_policy.config.rule]]
allow = [["certificate"]]
EOF
} > /opt/canoatest/portal_network.toml

# The same portal where a certificate a CA signed names its account by the UPN it carries, through the portal's matcher. The CA
# is the test's own: it writes /etc/canoatest/clients-ca.pem when it runs.
{
    cat /opt/canoatest/portal.toml
    cat <<'EOF'
authenticators = [
  { provider = "coqui_access_system" },
  { provider = "coqui_access_mtls", library = "/opt/canoatest/libcoqui_access_mtls_plugin.so",
    config = { trust = "ca", ca_file = "/etc/canoatest/clients-ca.pem" } },
]

matcher = [
  { by = "name", template = "{san.upn|local}", domain = "corp.example" },
]

[[portal.sign_in_policy.config.rule]]
allow = [["certificate"]]
EOF
} > /opt/canoatest/portal_matcher.toml

# The passwords of the test accounts, and the PAM service that checks them and counts how often it was asked.
printf 'canoatesta:pw-a-test\ncanoatestb:pw-b-test\ncanoatestc:pw-c-test\n' > /etc/canoatest/passwords
chmod 0644 /etc/canoatest/passwords
cat > /opt/canoatest/pam-check.sh <<'EOF'
#!/bin/bash
IFS= read -r -d '' pw || true
echo "$PAM_USER" >> /var/tmp/canoatest-pam-count
while IFS=: read -r user pass; do
    if [ "$user" = "$PAM_USER" ] && [ "$pass" = "$pw" ]; then exit 0; fi
done < /etc/canoatest/passwords
exit 1
EOF
chmod 0755 /opt/canoatest/pam-check.sh
: > /var/tmp/canoatest-pam-count
chmod 0666 /var/tmp/canoatest-pam-count
cat > /etc/pam.d/canoatest <<'EOF'
# pam_exec answers a failure as a system error, so the stack turns its status into a refusal: skip the deny when the check passed.
auth     [success=1 default=ignore] pam_exec.so expose_authtok quiet /opt/canoatest/pam-check.sh
auth     requisite pam_deny.so
auth     required  pam_permit.so
account  required  pam_permit.so
EOF

# The proxy's socket lives here, made for the portal.
mkdir -p /run/canoatest-portal
chown canoatestportal:"$tester_group" /run/canoatest-portal
chmod 0755 /run/canoatest-portal

cat > /etc/systemd/system/canoatest.slice <<'EOF'
[Slice]
MemoryMax=4G
TasksMax=2048
EOF

# One host per person, as that person, with its sockets in a folder only that person and the portal's group can reach.
cat > /etc/systemd/system/canoatest@.service <<'EOF'
[Unit]
Description=Canoa test host for %i

[Service]
Slice=canoatest.slice
User=%i
WorkingDirectory=~
ExecStartPre=+/usr/bin/install -d -m 0711 /run/canoatest
ExecStartPre=+/usr/bin/install -d -o %i -g canoatestportal -m 0710 /run/canoatest/%i
ExecStart=/opt/canoatest/canoa serve --portal /run/canoatest/%i --portal-user canoatestportal --portal-group canoatestusers --supervised
Restart=on-failure
RestartForceExitStatus=75
MemoryMax=1G
TasksMax=512
CPUQuota=200%
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
EOF
systemctl daemon-reload

# A folder for the account outside the group, as the unit would make it, so that only the host's own group check can refuse.
install -d -m 0711 /run/canoatest
install -d -o canoatestc -g canoatestportal -m 0710 /run/canoatest/canoatestc

printf 'canoatestportal ALL=(root) NOPASSWD: /opt/canoatest/canoa-instance\n' > /etc/sudoers.d/canoatest
chmod 0440 /etc/sudoers.d/canoatest
visudo -cf /etc/sudoers.d/canoatest

echo "ready"
