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
| `program` | the command line's name, lower-case letters, digits and hyphens (the lower-cased `name` when absent); the units and accounts derived from it on Linux |
| `config_file` | the configuration file's name in every layer (`<program>.cfg` when absent) |
| `folder` | the one folder under each operating system's configuration, system and data roots: `%APPDATA%`, `$XDG_CONFIG_HOME`, `%PROGRAMDATA%`, `/etc`, `%LOCALAPPDATA%`, `~/.local/share` (`name` when absent) |
| `home_dir` | the home under the person's own folder, and the per-project folder beside a workspace, dot included (`.<program>` when absent) |
| `icon` | an SVG with the paths `tools/canoa_mark.py` and the app's painter draw, relative to the brand file |
| `accent`, `accent_on_dark` | the colour the build's icons are drawn in, and the one on the web app's dark background (the accent when absent) |

The build gives the app these as `--dart-define`s, the host's build the file as `BRAND_FILE`, the runner its names
(`name_runner.sh`) and the Windows program its icon (`canoa_mark.py --brand FILE --out DIR` renders the set; it needs Pillow). The
tray icon is drawn at run time from the same mark in the accent of the look in force: an `.ico` of several sizes on Windows, a PNG
elsewhere. `--portable` and the Office add-in's identifiers still name Canoa; the launcher's and the add-in's other names and messages, the tool servers an agent sees, the speech providers' variables, the flag a built application takes to run as the host's own command and the app's own words follow the brand.

## The documentation follows the brand

`mkdocs build` reads the same brand file (`BRAND_FILE`, else `app/brand.json`) through the hook `tools/docs_brand.py`:

- the site's name is `name`, and the header's logo is the brand's `icon` drawn in `accent` (in `accent_on_dark` on the dark scheme);
- both colour schemes are painted with the default look's colours, the page, text, bar and accent of Canoa's look in light and dark,
  whatever the brand: a rebranded build changes the mark's colour and keeps the look, as the app does. The colours are
  `docs/assets/look.json`, which `app/test/docs_look_test.dart` keeps current
  (`cd app && CANOA_WRITE_LOOK=1 flutter test test/docs_look_test.dart` rewrites it);
- a brand other than the default also gets a favicon of its mark; the default keeps the committed `docs/assets/favicon.ico`;
- the pages name the product with placeholders, below.

```sh
BRAND_FILE=path/to/brand.json mkdocs build --strict
```

### Writing a page with placeholders

A page writes the product's names as placeholders, and the build renders them from the brand, the default brand included:

| Placeholder | Is | For |
| --- | --- | --- |
| `{{{ name }}}`, `{{{ short_name }}}` | the brand's `name` and `short_name` | the product in prose and headings: "the {{{ name }}} host" |
| `{{{ program }}}` | `program`, the command line's name (the brand's lower-cased name when the file gives none) | commands, and the names derived from it on Linux: `{{{ program }}} serve`, `{{{ program }}}@NAME.service`, `/opt/{{{ program }}}` |
| `{{{ executable }}}` | `executable` | the desktop program: `{{{ executable }}}.exe` |
| `{{{ config_file }}}` | `config_file` (`<program>.cfg` when absent) | the configuration file's name |
| `{{{ folder }}}` | `folder` (`name` when absent) | the folder under each operating system's configuration and data roots: `/etc/{{{ folder }}}` |
| `{{{ home_dir }}}` | `home_dir` (`.<program>` when absent), dot included | the home: `~/{{{ home_dir }}}` |
| `{{{ env_prefix }}}` | `env_prefix` | the product's environment variables: `{{{ env_prefix }}}_CONFIG` |
| `{{{ id }}}` | `id` | the reverse-DNS identifier |

A placeholder works in prose, headings, code blocks and links. In a link's fragment it renders as the heading's id would, so
`[text](browser.md#{{{ name }}}-on-this-computer)` follows the heading "{{{ name }}} on this computer". A `{{ other }}` that is not
in the table is left as written, and three braces keep a placeholder as written, as two (how this page
shows them).

Keep a name literal when it is not the product's but a fact of this repository or of a protocol: crate names (`canoa-host`,
`canoa-whispercpp`), the repository and its URLs (`Boriken-Dev/canoa`), wire identifiers (`dev.boriken.canoa/...`, `/canoa.json`,
`_canoa/...`), Cargo targets, build and test variables (`CANOA_DEPS`, `CANOA_WEB_DIST`), and a line that deliberately means Canoa
itself. `tools/test_docs_brand.py` fails on a `docs/*.md` line that names the product literally (`Canoa` as a word,
`canoa.cfg`, `CANOA_CONFIG`, `~/.canoa`, `canoa@`, `/etc/canoa`, `/opt/canoa`); a line that has to stay literal goes in its
allowlist, with the page and the reason. This page is on that list: it describes Canoa's own brand file.
