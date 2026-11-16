# The portal

`canoa portal` is the web app for a machine several people use. Each person signs in with their account on that machine and gets
a `canoa` host of their own, started as them, in their home: their agents, their settings, their sessions and their files are
theirs alone. Nothing of one person's is reachable from another's.

It is built with the `portal` feature (Unix only; it links the system's PAM library) and runs behind a proxy that speaks TLS.

```text
browser ──HTTPS──> proxy ──HTTP over a Unix socket──> canoa portal ──socket──> canoa serve --portal   (as the person, in their home)
```

## What the portal does

- **Signs people in** through an ordered list of coqui-access authenticators (`authenticators`, see [Signing in](#signing-in)).
  By default that is the operating system's accounts alone, through the `system` provider: PAM checks the password under a
  service of its own and NSS says who the account is, so accounts from SSSD or LDAP are there too. Whatever method signs
  someone in, it ends at an account of the operating system, and that account must pass the same checks: a name that does not
  match `[A-Za-z_][A-Za-z0-9._-]{0,31}` is refused before anything is asked, and so are root and any account below `min_uid`.
  **Name the groups** that may sign in (`groups`): with none, every account on the machine that has a password can ask for a
  host, and a host is a process that runs as that account. A method adds a way in, never a person.
- **Limits failed sign-ins** before any provider is asked, for every method, in one step that also counts the try, so tries made together cannot all pass.
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
  pairing. The portal pings each channel every `ping_seconds` (20) so a proxy that cuts a silent connection keeps it, and closes
  one that has answered none of two pings in a row.
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

**Measure the failure floor.** `failure_floor_seconds` (default 2) hides how long a directory takes to answer only if it is
longer than the slowest answer. Before turning the portal on, time wrong passwords through the PAM service for an account that
exists and for one that does not, many times each (`pamtester canoa NAME authenticate`, or a loop over `su`), and set the floor
above the slowest, with margin: the default was not measured against your directory. A directory that sometimes takes longer
than the floor shows through those answers.

## Signing in

`authenticators` lists the providers, in order. Each is a coqui-access provider and the configuration it is built from, checked
against the provider's own schema when the portal starts:

```toml
authenticators = [
  { provider = "coqui_access_system" },
  { provider = "coqui_access_password", library = "/opt/canoa/libcoqui_access_password_plugin.so",
    config = { source = { file = { path = "/var/lib/canoa-portal/passwords.toml", format = "toml" } } } },
  { provider = "coqui_access_mtls", library = "/opt/canoa/libcoqui_access_mtls_plugin.so", enrol = true,
    config = { trust = "pinned", source = { file = { path = "/var/lib/canoa-portal/certificates.toml", format = "toml" } } } },
]
```

- `coqui_access_system` is built in. For the keys it leaves out (`pam_service`, `min_uid`, `groups`) it takes the portal's own
  settings, so the default list, `[{ provider = "coqui_access_system" }]`, signs in as the portal always has.
- Any other provider is a Guatiao library, named by `library` (an absolute path; the portal's account must be able to read
  it). The portal runs its code, so it refuses a library unless the file and every directory above it belong to root and no
  group or other can write them; `libraries_any_owner = true` lifts that, with a warning in the log at every start. The
  providers of
  [coqui-access](https://github.com/Boriken-Dev/coqui-access) are `coqui-access-password-plugin`, `coqui-access-mtls-plugin`
  and the others there; build them with `cargo build --release -p coqui-access-mtls-plugin`.
- Each credential goes to the providers that take it, in list order. A name and a password go to those that take a password:
  a provider that answers `Unknown` passes the credential on, one that answers `Invalid` ends the try. The system provider
  answers `Unknown` for a name it does not admit and `Invalid` for a wrong password. `coqui_access_password` answers `Unknown`
  for a name its file does not hold and `Invalid` for a wrong password of one it does, so in either order the providers
  after it are asked about the other names. A client certificate goes to those that take a request. A provider that takes
  bearer keys (`coqui_access_key`) is left out with a note in the log: a key is not a sign-in for the page.
- A provider answers with who the credential belongs to. The portal then asks the operating system's directory for that
  account under `min_uid` and `groups`; a name no operating-system account has, or an account outside the groups, is refused
  like a wrong password. A password file or a certificate table can therefore name an operating-system account and nothing else.
- The throttle and the failure floor above apply to every method, and a wrong password, an unknown account and an account the
  portal does not admit all read the same.
- **Trusting a CA.** `trust = "ca"` believes every certificate the CA signed that passes the provider's `subject` and `san`
  globs, and **does not check revocation**: a certificate that was revoked signs in until it expires. Whenever a CA is trusted,
  limit it with those globs (`san = ["upn:*@corp.example"]`, say) and give the matcher a rule bound to the same realm, so a
  certificate of the CA for another purpose or another realm is nobody here. The provider `coqui_access_proxy` is refused when
  the portal starts: it believes a certificate a header carries, whatever the proxy checked.
- A certificate provider that names no account itself (a CA-trusted certificate with no `account` picks) answers by the
  certificate's claims, and the portal's **matcher** ties them to an account in a directory: the directory, not the
  certificate, decides the account and its groups. The rules are tried in order and each renders a template over the claims
  (`san.upn`, `san.email`, `subject.cn`, ...). The defaults are the UPN as a name, then the email address as an email; the
  subject's common name is whatever the issuer let the holder type, so it is not among them. The account found is then admitted
  by the operating system under `min_uid` and `groups` like any other. Exactly one account is a match; more than one, or none,
  is refused like a wrong certificate, through the throttle and the floor, and the log says which without quoting the claims:

  ```toml
  # corp.example's UPNs are account names there: alice@corp.example signs in as alice. `local` strips a realm, so it names
  # the one it strips; another realm's alice is nobody here.
  matcher = [
    { by = "name", template = "{san.upn|local}", domain = "corp.example" },
  ]
  ```

  A rule with `local` and no `domain` stops the portal from starting. A UPN or email with more than one `@` makes no candidate for
  a rule that strips or takes the realm (the matcher's own rule, for every rule): the realm is the part after the last, and
  what remains of two would be a name that holds an `@`. The matcher looks the account up among the operating system's
  accounts, which answer `name`, `email` and `id` lookups, and the portal then admits that account under `min_uid` and `groups`.
  **There is no directory setting yet:** a directory that searches (LDAP) names a person by its distinguished name, not by the
  numeric user id the operating system's account has, so a match there could not be told to be the same account; until a
  provider gives the user id, the matcher asks the system's accounts only. Only a certificate that arrived on the proxy's
  socket reaches the matcher at all. Pin certificates (`trust = "pinned"`) to skip it.

`GET /portal/methods` (no cookie needed) answers `{"methods": ["password", "certificate"]}`: the ways in the sign-in policy
(below) allows this connection, among the ones this portal has. The certificate method is listed only on the proxy's socket.
The sign-in page is built from the same list:
a password form, and a **Sign in with your certificate** button when the certificate method is on, each with its own error. A
`429` carries `Retry-After` and the page counts it down and holds the buttons; the page's one script, `/portal/login.js`, does
only that, and the page's policy allows scripts and styles from the portal itself and nothing inline.

### Signing in by certificate

The browser's certificate reaches the portal only in a header the proxy sets, and only on the proxy's socket: `certificate_header`
(default `x-client-cert`) carries it as base64 of the DER, leaf first, several certificates separated by commas, and a request
with the header twice, a header over 16 KiB, bad base64 or more than eight certificates is refused. The portal hands the chain to
the providers that take a request as a transport would after verifying the key, and the **provider** decides whether to trust
it: `coqui_access_mtls` pins certificates by their SHA-256 fingerprint (`trust = "pinned"`), accepts those a CA signed
(`"ca"`, which needs the chain, so the proxy must forward the intermediates) or either (`"pinned-or-ca"`). Then the account is
looked up as for a password.

The sign-in page says why a certificate try failed when that is no secret. A try that carried **no certificate** (the browser
chose none: Chrome and Edge remember a choice, "none" included, until the browser is restarted) is answered with a notice to
check that the card is inserted and to restart the browser; it is not counted by the throttle and no provider is asked, so a
person cannot lock themselves out by clicking again. An **expired** certificate and one **pinned to no account** each have their
own notice, still count as failures, and name no account. The log says which of the three it was, never the certificate.

A pinned certificate is refused once it is past its validity dates, whatever the provider says. With `trust = "ca"` or
`"pinned-or-ca"`, always set the provider's `san` or `subject` globs to the names your CA issues for people, and issue those
certificates with the `clientAuth` extended key usage only: a CA with no restriction signs for anyone who can get a certificate
from it, and the provider reads the account's name from the certificate's fields.

`coqui_access_proxy` is not a certificate sign-in here: it trusts whatever the proxy forwarded and checks no CA, which is right
behind a proxy that verified the certificate and wrong behind one that does not.

**Enrolling a certificate.** With `enrol = true` on the certificate provider, a person who signed in **by password** and whose
request carries a certificate that is pinned to no one is taken to `/portal/certificate`, which shows the certificate's subject
and SHA-256 fingerprint and **Use this certificate to sign in**. Confirming pins the fingerprint to their account in the
provider's own store (its account kind), under the portal's state directory, never in the person's home; after that the
certificate alone signs them in. The same page shows whether a certificate is pinned to the account and **Remove it**. The rules:

- Pinning needs the password. The session that pinned it must have been signed in by password at most `enrol_seconds` ago
  (default 600) **with this same certificate on that request**; for any other certificate, or any other session (one a
  certificate opened, or a stolen cookie presenting its own certificate), the form asks for the password again, which goes
  through the sign-in throttle. A session a certificate opened can remove a pin.
- A certificate pinned to another account cannot be enrolled, an expired one is never pinned, and a pin an administrator
  disabled cannot be replaced or removed from the page.
- The pin records the user id the account has. A pin made for another id (the account was deleted and made again under the same
  name) is not the account's and signs no one in; the person may pin again.
- Sessions a certificate opened end when the pin they used is removed or replaced. Every pin and removal is logged at `warn` with
  the certificate's fingerprint and the browser's address.
- An account has one pinned certificate: enrolling another replaces it, which is how a renewed card is re-enrolled. A key is
  pinned only through the certificate's fingerprint (`coqui_access_mtls` offers no public-key pin), so a renewed certificate
  with the same key has to be enrolled again.
- Every unknown fingerprint a certificate sign-in presents is also written to the provider's waiting list (a file beside the
  table, bounded by the provider's own limits, recorded against the browser's address), which an administrator can read and
  accept. Never accept a waiting entry on its say-so: first compare its fingerprint, out of band, with the one the person
  reads from their own certificate. An entry is not the person's.
- Pin only what the person confirmed on the page: the subject is whatever the certificate's maker wrote.

**What the proxies must do** for the certificate method, when an HAProxy terminates the public TLS and nginx on the
portal's machine forwards to the portal's socket:

1. HAProxy **asks** for a client certificate and does not judge it. On the `bind` line of the frontend for the portal's name:
   `ssl crt PUBLIC.pem ca-file ANY-CA.pem verify optional ca-ignore-err all crt-ignore-err all`. `ca-file` must exist for
   `verify optional` to be accepted; its contents decide nothing here, since the portal pins certificates itself. HAProxy still
   makes the browser prove it holds the certificate's key during the handshake, which is what the portal relies on. A browser
   with no certificate connects normally and signs in by password.
2. HAProxy sets the header on the forwarded request, and first removes any the browser sent:

   ```text
   http-request del-header X-Client-Cert
   http-request set-header X-Client-Cert %[ssl_c_der,base64] if { ssl_c_used }
   ```

   `ssl_c_der` is the leaf only, as DER, and `base64` writes it on one line: that is the portal's encoding (`DER, base64,
   leaf first`). No CA is named anywhere in HAProxy's decision; pinned trust therefore needs nothing else of it.
3. nginx passes the header to the portal only from HAProxy, and drops it from anyone else. The `map` goes in the `http`
   context (a `conf.d` file); every `location` that proxies to the portal sets **all** the proxy headers (see the sample
   under "The proxy" below, which has `X-Client-Cert` in it).

   ```nginx
   map $realip_remote_addr $canoa_client_cert {
       HAPROXY_ADDRESS  $http_x_client_cert;   # replace with HAProxy's address as nginx sees it
       default          "";
   }
   ```

   `$realip_remote_addr` is the address the connection came from before `set_real_ip_from HAPROXY_ADDRESS; real_ip_header
   X-Forwarded-For;` replaced `$remote_addr`, so a browser that reaches nginx directly cannot name itself HAProxy. A certificate
   is about 1 to 3 KiB of base64; nginx's default header buffers (4 of 8 KiB) hold it.

   **The inheritance trap.** nginx inherits `proxy_set_header` from the `server` block into a `location` only when the
   location sets none of its own. A `location` that sets any (the sample's `Host`, say) inherits none, so a
   `proxy_set_header X-Client-Cert` at server level is silently not sent from it, and the person is never offered the
   certificate; the reverse, with the certificate header set only in a location, leaves the other locations without it and
   without the browser's own copy removed. Keep every `proxy_set_header` in one place: all in each proxying location (as in
   the sample), or all at server level with no location setting any.
4. In the portal's settings, `certificate_header` is the same name (lower case in the file), `trusted_proxies` is as before, and
   the certificate provider has `enrol = true` and `trust = "pinned"` (add `ca` only when the chain is forwarded). The portal's
   unit gets `StateDirectory=canoa-portal` and `StateDirectoryMode=0700`, since `ProtectSystem=strict` leaves nothing else
   writable, and the table lives there (`/var/lib/canoa-portal`): the files hold who may sign in, and the portal warns at start
   about a `source` or `waiting` file a group or others can read. Build the plugin library from coqui-access and install it
   where `library` says, owned by root.
5. **The HAProxy to nginx hop** is where the header is believed, so it must be authenticated, not only encrypted: nginx on
   the HAProxy-facing server takes TLS 1.3 only (`ssl_protocols TLSv1.3;`) and requires HAProxy's own client certificate
   (`ssl_verify_client on; ssl_client_certificate HAPROXY-CLIENT-CA.pem;`), and HAProxy's backend line presents it and verifies
   nginx's (`server portal ADDRESS:PORT ssl crt HAPROXY-CLIENT.pem ca-file NGINX-CA.pem verify required sni str(PORTAL-NAME)
   ssl-min-ver TLSv1.3`). Without that, anything that can reach nginx's port from HAProxy's address, or become it, can send the
   header. Terminate the browser's side in HAProxy for the portal's name on a row of its own, with `ssl-min-ver TLSv1.3` there
   too, so a client that cannot do TLS 1.3 is not offered the certificate at all.

The portal believes the header only because it arrived on `socket`. Anything that can write to that socket (the proxy's group)
can sign in as anyone whose certificate is pinned, by sending the header itself.

### Which ways in a connection may use

`sign_in_policy` decides which of the ways in above a connection may use, and whether the steps a sign-in has passed are enough.
It is a coqui-access
[sign-in policy](https://github.com/Boriken-Dev/coqui-access/blob/main/docs/sign-in.md): `provider` defaults to the rules
provider `coqui_signin_rules`, which is built in, and any other comes from a `library` as an authenticator does. A password
only from inside a network and a certificate from anywhere:

```toml
[[sign_in_policy.config.rule]]
networks = ["10.0.0.0/8"]
allow = [["password"], ["certificate"]]

[[sign_in_policy.config.rule]]
allow = [["certificate"]]
```

The rules are tried in order; the first whose conditions hold decides. An address outside 10.0.0.0/8, an IPv6 address and an
address the portal could not determine all fall to the second rule, so put the strictest last. Each `allow` is a list of
chains; a chain is a set of methods that must all succeed for one account. With no `sign_in_policy`, each way in the portal has
is enough alone, from anywhere.

- **The page offers only what is allowed.** The sign-in page and `GET /portal/methods` list the methods the policy allows for
  that connection, in the policy's order, and nothing else.
- **A refused method is refused first.** A sign-in by a method the policy does not offer from where the browser is gets `403`
  before the throttle counts it and before any provider is asked: a password from outside 10.0.0.0/8 never reaches PAM or a
  directory, and counts against no address and no account. The log line names the method, the address and the rule, never a
  name that was typed.
- **A valid credential is decided again.** Once a provider accepts it, the policy is asked once more with the account it named
  (its user id, name and groups) and the step passed. If that is enough, the session opens and records the steps; if it is not
  (a chain that needs more, or a rule for that account that does not allow the method), the answer, the count and the delay
  are those of a wrong password, so nothing tells the two apart. So is a policy that cannot decide at that point: it answers
  as a refusal and the log says why. A password retyped at `/portal/certificate` is a password
  sent: the policy decides whether this connection may send one before anything is asked.
- **A session is decided again when it is used.** `/canoa.json`, `/portal/pair`, each new channel, `/portal/session` and
  `/portal/certificate` decide with the steps the session was opened with, the account's groups now and the address the
  browser is at now. When the directory cannot say which groups the account is in, that request answers `503` and the
  session stands. A session that is no longer enough ends (`401`; the app goes
  back to the sign-in page): a password session carried out of 10.0.0.0/8 ends at its next use, and a laptop that moves from
  the office to elsewhere signs in again with its certificate. A certificate session stands wherever it is.
- **The address** is the one the proxy reports: on `socket`, the last address in `client_address_header` that is not in
  `trusted_proxies`; on `listen`, the connection's own peer. With nginx in front, either of these sets it (the first is the
  portal's default):

  ```nginx
  proxy_set_header X-Forwarded-For $remote_addr;
  proxy_set_header X-Real-IP $remote_addr;      # with client_address_header = "x-real-ip"
  ```

  When the header is missing, is not an address, or every address in it is in `trusted_proxies`, the address is **unknown**:
  it matches no network, and the policy is told so, never a placeholder. Addresses are compared as the address they are
  (`::ffff:10.0.0.1` is `10.0.0.1`). **List every proxy in front of the portal in `trusted_proxies`.** With HAProxy in front of
  nginx, nginx puts the address it was connected from in the header, and that is HAProxy's own, inside your network, whenever
  HAProxy did not add the browser's: unless HAProxy is listed, the portal takes it for the browser and every browser reads as
  inside. Listed, a hop that carries no browser address reads as unknown, which is outside.
- **A policy that cannot decide offers nothing.** The page, `/portal/methods` and the sign-in answer `503` and the log says
  why; the portal never falls back to allowing.
- **A chain of more than one method** (`["password", "totp"]`) is asked for step by step; see "A second step" below.
- **Providers are told apart by their id.** The policy names a provider by the id its `authenticators` entry has, so two
  entries with the same id cannot be told apart: the first is asked, and naming the id never reaches the second.
- The log says which policy is in force when the portal starts: `sign-in policy: 2 rules`, the provider's id, or `none: every
  method from anywhere`.

### A second step

A chain of more than one method makes a sign-in take more than one step. The one-time code (`totp`) and the recovery code
(`recovery_code`) are the methods that follow another: they are checked for the account an earlier step named, so the policy
never offers them first.

```toml
authenticators = [
  { provider = "coqui_access_system" },
  { provider = "coqui_access_totp", library = "/opt/canoa/libcoqui_access_totp_plugin.so",
    config = { source = { file = { path = "/var/lib/canoa-portal/totp.toml", format = "toml" } }, issuer = "Canoa" } },
]

# From inside the network only, a person who has no code yet may set one up while signing in, with their password. Nothing else
# lets a sign-in do that: a rule for `enrol` does not (see "Setting a code up").
[[sign_in_policy.config.rule]]
purposes = ["enrol_first"]
networks = ["10.0.0.0/8"]
allow = [["password"]]

# Setting a code up again, signed in, needs the password typed again.
[[sign_in_policy.config.rule]]
purposes = ["enrol"]
allow = [["password"]]

# Outside the network a code follows the password; inside it the password is enough.

[[sign_in_policy.config.rule]]
networks = ["10.0.0.0/8"]
allow = [["password"]]

[[sign_in_policy.config.rule]]
allow = [["password", "totp"], ["password", "recovery_code"]]
```

The one-time code provider needs a `source` file: the portal asks it to check codes and to set them up, and the file is what
the two share. It is the factor itself, a readable secret for each account, so keep it where only the portal's account can read
it (the unit's `StateDirectory=canoa-portal`, mode `0700`). Install the library like any other, owned by root.

- **A pending sign-in.** When the steps passed are not enough and the policy offers more, the portal keeps what has passed in
  memory and gives the browser the cookie `__Host-canoa-pending` (`HttpOnly`, `Secure`, `SameSite=Strict`) naming it. The page
  then asks for the next step: a code field (`autocomplete="one-time-code"`, `inputmode="numeric"`), **Use a recovery code**
  where the policy offers one, a line saying the browser must present its certificate where that is the step, and **Start
  over**. `GET /portal/methods` with the cookie answers `{"step": "next", "methods": [...]}`. When a sign-in by password
  carries a certificate on the proxy's socket and the next step may be a certificate, that is tried in the same request.
  The portal restarting ends every pending sign-in.
- **It ends.** After `pending_seconds` (default 300) or `max_pending_tries` tries (default 5), right or wrong, the sign-in ends
  and the page starts over; the try that comes after the last is not checked.
- **A step names the same account.** A code is presented for the account the first step named, and nothing the browser sends
  names another. A password or certificate that names a different account fails and counts.
- **Guessing a code is bounded four ways.** Each try is counted like a wrong password for the address and the account, which
  stops it at `max_failures_per_pair`; each also counts for the account, from any address, and after
  `max_factor_failures_per_person` failures (default 5) in `failure_window_seconds`, or `max_factor_failures_per_day`
  (default 20) in `factor_day_seconds` (a day), further tries for that account are refused without asking the provider,
  whichever address they come from and however often the person starts over; and the provider delays an account itself after
  wrong codes in a row. **Keep the provider's own lockout on** (`lockout_after`, 5 by default): it is the one count that also
  holds if the portal's memory is lost by a restart, and `lockout_after = 0` leaves a restart giving a guesser a fresh count. A
  refused try is a failed one for its address, and the portal logs the first of them at `warn`, saying which limit and from
  where, never a name: it is what someone guessing at an account looks like. The cost is that someone who knows the password
  can keep the code step shut for that person while they keep failing; the window's failures leave it as time passes, so it is
  not for good, but the day's cap holds a day. **Changing the account's password is the remedy for a lock-out**: whoever was
  guessing needed the old one, and the person's counts, which the portal keeps in memory, do not change with it. An
  administrator clears them with `canoa portal --config FILE factor unlock NAME`: it writes the time to `<source>.unlock`, a
  running portal clears the account's counts at its next try for it (no restart, and once for each unlock), and the provider's
  own delay of the account ends with it. Restarting the portal clears every count. The first refusal in a window is logged at
  `warn`, naming the account, the address and which limit, and the ones after it at `info`. Every failed step is answered no sooner than `failure_floor` after the request began.
- **A session does not outlive its code.** A session opened with a code or a recovery code remembers which code it was, and is
  looked at again at each use and, for a channel left open, every few seconds: it ends when the account has no code, when an
  administrator disabled or reset it, and when the code is another one than the session was made with (one set up again). The
  record of when a code was made is in whole seconds, so a code replaced within the second of the one before is not told apart.
- **A code is the account's, by name and user id.** A code is kept under both (`1000:alice`), so a user id made again for
  another account does not find the code of the one before, and a recovery code of one is no recovery code of the other. An
  account that has no code but whose user id has a record or a reset mark under **another name** (it was renamed, or another was
  made in its place) counts as reset: it is not open to setting a first code up for itself, and an administrator's `enrol`
  lifts that. A code table that cannot be read is neither "has" nor "has not": a request answers `503` and a session or channel
  stays until it can be.

#### Setting a code up

`GET /portal/factor` says whether the account has a code. The policy decides who may set one up, and **with no rule for it only
an administrator does**. Two purposes, because they are not the same trust:

- A signed-in person is asked as `enrol`: the policy says which factors they must show again (the password typed again, say);
  they do, as at sign-in and through the same counts, and only then is a code set up. The cookie of a session alone sets
  nothing up.
- A sign-in that has passed its first step and is asked for a code the account does not have is asked as `enrol_first`, and
  **only a rule that names `enrol_first` lets it**; a rule for `enrol` does not. There is no session yet, so whoever holds the
  steps passed (a stolen password, say) is who would set the code up, and the person it belongs to is shut out. Write that
  rule for the networks and factors you trust with it, as the example above does, or leave it out and have an administrator
  set codes up. An account that has a code is never offered this: its password alone cannot replace the code. A first setup
  during a sign-in is logged at `warn`, with the address.
- The page shows the `otpauth://` link as text and as a QR code (drawn by the portal as an inline image, with no script), the
  recovery codes once, and a field for the first code from the app. Nothing is a factor until that code confirms it, and it is
  spent. In a sign-in, the confirming code is also the step the sign-in was waiting for. Starting again replaces the setup
  waiting; the code in use keeps working until a new one is confirmed.
- Whoever holds the first factor can set up a code for an account that has none, where an `enrol_first` rule allows it. That is
  what such a rule says.

`canoa portal --config FILE factor ...`, run as the portal's account, is the administrator's way:

```sh
canoa portal --config /etc/canoa/portal.toml factor list                    # one JSON line per account: id, user, enrolled, disabled, waiting, reset
canoa portal --config /etc/canoa/portal.toml factor enrol alice              # prints the link and the recovery codes, once
canoa portal --config /etc/canoa/portal.toml factor enrol alice --code 123456  # the first code from the app confirms it
canoa portal --config /etc/canoa/portal.toml factor reset alice              # removes the code, its recovery codes and any setup waiting
canoa portal --config /etc/canoa/portal.toml factor reset --id 1000:alice    # the same by the id `list` shows, for an account that is gone
canoa portal --config /etc/canoa/portal.toml factor unlock alice             # clears the account's failure counts, after its password was changed
```

After `reset`, the sessions that used the code end at their next use (a channel within a few seconds), and the account is
**marked**: it sets no code up for itself, whatever rule there is, until an administrator sets one up with `enrol` and
`--code`. Reset is for a code that was lost or may be someone else's, which usually means the password was too: **change the
account's password as well**, or the one who held both is back in with the password alone. The link and the codes `enrol`
prints are the factor: treat that output as a secret. No log line, error message or page other than that answer holds a link, a
code or a recovery code.

The mark is written before the code is removed, under a lock, so a reset that is interrupted leaves the account marked with its
code, not unmarked without one. The command refuses to run as an account that does not own the one-time code provider's files (`source`, `pending`, `waiting`, and their lock, reset and unlock files) and the folders they are in: it would make files the portal's account cannot read, and every code step would answer `503`. Files of other providers, a password table under a folder root owns, say, are not its concern. The provider is found by `totp` in its id or its library's file name. Run it as the portal's account, for instance
`sudo -u canoa`.

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
else, so a link in the person's own folder to another person's socket leads nowhere. Whoever reaches the control socket has signed in
as that person, so each key it introduces holds every page grant on the control channel (see [the host](host.md)) without being
named in canoa.cfg: the limit is the portal's sign-in, not the key.

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
authenticators = [{ provider = "coqui_access_system" }]   # see "Signing in"
certificate_header = "x-client-cert"   # where the proxy puts the browser's certificate (base64 DER)
enrol_seconds = 600                    # how long after a password sign-in a certificate may be pinned, and after showing the enrol factors a code set up
trusted_proxies = []                   # proxies of ours that may appear in X-Forwarded-For before the browser's address
client_address_header = "x-forwarded-for"   # or the header your proxy puts the browser's address in
# sign_in_policy: which ways in a connection may use, from where; a table, see "Which ways in a connection may use"
session_seconds = 28800
max_sessions_per_user = 8
idle_seconds = 1800
max_hosts = 64
max_message_bytes = 4194304
ping_seconds = 20                      # a ping on each relayed channel this often (0: none); two unanswered close it
max_failures_per_pair = 3
max_failures_per_address = 10
slow_after_user_failures = 4
slow_seconds = 2
failure_window_seconds = 900
failure_floor_seconds = 2
pending_seconds = 300                  # how long a sign-in that passed a step waits for the next
max_pending_tries = 5                  # tries it allows at the next steps
max_factor_failures_per_person = 5     # failed second-step tries for one account, from anywhere, in the window
max_factor_failures_per_day = 20       # and in factor_day_seconds
factor_day_seconds = 86400
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

**The proxy** passes the browser's address and the upgrade, over the socket. Every `proxy_set_header` the portal needs is in the
location, because a location that sets any inherits none from the server (see the trap under "Signing in by certificate"); the
last one is for the certificate method and is empty, hence not sent, when `$canoa_client_cert` is empty (leave it out and the
`map` above with the method off):

```nginx
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
map $realip_remote_addr $canoa_client_cert {
    HAPROXY_ADDRESS  $http_x_client_cert;   # replace with the proxy in front of nginx, or leave the map out
    default          "";
}
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
        proxy_set_header X-Client-Cert $canoa_client_cert;
        proxy_read_timeout 1h;
    }
}
```

## The browser's side

| Request | Answer |
| --- | --- |
| `GET /portal/login`, `POST /portal/login` (`method` = `password` with `user` and `password`, or `certificate`, as a form) | the sign-in page; a right credential is `303 /` with the cookie (`303 /portal/certificate` for a password sign-in that carries a certificate to enrol), a wrong one `401`, a method this connection does not offer `400`, too many failures `429` |
| `GET /portal/methods` | the ways in this connection may use, as JSON; no cookie |
| `GET /portal/certificate`, `POST /portal/certificate` (`action` = `enrol` or `remove`) | the certificate the browser offers and the one pinned to the account; pin or remove (signed in) |
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
- The same browser key holds `store.programs` and `accounts`: from the app the person installs, updates and removes agent
  programs in their own programs folder and signs in to providers, with the sign-in's typed answers sealed to the browser's
  key. Both run code as that person, which is what their sign-in already allows; neither reaches another person's host.
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
