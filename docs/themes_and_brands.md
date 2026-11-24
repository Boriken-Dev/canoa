---
brand_names: false
---

# Words, symbols and brands

Everything the app writes and the symbols it draws for a section come from one text provider. A look can replace any subset of
them, a language can translate them, and a build can carry another name.

## The words of the app

The app holds a base of words, in English and Spanish, each under a key such as `nav.conversations` or `composer.more`. A word is
looked up in this order, and the first that has it is shown:

1. the look's own text, in the language the person reads;
2. the look's own default language;
3. the base text, in the language the person reads;
4. the base text in English;
5. the key itself, when nobody has it.

The language is `[appearance] language` in canoa.cfg (Settings, Appearance, Language), or the system's when none is chosen or when
nothing has words in the one chosen. A language is offered for choosing when the base or the look in force has words in it, each
named by its own `language.name` word.

`{brand}` and `{short}` in a word are the build's name and short name; any other `{name}` is filled in by the screen that shows it.

## A look that brings its own words

A look listed by the host (`looks/list`) may carry a `text` table beside its colours:

```json
{
  "id": "stage",
  "label": "Stage",
  "vendor": "stage",
  "dark": { "accent": "#33AA55" },
  "light": { "accent": "#116633" },
  "text": {
    "default_language": "en",
    "strings": {
      "en": { "nav.conversations": "Scenes", "nav.settings": "Backstage" },
      "fr": { "language.name": "Français", "nav.conversations": "Scènes" }
    },
    "symbols": {
      "conversations": { "glyph": ">" },
      "store": { "like": "terminal" }
    }
  }
}
```

It replaces as many words as it names and takes the rest from the base, so a look that sets none draws the base unchanged, and one
that adds a language is offered that language. A symbol is drawn as a glyph of text (`"glyph"`, or the bare string) or as another
of the base's symbols (`"like"`): the base symbols are `conversations`, `hosts`, `terminal`, `usage`, `secrets`, `loadouts`,
`store`, `settings`, `sign_out`, `layout`, `search` and `more`. The tray's menu and tooltip follow the look the same way.

## A build with another name

`app/brand.json` holds what a build is called and how it names things; a build reads another file with `build.py --brand FILE` (or
`BRAND_FILE`):

| Key | What it is |
| --- | --- |
| `name`, `short_name` | what a person reads: the window, the tray, the words of the app |
| `executable` | the desktop program's name (`CanoaUI`), and the bundle's on macOS is `name` |
| `id` | the reverse-DNS identifier; its last label is dropped to make the organisation `flutter create` is given |
| `env_prefix` | the prefix of the environment variables the app and the host read: `CONFIG` is `<PREFIX>_CONFIG` |
| `icon` | an SVG with the paths `tools/canoa_mark.py` and the app's painter draw, relative to the brand file |
| `accent`, `accent_on_dark` | the colour the build's icons are drawn in, and the one on the web app's dark background (the accent when absent) |

The build gives the app these as `--dart-define`s, the host's build the prefix as `BRAND_ENV_PREFIX`, the runner its names
(`name_runner.sh`) and the Windows program its icon (`canoa_mark.py --brand FILE --out DIR` renders the set; it needs Pillow). The
tray icon is drawn at run time from the same mark in the accent of the look in force: an `.ico` of several sizes on Windows, a PNG
elsewhere. `--portable` and the Office add-in's identifiers still name Canoa.

## The documentation follows the brand

`mkdocs build` reads the same brand file (`BRAND_FILE`, else `app/brand.json`) through the hook `tools/docs_brand.py`:

- the site's name is `name`, and the header's logo is the brand's `icon` drawn in `accent` (in `accent_on_dark` on the dark scheme);
- both colour schemes are painted with the default look's colours, the page, text, bar and accent of Canoa's look in light and dark, whatever the brand: a rebranded build changes the mark's colour and keeps the look, as the app does. The colours are `docs/assets/look.json`, which `app/test/docs_look_test.dart` keeps current (`cd app && CANOA_WRITE_LOOK=1 flutter test test/docs_look_test.dart` rewrites it);
- a brand other than the default also gets a favicon of its mark (the default keeps the committed `docs/assets/favicon.ico`) and its names in the text of the pages. Only a closed list of tokens is replaced: `Canoa` (`name`), `CanoaUI` (`executable`), `canoa.cfg` (`config_file`, `<program>.cfg` when absent), `CANOA_` (`env_prefix`), `.canoa` (`home_dir`, `.<program>` when absent), the folder in `/etc/Canoa`, `%APPDATA%\Canoa` and the like (`folder`, `name` when absent), and `canoa@`, `canoa-portal`, `/opt/canoa` and `/run/canoa` (`program`, the lower-cased `name` when absent). Crate and repository names (`canoa-host`, `Boriken-Dev/canoa`), wire identifiers (`dev.boriken.canoa/...`) and URLs are never replaced. A page about the project and not the product (the roadmap, the offline dependencies, the browser comparisons) and this page opt out with `brand_names: false` in its front matter.

With the default brand the pages are not touched, so the site is the one the repository has always built.

```sh
BRAND_FILE=path/to/brand.json mkdocs build --strict
```

