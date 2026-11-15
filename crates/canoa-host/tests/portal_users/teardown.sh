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
for name in canoatesta canoatestb canoatestc canoatestportal; do
    if id "$name" >/dev/null 2>&1; then
        # A host that was just stopped may still be exiting, and userdel refuses an account with a process.
        for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -u "$name" >/dev/null 2>&1 || break; sleep 1; done
        userdel --remove "$name" 2>/dev/null || userdel "$name"
    fi
done
rm -rf /home/canoatesta /home/canoatestb /home/canoatestc
getent group canoatestusers >/dev/null && groupdel canoatestusers
echo "removed"
