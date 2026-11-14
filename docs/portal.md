# The portal

`canoa portal` is the web app for a machine several people use. Each person signs in with their account on that machine and gets
a `canoa` host of their own, started as them, in their home: their agents, their settings, their sessions and their files are
theirs alone. Nothing of one person's is reachable from another's.

It is built with the `portal` feature (Unix only; it links the system's PAM library) and runs behind a proxy that speaks TLS.

```text
browser ──HTTPS──> proxy ──HTTP, loopback──> canoa portal ──socket──> canoa serve --portal   (as the person, in their home)
```

## What the portal does

- **Signs people in** with the operating system's accounts, through coqui-access's `system` provider: PAM checks the password
  under a service of its own and NSS says who the account is, so accounts from SSSD or LDAP are there too. A name that is not a
  plain account name (`a@b`, `a/b`, a leading dot or hyphen) is refused before anything is asked.
- **Counts failed sign-ins** by the browser's address, which the proxy puts in `X-Forwarded-For` (only a header from a
  `trusted_proxies` address is believed, and the last address in it that is not a proxy is the browser's), and by account
  name. Past the limit a sign-in is refused before PAM is asked, so a guessing browser cannot lock an account in a directory.
  Successful sign-ins do not clear the counts.
- **Keeps the browser signed in** with one cookie, `__Host-canoa`: `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, no
  `Domain`. The token is never in a page's script or a response body, and only its digest is kept. A sign-in, a sign-out and a
  channel need the request's `Origin` to be this site's own.
- **Starts the person's host** as the template unit `canoa@NAME.service` when they first use it, through `systemctl`, which a
  polkit rule lets the portal's service user run for those units and nothing else, and **stops it** after `idle_seconds`
  with no channel open. A host the portal finds running when it starts is counted as just used.
- **Introduces the browser's own key to that host.** The host still proves itself to the browser and the browser to the host by
  key, through the portal, which relays each message without opening it: a sign-in is not a pairing.
- **Serves the web app** from `site` to signed-in browsers only.

The portal runs as a service user that owns nothing of anyone's: it never reads a person's files and holds no key of theirs.

## The person's host: `canoa serve --portal`

```sh
canoa serve --portal /run/canoa/NAME --portal-user canoa --supervised
```

Every entry and the control channel listen on a Unix socket in the directory (`entries/ENTRY.sock`, `control.sock`), whatever
canoa.cfg says of an address, and the web app is not served from it. `portal.sock` takes three requests, each a line of JSON
answered with a line of JSON: `{"op":"index"}` (the host's id, method and entries), `{"op":"accept","peer":ID}` (accepts a key
that connected and is waiting, as `canoa accept` does) and `{"op":"revoke","peer":ID}`.

The sockets' mode lets any local user connect. What keeps another account out is the host: it asks the system whose process is
at the other end of each connection and closes any that is not `--portal-user`, before reading a byte. The directory is made
by the unit, as root, under a `/run/canoa` only root can write, so only the person and root can put a socket in it. Every key
the portal introduces is the person's own browser, so each holds every page grant on the control channel (see
[the host](host.md)) without being named in canoa.cfg.

## Setting it up

The portal's settings are a TOML file; every key is optional.

```toml
listen = "127.0.0.1:7890"              # loopback only
site = "/opt/canoa/site"               # the web app's build (flutter build web --csp --no-web-resources-cdn)
socket_dir = "/run/canoa"
unit = "canoa@{user}.service"
pam_service = "canoa"
min_uid = 1000                         # the lowest user id that may sign in
groups = []                            # when any are named, an account must be in one
hosts = ["canoa.example"]              # Host names a request may carry, besides localhost and addresses
trusted_proxies = ["127.0.0.1", "::1"]
client_address_header = "x-forwarded-for"
session_seconds = 28800
idle_seconds = 1800
max_failures_per_address = 5
max_failures_per_user = 10
failure_window_seconds = 900
```

`elevate = ["sudo", "-n"]` puts a command before `systemctl` for `start` and `stop`, for a machine with no polkit; give it one
sudoers line per unit (a wildcard in a unit name is a hole). Every other limit (`max_connections`, `max_sessions`,
`max_relays_per_user`, `request_seconds` and the rest) is a key of the same name with a default.

**The PAM service**, `/etc/pam.d/canoa`, for SSSD:

```text
auth    required pam_sss.so
account required pam_sss.so
```

Without a file, PAM falls back to `other`, which refuses everyone.

**The person's unit**, `/etc/systemd/system/canoa@.service`:

```ini
[Unit]
Description=Canoa host for %i

[Service]
User=%i
WorkingDirectory=~
ExecStartPre=+/usr/bin/install -d -m 0755 /run/canoa
ExecStartPre=+/usr/bin/install -d -o %i -m 0755 /run/canoa/%i
ExecStart=/opt/canoa/canoa serve --portal /run/canoa/%i --portal-user canoa --supervised
Restart=on-failure
RestartForceExitStatus=75
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full
```

The entries every person's host offers go in a canoa.cfg beside the program (`/opt/canoa/canoa.cfg`); each person's own
`~/.config/Canoa/canoa.cfg` goes over it, and is what their settings are written to. `--supervised` and `Restart=` let a page
ask the host to restart itself.

**The portal's unit** runs as a service user, for example `canoa`, with `ProtectHome=yes`, `ProtectSystem=strict` and
`NoNewPrivileges=yes`: it needs no home and no write access.

**The polkit rule** that lets that user start and stop the person units and nothing else, in
`/etc/polkit-1/rules.d/50-canoa.rules`:

```js
polkit.addRule(function (action, subject) {
  if (action.id == "org.freedesktop.systemd1.manage-units" && subject.user == "canoa") {
    var unit = action.lookup("unit");
    var verb = action.lookup("verb");
    if ((verb == "start" || verb == "stop") &&
        /^canoa@[A-Za-z0-9_][A-Za-z0-9._-]{0,31}\.service$/.test(unit)) {
      return polkit.Result.YES;
    }
  }
});
```

**The proxy** passes the browser's address and the upgrade:

```nginx
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
server {
    server_name canoa.example;
    # ... TLS ...
    location / {
        proxy_pass http://127.0.0.1:7890;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
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
| anything else, `GET` | the web app |

A page connects to an entry once, which the host turns away and remembers as waiting, asks `/portal/pair` to accept its key,
and connects again. The key stays paired in the person's home: the next visit, and a restart after an idle stop, need no new
pairing. The bundled web app does not make these calls yet.

## What to know

- A host nobody has a channel open to stops after `idle_seconds`, and so does what it was running: a long task in an agent
  ends with it. Keep a page open, or raise the setting.
- Hosts are started one at a time, each given `start_seconds` to answer.
- Kerberos tickets for the person's processes are not provided by the portal.
- The control channel can change a person's own canoa.cfg sections and uploads; it is the person's own account that does it.
