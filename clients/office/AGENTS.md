# `clients/office/`: contributor orientation

The Office add-in (Outlook, Word, Excel, PowerPoint): manifests, the task pane's pages and the relation logic. It does not
ship as a package; `canoa-host` includes the pages in `taskpane/` at compile time (`crates/canoa-host/src/web/office.rs`) and
serves them under `/office/` when `[host.web] office = true`. [`README.md`](README.md) says how to turn it on and sideload it.

- `taskpane/relation.js` is pure and imports nothing from Office.js: keep it that way, it is what `npm test` covers. The relation
  kinds (`conversation`, `document`) and the address parameters (`mode=chat`, `relation=<kind>:<key>`, `label=`) are the app's
  (`app/lib/client/`): change both sides together.
- The pane and the app talk by two short JSON strings through `postMessage` (`relationMessage`, `parsePaneMessage` in
  `relation.js`; `app/lib/client/pane_message.dart`): change both sides together. The portal's half is
  `crates/canoa-host/src/portal/office.rs`; its routes and cookies are in the host's `AGENTS.md`.
- A page added under `taskpane/` is served only once `office.rs` lists it, with the policy it needs; a new route there needs a
  test in `crates/canoa-host/src/web/tests.rs` saying what it relaxes and that every other route keeps its headers.
- Inline script is refused by the policy: scripts are files of this origin. Office.js is loaded from `<head>` at the address in
  `office.rs` (`JS_ORIGIN`); a page that loads it from another address fails under the policy.
- `render.mjs` keeps one add-in id per manifest kind (`IDS`): changing one makes Office treat the add-in as a new one.
- Manifest comments must not hold two hyphens in a row (XML); the render tests check it.

```sh
cd clients/office && npm test          # node --test, no dependencies
cargo test -p canoa-host --lib -- web:: office     # the routes' policies
cargo test -p canoa-host --features portal --lib -- portal::tests::office     # the portal's side (Unix only)
```
