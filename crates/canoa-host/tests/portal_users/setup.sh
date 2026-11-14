#!/bin/sh
# Makes what tests/portal_users.rs needs on a Linux machine with systemd, as root: three throwaway accounts (canoatestportal for
# the portal, canoatesta and canoatestb for the two people), a PAM service that checks a password file of its own, the template
# unit, a sudoers rule that lets the portal start and stop the two test units, and a copy of the built programs.
#
#   sudo sh setup.sh TARGET_DIR      # TARGET_DIR holds canoa and examples/stdio_agent, such as target/debug
#   sudo sh teardown.sh              # removes all of it
#
# Every name starts with canoatest. Nothing here touches another account, sssd, or the real PAM services.
set -eu

target=${1:?"usage: setup.sh TARGET_DIR"}
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }

for name in canoatestportal canoatesta canoatestb; do
    if id "$name" >/dev/null 2>&1; then
        echo "$name exists already; run teardown.sh first" >&2
        exit 1
    fi
done

useradd --system --no-create-home --shell /usr/sbin/nologin canoatestportal
useradd --create-home --shell /usr/sbin/nologin canoatesta
useradd --create-home --shell /usr/sbin/nologin canoatestb
chmod 0700 /home/canoatesta /home/canoatestb

install -d -m 0755 /opt/canoatest /opt/canoatest/site /etc/canoatest
install -m 0755 "$target/canoa" /opt/canoatest/canoa
install -m 0755 "$target/examples/stdio_agent" /opt/canoatest/stdio_agent
echo '<!doctype html>the app' > /opt/canoatest/site/index.html
# What the test runs as one account to reach another's sockets, from where every account can read it.
install -m 0644 "$(dirname "$0")/uds.py" /opt/canoatest/uds.py

# The host's entries, in the layer beside the program; each person's own canoa.cfg goes over it.
cat > /opt/canoatest/canoa.cfg <<'EOF'
[host.agents.fixture]
program = "/opt/canoatest/stdio_agent"
EOF

cat > /opt/canoatest/portal.toml <<'EOF'
listen = "127.0.0.1:7890"
site = "/opt/canoatest/site"
socket_dir = "/run/canoatest"
unit = "canoatest@{user}.service"
elevate = ["sudo", "-n"]
pam_service = "canoatest"
min_uid = 1000
idle_seconds = 4
start_seconds = 20
pair_seconds = 8
max_failures_per_address = 3
EOF

# The passwords of the test accounts, and the PAM service that checks them and counts how often it was asked.
printf 'canoatesta:pw-a-test\ncanoatestb:pw-b-test\n' > /etc/canoatest/passwords
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

# One host per person, as that person, with its sockets in a folder only that person (and root) can write.
cat > /etc/systemd/system/canoatest@.service <<'EOF'
[Unit]
Description=Canoa test host for %i

[Service]
User=%i
WorkingDirectory=~
ExecStartPre=+/usr/bin/install -d -m 0755 /run/canoatest
ExecStartPre=+/usr/bin/install -d -o %i -m 0755 /run/canoatest/%i
ExecStart=/opt/canoatest/canoa serve --portal /run/canoatest/%i --portal-user canoatestportal --supervised
Restart=on-failure
RestartForceExitStatus=75
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
EOF
systemctl daemon-reload

systemctl_path=$(command -v systemctl)
{
    printf 'canoatestportal ALL=(root) NOPASSWD:'
    sep=' '
    for person in canoatesta canoatestb; do
        printf '%s%s start canoatest@%s.service, %s stop canoatest@%s.service' "$sep" "$systemctl_path" "$person" "$systemctl_path" "$person"
        sep=', '
    done
    printf '\n'
} > /etc/sudoers.d/canoatest
chmod 0440 /etc/sudoers.d/canoatest
visudo -cf /etc/sudoers.d/canoatest

echo "ready"
