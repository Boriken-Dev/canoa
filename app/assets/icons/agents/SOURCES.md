# Where each agent mark came from

Each mark names the program a person runs; it is the vendor's trademark, shown for that purpose only, and nothing here
grants a right in it. None is redrawn: the path data is the vendor's or the icon set's, unchanged. `claude.svg`,
`codex.svg`, `antigravity.svg` and `grok.svg` have the fill attribute named below added; no path was edited.
`test/agent_marks_test.dart` checks that `lib/sources/agent_marks.dart` carries exactly these paths.

| File | Provider id | Origin | Terms | Fill |
| --- | --- | --- | --- | --- |
| `claude.svg` | `claude` | Simple Icons 16.37.0, `icons/claude.svg`: https://cdn.jsdelivr.net/npm/simple-icons@16.37.0/icons/claude.svg (the set's colour for the mark, `#D97757`) | The file is CC0 1.0 (https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md); the mark is Anthropic's trademark. | `#D97757` (the set's brand colour) |
| `codex.svg` | `codex` | Simple Icons 15.0.0, `icons/openai.svg`: https://cdn.jsdelivr.net/npm/simple-icons@15.0.0/icons/openai.svg. Later releases of the set dropped the OpenAI icon; the file at this release is unchanged. openai.com/brand answered 403 to every fetch from this machine, so the vendor's own page could not be read. | CC0 1.0 file; the mark is OpenAI's trademark. | `currentColor` (monochrome: the text colour) |
| `antigravity.svg` | `antigravity` | The path of the mark in the logo menu ("Copy logo") of https://antigravity.google, fetched 2026-11-13. The page's own copy lays gradient layers over this shape; only the shape and its flat base fill `#3186FF` are kept, since the gradients do not survive at 16-20 px. | The page offers the logo to copy; it states no further terms. The mark is Google's trademark. | `#3186FF` (the file's own) |
| `grok.svg` | `grok` | The two glyph paths of https://grok.com/images/favicon.svg (the site's own favicon, an SVG; the rounded tile, blur and outline layers around the glyph are left out), fetched 2026-11-13. | No reuse terms are stated. The mark is xAI's trademark, shown to name the program the person runs; using it is the project owner's decision. | `currentColor` (monochrome: the text colour) |
| `pi.svg` | `pi` | https://pi.dev/logo-auto.svg, the file the site's logo menu downloads as `pi-logo.svg`, fetched 2026-11-13, unchanged. | Offered for download by the site, which states no further terms. | three colours, the file's own |

## Not here

- An unknown provider id, an entry with no provider, and a command a person wrote keep the generic icons.
