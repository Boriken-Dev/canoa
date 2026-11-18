# `clients/office/`: contributor orientation

The Office add-in (Outlook, Word, Excel, PowerPoint): manifests, the task pane's pages and the relation logic. It does not
ship as a package; `canoa-host` includes the pages in `taskpane/` at compile time (`crates/canoa-host/src/web/office.rs`) and
serves them under `/office/` when `[host.web] office = true`. [`README.md`](README.md) says how to turn it on and sideload it.

- `taskpane/relation.js` is pure and imports nothing from Office.js: keep it that way, it is what `npm test` covers. The relation
  kinds (`conversation`, `document`) and the address parameters (`mode=chat`, `relation=<kind>:<key>`, `label=`, `client=`) are the app's
  (`app/lib/client/`): change both sides together.
- The pane and the app talk by two short JSON strings through `postMessage` (`relationMessage`, `parsePaneMessage` in
  `relation.js`; `app/lib/client/pane_message.dart`): change both sides together. The portal's half is
  `crates/canoa-host/src/portal/office.rs`; its routes and cookies are in the host's `AGENTS.md`.
- A page added under `taskpane/` is served only once `office.rs` lists it, with the policy it needs; a new route there needs a
  test in `crates/canoa-host/src/web/tests.rs` saying what it relaxes and that every other route keeps its headers.
- Inline script is refused by the policy: scripts are files of this origin. Office.js is loaded from `<head>` at the address in
  `office.rs` (`JS_ORIGIN`); a page that loads it from another address fails under the policy.
- `launch.js` and `tools.js` are pure like `relation.js` (`fetch`, the clock, the open item and the question put to the person are arguments), and so are the tool modules, which take Office.js's `Office`, `Word` or `Excel` as an argument: tests give them stand-ins. A tool that changes anything is `write: true`, which `tools.js` asks in the pane before it runs; a new write tool without it would run unasked. The wire (`/office/tools/open|poll|send|close`) is the host's `web/local.rs`: change both sides together. A tool answer is capped well under the host's 4 MiB message limit.
- The pane's address carries the home's secret as `office_key` when the manifest does (`render.mjs --secret-file`); `launch.js` reads it and sends it as `x-canoa-office-key`, `appAddress` passes it to the app, and `app/lib/served/local_admission.dart` sends it with its request.
- `launch.html` is not a page of the pane: the launcher (`canoa office-launcher`) answers `taskpane.html` with it when no host runs.
- `render.mjs` keeps one add-in id per manifest kind (`IDS`): changing one makes Office treat the add-in as a new one.
- Manifest comments must not hold two hyphens in a row (XML); the render tests check it.
- `sideload.ps1` writes only the one value under `HKCU\Software\Microsoft\Office\16.0\WEF\Developer`; its tests pass `-Key` a
  scratch key under `HKCU:\Software\CanoaTest\` and remove it, and nothing in the repository runs the script against the real key.
- The page on the person's own computer is accepted by `[host.web] office_local` (`POST /office/admit`, see
  `crates/canoa-host/AGENTS.md`); `taskpane.js` watches a host whose `/canoa.json` names `admit` and says when it stops answering, and `app/lib/served/local_admission.dart`
  is the app's side of the request.

```sh
cd clients/office && npm test          # node --test, no dependencies
cargo test -p canoa-host --lib -- launcher web::local     # the launcher, the secret and the panes' sessions
cargo test -p canoa-host --test office_tools     # a real host: a pane's tools reach a session, the secret, a restart
cargo test -p canoa-host --lib -- web:: office     # the routes' policies
cargo test -p canoa-host --features portal --lib -- portal::tests::office     # the portal's side (Unix only)
cargo test -p canoa-host --test web                # a real host: HTTPS, and the page's key accepted with no pairing
CANOA_E2E_BROWSER=<Edge> CANOA_E2E_HOST=<canoa> CANOA_E2E_AGENT=<stdio_agent> CANOA_E2E_WEB=<flutter build web --csp output> node e2e/local.mjs
```
