# Views from agents and overlays (A2UI)

An agent, or an overlay, can show its data as a view beside the conversation: a card, a table, a form, a report. The view is
described as data in the [A2UI 1.0](https://a2ui.org) protocol and drawn by Canoa with its own widgets in the colours of your
look. Nothing a view carries runs: it chooses structure, never colours, code or addresses to fetch.

## From an agent

An agent sends A2UI messages in the `_meta.a2ui` of any `session/update`. An update kind the chat does not draw (`a2ui`) shows
nothing else in the conversation. `_meta.a2ui` is a message, a list of messages, or

```json
{"messages": [ ... ], "target": "side_panel", "title": "Test report"}
```

`target` is where a view the messages create opens: `side_panel` (the default, the right panel), `tab` (a tab of the bottom
panel) or `window` (a window of its own, over the app). `title` names its tab or window.

Every message has `"version": "1.0"` and exactly one of `createSurface`, `updateComponents`, `updateDataModel` and
`deleteSurface`. Anything else is refused and reported back, never guessed at: another version, several keys, an unknown key, a
`theme` in `createSurface` (the app draws the view), and `callRendererFunction` or `callAgentFunction` (no function is run for an
agent). A surface names its catalog with `catalogId`; the ones this app takes are

| Catalog | `catalogId` |
| --- | --- |
| A2UI's basic catalog | `https://a2ui.org/specification/v1_0/catalogs/basic/catalog.json` |
| Canoa's (the basic one and `Badge`, `Metric`, `Table`, `CodeBlock`, `Progress`) | `urn:dev.boriken.canoa:a2ui:catalog:1` |

A surface that names none uses Canoa's. Drawn from the basic catalog: `Text`, `Row`, `Column`, `List`, `Card`, `Tabs`, `Modal`,
`Divider`, `Icon`, `Image`, `Button`, `TextField` and `CheckBox`. Not drawn yet: `Video`, `AudioPlayer`, `ChoicePicker`, `Slider`,
`DateTimeInput`; a component the catalog does not have shows a notice in its place and the rest is drawn. Values are literals,
`{"@path": "/pointer"}` reads of the surface's data model (a path without a leading `/` is relative to the row of a list
template), or `{"@call": "formatString" | "formatNumber" | "pluralize" | "required" | "and" | "or" | "not", "args": {...}}`.
`TextField` and `CheckBox` write what you type or tick back to the data model. An `Image` is drawn only from a `data:image/` address
in the message; no other address is fetched.

### What a button does

A `Button` whose action is `{"event": {"name": ..., "context": {...}}}` sends the protocol's `action` message back to the agent
(`name`, `surfaceId`, `sourceComponentId`, `timestamp`, `context` with its bindings read now) as a prompt: the line
`A2UI action: NAME` and the message in a fenced `json` block, so it is in the conversation and an agent that takes no files gets
it as well. A turn that runs queues it. The surface's data model is not sent unless the agent asked (`sendDataModel`) and the app
allows it, which it does not. A `functionCall` action runs only `openUrl`, and only for an `http` or `https` address.

What is wrong with a view (an unknown component, a missing property, a parent or child a catalog does not allow, a message past a
limit) is shown above the view and sent back as the protocol's `error` (`VALIDATION_FAILED`, `UNALLOWED_PARENT`,
`UNALLOWED_CHILD`), at most three times for a surface, so a view that cannot be drawn never starts a loop of turns.

### Limits

They are bounds against an agent that sends far more than anyone could read, never against you: 400 components in a surface, 12
levels deep, 200 rows of a list or table, 256 KiB of data model, 512 KiB in a message, 8 surfaces in a session and 40 messages in a
second. Each is a field of the app's `A2uiLimits` with no floor (canoa.cfg has no key for them yet). Past one, the view is cut and says so (a notice above the
view, "N more rows are not shown", a branch that shows "Deeper than N levels"), and an update that would pass the data limit is
refused whole.

## From an overlay

An overlay may ship views as files, which the Store's page for the overlay lists under Views, each with an Open button that draws
it in the conversation that is open (and the reason, when it cannot be drawn):

- `surfaces/NAME.json`: `{"title": "...", "target": "side_panel", "messages": [ ...A2UI messages... ]}`, a fixed view with its data
  model. Its surface ids are made the overlay's own (`overlay:OVERLAY:ID`), so two overlays never meet.
- `catalogs/NAME.json`: `{"catalogId": "urn:...", "components": {"Stat": {"extends": "Metric", "defaults": {"label": "Open"},
  "allowedParents": ["Column"], "allowedChildren": []}}}`. A catalog is data: each component is a name and presets for a component
  Canoa already draws, so an overlay chooses structure and the look chooses colours. The `catalogId` is the overlay's own; one that
  repeats a component's name, or `extends` one Canoa does not draw, refuses the whole catalog.

The host reads the files with the rest of the overlay (`store/read_ahead` answers `views`: the top-level JSON files of the two
folders, at most 16 files of 128 KiB), and the app checks each view against the catalogs the overlay brought before offering it.
A WebAssembly module or a web page as a widget is not supported: it needs a sandbox Canoa does not have.
