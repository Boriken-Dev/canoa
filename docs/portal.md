# The portal

`canoa portal` is the web app for a machine several people use. Each person signs in with their account on that machine and gets
a `canoa` host of their own, started as them, in their home: their agents, their settings, their sessions and their files are
theirs alone. Nothing of one person's is reachable from another's.

It is built with the `portal` feature (Unix only; it links the system's PAM library) and runs behind a proxy that speaks TLS.

```text
browser ──HTTPS──> proxy ──HTTP over a Unix socket──> canoa portal ──socket──> canoa serve --portal   (as the person, in their home)
```

## What the portal does

- **Signs people in** with the operating system's accounts, through coqui-access's `system` provider: PAM checks the password
  under a service of its own and NSS says who the account is, so accounts from SSSD or LDAP are there too. A name that does
  not match `[A-Za-z_][A-Za-z0-9._-]{0,31}` is refused before anything is asked, and so are root and any account below
  `min_uid`. **Name the groups** that may sign in (`groups`): with none, every account on the machine that has a password can
  ask for a host, and a host is a process that runs as that account.
- **Limits failed sign-ins** before PAM is asked, in one step that also counts the try, so tries made together cannot all pass.
  An address (an IPv6 one by its /64) may fail `max_failures_per_pair` times for one account and `max_failures_per_address`
  times in all inside `failure_window_seconds`; past either, the try is refused with 429. An account that has failed more than
  `slow_after_user_failures` times from anywhere makes each further try wait `slow_seconds` longer, but is never shut out. A
  failure of the accounts themselves (the directory is down) counts too; a success takes its own count back. Every failed
  sign-in is answered no sooner than `failure_floor_seconds` after it began, so its time does not tell an account that exists
  from one that does not, as long as the difference is shorter than the floor: set it above the slowest a directory answers.
- **Keeps the browser signed in** with one cookie, `__Host-canoa`: `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, no
  `Domain`. The token is never in a page's script or a response body, and only its digest is kept. An account keeps at most
  `max_sessions_per_user` sessions; the oldest goes. A sign-in, a sign-out, a pairing and a channel need the request's `Origin`
  to be this site's own scheme and host, and `/canoa.json`, which starts a host, refuses a browser that says it is a
  cross-site request. The sign-in page sends `Referrer-Policy: same-origin`, since a browser posts `Origin: null` from a page
  that sends no referrer.
- **Starts the person's host** as the template unit `canoa@NAME.service` when they first use it, and **stops it** after
  `idle_seconds` with no channel open. At most `max_hosts` run at once. The account is looked up again each time a host is
  wanted: one that was removed or renamed since the sign-in loses its sessions. A host that is slow to answer delays only its
  own person.
- **Introduces the browser's own key to that host.** The host still proves itself to the browser and the browser to the host by
  key, through the portal, which relays each message (at most `max_message_bytes`) without opening it: a sign-in is not a
  pairing.
- **Serves the web app** from `site` to signed-in browsers only.

The portal never reads a person's files and holds no key of theirs. But **its account can start a host as any account the
unit and the helper or polkit rule allow, and so can reach the host of every person allowed to sign in**: whoever takes over
the portal's process takes the same reach. Treat that account as the most sensitive on the machine and harden it as
below; the layers that keep it from starting a host for anyone outside the sign-in groups, root or a system account are the
host's own check (decisive), the root helper and the portal's own sign-in.

### What the limits do and do not protect

The limits keep one browser from guessing quickly. They do not keep an account from being locked in the directory: guesses from
many addresses add up in the directory's own count. Size `max_failures_per_pair`, `slow_after_user_failures` and the window below
the directory's lockout threshold and observation window (on a domain, read them from its password policy before turning the
portal on), and watch the log for sign-ins refused for their failures.

## The person's host: `canoa serve --portal`

```sh
canoa serve --portal /run/canoa/NAME --portal-user canoa --portal-group canoa-users --supervised
```

Every entry and the control channel listen on a Unix socket in the directory (`entries/ENTRY.sock`, `control.sock`), whatever
canoa.cfg says of an address, and the web app is not served from it. `portal.sock` takes three requests, each a line of JSON
answered with a line of JSON: `{"op":"index"}` (the host's id, method and entries), `{"op":"accept","peer":ID}` (accepts a key
that connected and is waiting, as `canoa accept` does) and `{"op":"revoke","peer":ID}`.

The host **refuses to run**, whatever the portal asked for, as root, in the system and dynamic-user ranges (61184 to 65535,
`nobody` included; the ids a directory gives people are larger or smaller), as a user id below `--portal-min-uid` (default
1000), as an account in none of the `--portal-group` groups (at least one must be named, and the unit fixes them), as an
account whose name (`USER`, which systemd sets for `User=`) is not the last part of the directory, or with a directory its
account does not own. This is the check that decides: the portal, the helper and the polkit rule are only earlier ones. The unit
below makes those agree; a portal asked to start a host for another name would otherwise run it as that account.

The sockets' mode lets any local user connect. What keeps another account out is the host: it asks the system whose process is
at the other end of each connection and closes any that is not `--portal-user`, before reading a byte. The portal does the same
in the other direction: it dials a person's socket for that person's user id and turns away a socket whose maker is anyone
else, so a link in the person's own folder to another person's socket leads nowhere. Every key the portal introduces is the
person's own browser, so each holds every page grant on the control channel (see [the host](host.md)) without being named in
canoa.cfg.

## Setting it up

The portal's settings are a TOML file; every key is optional.

```toml
socket = "/run/canoa-portal/portal.sock"   # what the proxy reaches the portal at
socket_group = "canoa-proxy"                # the group that may connect to it: the proxy's user and this account only
site = "/opt/canoa/site"               # the web app's build (flutter build web --csp --no-web-resources-cdn)
socket_dir = "/run/canoa"
unit = "canoa@{user}.service"
pam_service = "canoa"
min_uid = 1000                         # the lowest user id that may sign in; root never may
groups = ["canoa-users"]               # the groups an account must be in: name them
hosts = ["canoa.example"]              # Host names a request may carry, besides localhost and addresses
trusted_proxies = []                   # proxies of ours that may appear in X-Forwarded-For before the browser's address
client_address_header = "x-forwarded-for"
session_seconds = 28800
max_sessions_per_user = 8
idle_seconds = 1800
max_hosts = 64
max_message_bytes = 4194304
max_failures_per_pair = 3
max_failures_per_address = 10
slow_after_user_failures = 4
slow_seconds = 2
failure_window_seconds = 900
failure_floor_seconds = 2
```

`listen = "127.0.0.1:7890"` opens a loopback TCP address as well, for development: any local user can connect to it, or bind
it first while the portal is down, so what a client says there about the browser's address is never believed and passwords
must not cross it. Behind a proxy, use `socket` alone. The browser's address is believed only on `socket`.

**Start and stop: polkit is the better way.** The portal's account needs the right to start and stop the person units and
nothing else. A polkit rule gives it that with the portal's unit fully hardened; the root helper below works where there is no
polkitd, but costs the portal `NoNewPrivileges` and the empty capability set. Before relying on polkit, confirm on the machine
that `polkitd` runs, and check its systemd version: the rule's safety depends on systemd passing the unit name of a
`start` and `stop` to polkit and not allowing anything else through `manage-units` (a transient unit is created by
`StartTransientUnit`, a different call that carries no unit name the rule could check, and the rule below does not match it).

**The root helper**, with no sudoers wildcard: install
`crates/canoa-host/dist/canoa-instance` as `/usr/libexec/canoa-instance` (root, `0755`), add

```text
canoa ALL=(root) NOPASSWD: /usr/libexec/canoa-instance
```

to a sudoers file, and set `elevate = ["sudo", "-n"]` and `systemctl = "/usr/libexec/canoa-instance"`. It takes
`start|stop canoa@NAME.service` and nothing else, checks NAME against the pattern above, requires the account to be in one of
its `ALLOWED_GROUPS` (set it to the portal's `groups`; with none it starts nothing), and refuses a user id below 1000 or in the
system and dynamic-user ranges (61184 to 65535, which includes `nobody`). It needs the portal's unit to leave out `NoNewPrivileges` and `CapabilityBoundingSet` (see below).

**The polkit rule**, which needs neither, in `/etc/polkit-1/rules.d/50-canoa.rules` (set `elevate` and `systemctl` to nothing
and the portal runs plain `systemctl`):

```js
polkit.addRule(function (action, subject) {
  if (action.id == "org.freedesktop.systemd1.manage-units" && subject.user == "canoa") {
    var unit = action.lookup("unit");
    var verb = action.lookup("verb");
    if ((verb == "start" || verb == "stop") &&
        /^canoa@[A-Za-z_][A-Za-z0-9._-]{0,31}\.service$/.test(unit)) {
      return polkit.Result.YES;
    }
  }
});
```

A rule cannot look at user ids or groups: the host's own refusal (of root, the system ranges, ids below `--portal-min-uid` and any
account outside the `--portal-group` groups) is what stops a unit for `postgres`, `nobody` or a person who may not sign in, and
the helper checks the same. Keep the rule's regex and the helper's pattern identical to the validator's; a test
reads this page for it.

**The PAM service**, `/etc/pam.d/canoa`, for SSSD:

```text
auth    required pam_sss.so
account required pam_sss.so
```

Without a file, PAM falls back to `other`, which refuses everyone.

**The slice** that bounds all the people's hosts together, `/etc/systemd/system/canoa.slice`:

```ini
[Slice]
MemoryMax=24G
TasksMax=8192
```

**The person's unit**, `/etc/systemd/system/canoa@.service`:

```ini
[Unit]
Description=Canoa host for %i

[Service]
Slice=canoa.slice
User=%i
WorkingDirectory=~
ExecStartPre=+/usr/bin/install -d -m 0711 /run/canoa
ExecStartPre=+/usr/bin/install -d -o %i -g canoa -m 0710 /run/canoa/%i
ExecStart=/opt/canoa/canoa serve --portal /run/canoa/%i --portal-user canoa --portal-group canoa-users --supervised
RestrictSUIDSGID=yes
Restart=on-failure
RestartForceExitStatus=75
MemoryMax=2G
TasksMax=512
CPUQuota=200%
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
```

`/run/canoa` is `0711`, so no one can list it, and each person's folder is `0710`, owned by them with the group `canoa`: the
portal's account (and no one else) can reach the sockets in it. Size the limits to the machine and what a person runs; a
person's agents run inside them. `--supervised` and `Restart=` let a page ask the host to restart itself. The entries every
person's host offers go in a canoa.cfg beside the program (`/opt/canoa/canoa.cfg`); each person's own `~/.config/Canoa/canoa.cfg`
goes over it, and is what their settings are written to.

**The portal's account and unit.** The account is a system account with no shell, no password and no scheduled jobs:

```sh
useradd --system --no-create-home --shell /usr/sbin/nologin canoa
passwd -l canoa
# and leave it out of /etc/cron.allow and /etc/at.allow, or name it in cron.deny and at.deny
```

`/etc/systemd/system/canoa-portal.service`:

```ini
[Service]
User=canoa
Group=canoa-proxy
SupplementaryGroups=canoa
RuntimeDirectory=canoa-portal
RuntimeDirectoryMode=0750
ExecStart=/opt/canoa/canoa portal --config /etc/canoa/portal.toml
UMask=077
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectProc=invisible
ProcSubset=pid
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
CapabilityBoundingSet=
RestrictAddressFamilies=AF_UNIX
IPAddressDeny=any
RestrictSUIDSGID=yes
RestrictNamespaces=yes
RestrictRealtime=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallFilter=@system-service
SystemCallArchitectures=native
```

`AF_UNIX` and `IPAddressDeny=any` are for a portal on `socket` whose accounts come through SSSD, which is reached over a Unix
socket; add `AF_INET AF_INET6` and an `IPAddressAllow=` for the directory only if PAM must reach it directly, or if `listen`
is used.

With the sudo helper, leave out `NoNewPrivileges` and `CapabilityBoundingSet=`, which stop `sudo` from working; the polkit rule
needs neither change. `pam_unix` cannot check a password under `NoNewPrivileges` at all, since it asks the setuid
`unix_chkpwd`; `pam_sss` does not, which is another reason to use SSSD. SSSD must answer with the names the portal sees:
set `use_fully_qualified_names = False` in the domain's section of `sssd.conf`, or no one's name matches a unit and a folder.

Set `kernel.yama.ptrace_scope` to 1 or more: it lets a process trace only its own descendants, so a compromised process of
the portal's account cannot attach to the portal's other processes. (Accounts cannot trace one another's anyway without
`CAP_SYS_PTRACE`, which is why it is no help between people.) The portal's group is `canoa-proxy`, whose members are this account and the
proxy's user: put the proxy's user in the group `canoa-proxy`, which is what lets it into `/run/canoa-portal`, and not in `canoa`,
which would let it into every person's folder under `/run/canoa`.

**The proxy** passes the browser's address and the upgrade, over the socket:

```nginx
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
server {
    server_name canoa.example;
    # ... TLS ...
    location / {
        proxy_pass http://unix:/run/canoa-portal/portal.sock:;
        proxy_http_version 1.1;
        proxy_set_header Host $http_host;
        proxy_set_header X-Forwarded-For $remote_addr;   # replaces what the browser sent
        proxy_set_header X-Forwarded-Proto https;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout 1h;
    }
}
```

## The browser's side

| Request | Answer |
| --- | --- |
| `GET /portal/login`, `POST /portal/login` (`user`, `password` as a form) | the sign-in form; a right password is `303 /` with the cookie, a wrong one `401`, too many failures `429` |
| `POST /portal/logout` | ends the session and every channel it holds |
| `GET /canoa.json` | the person's host: `peer`, `method`, `os`, `user` and each entry with a `url` through the portal (`/portal/ws/entry/NAME`, `/portal/ws/control`), starting the host if it is not running |
| `GET /portal/ws/entry/NAME`, `GET /portal/ws/control` | a WebSocket to that entry or the control channel of the person's own host |
| `POST /portal/pair` `{"peer": ID}` | accepts the browser's key once it has connected and waits in the host |
| `GET /manifest.json`, `/favicon.ico`, `/icons/NAME` | given to anyone, signed in or not: a browser fetches the web manifest with no cookie, and they hold nothing of anyone's |
| anything else, `GET` | the web app, to a signed-in browser; to any other `303 /portal/login` |

A page connects to an entry once, which the host turns away and remembers as waiting, asks `/portal/pair` to accept its key,
and connects again. The key stays paired in the person's home: the next visit, and a restart after an idle stop, need no new
pairing.

The bundled web app makes these calls itself. It reads `/canoa.json`, sees `user` and knows it is behind a portal. A `401` from that
request, from `/portal/session` or from `/portal/pair` sends the browser to `/portal/login`. Its first connection to the control
channel is turned away, so it checks `GET /portal/session` (still signed in), posts `{"peer": ID}` to `/portal/pair` with its own key
and connects again. The rail shows the account and a **Sign out**, which posts `/portal/logout` and returns to the sign-in form. No
token is in the page: the cookie is `HttpOnly`, and `fetch` and the WebSocket send it by themselves.

## What to know

- A host nobody has a channel open to stops after `idle_seconds`, and so does what it was running: a long task in an agent
  ends with it. Keep a page open, or raise the setting.
- A person's host is started once, however many requests come together, and given `start_seconds` to answer.
- Kerberos tickets for the person's processes are not provided by the portal.
- The control channel can change a person's own canoa.cfg sections and uploads; it is the person's own account that does it.
- The portal opens no PAM session, so nothing makes a home directory: it must exist before the person's first sign-in.

## Before turning it on

A checklist for the machine, in order:

1. **Homes.** Make each person's home before their first sign-in (`pam_mkhomedir` or `oddjob-mkhomedir` in a login service, or by
   hand): the unit's `WorkingDirectory=~` fails without it.
2. **Groups.** Create the sign-in group (`canoa-users`), put the people in it, and name it in the portal's `groups`, the
   unit's `--portal-group` and the helper's `ALLOWED_GROUPS`.
3. **SSSD.** `use_fully_qualified_names = False`, and `id NAME` answering for a person from the portal's account.
4. **Polkit.** Confirm `polkitd` runs and note `systemctl --version`; check that the rule starts `canoa@NAME.service` for the
   portal's account and nothing else (`systemctl start canoa@root.service` must be refused). Use the helper if there is no
   polkit, with the changes above.
5. **The directory's lockout policy.** Read the domain's password policy and set the limits below its threshold.
6. **Hardening.** The portal's unit, the slice, the unit's limits, `ptrace_scope`, the account without shell, password or jobs.
7. **The proxy** reaches the portal's socket and sets `Host`, `X-Forwarded-For` and `X-Forwarded-Proto`.
