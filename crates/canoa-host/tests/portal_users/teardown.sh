#!/bin/sh
# Removes everything setup.sh made, by name. Safe to run twice.
set -u
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }

for person in canoatesta canoatestb; do
    systemctl stop "canoatest@$person.service" 2>/dev/null
done
rm -f /etc/systemd/system/canoatest@.service /etc/systemd/system/canoatest.slice /etc/sudoers.d/canoatest /etc/pam.d/canoatest
systemctl daemon-reload
rm -rf /opt/canoatest /etc/canoatest /run/canoatest /run/canoatest-portal /var/tmp/canoatest-pam-count
for name in canoatesta canoatestb canoatestportal; do
    if id "$name" >/dev/null 2>&1; then
        userdel --remove "$name" 2>/dev/null || userdel "$name"
    fi
done
rm -rf /home/canoatesta /home/canoatestb
echo "removed"
