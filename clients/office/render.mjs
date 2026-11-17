// Renders an add-in manifest for one origin: `node render.mjs outlook|documents --origin https://HOST [--hosts A,B] [--out FILE]`.
// The origin is the person's own Canoa host, so a manifest is made for it and not kept in the repository.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** The add-in ids: fixed per kind, so a person who renders again for another origin replaces the add-in, not adds a second. */
export const IDS = {
  outlook: '9aa21379-39d3-4bf5-a771-e341b13d51db',
  documents: '4c67403b-6e15-4886-9bfb-c624ca3fcf8a',
};

/** The add-in's version in the manifest: four numbers. */
export const VERSION = '0.1.0.0';

/** The Office applications a documents manifest may name, as Office's manifest calls them, with the application each is. */
export const DOCUMENT_HOSTS = { Document: 'Word', Workbook: 'Excel', Presentation: 'PowerPoint' };

/**
 * `origin` as the manifest writes it, or an error: `https://host[:port]` with nothing after it. Office on the web loads only
 * HTTPS pages, and a path, a query, a fragment or a login in the address would not be an origin.
 * @param {string} origin
 * @returns {string}
 */
export function checkOrigin(origin) {
  let url;
  try {
    url = new URL(origin);
  } catch {
    throw new Error(`not an address: ${origin}`);
  }
  if (url.protocol !== 'https:') throw new Error('the origin must be https: Office on the web loads no other page');
  if (url.username || url.password || url.search || url.hash || (url.pathname !== '/' && url.pathname !== '')) {
    throw new Error('the origin is a scheme, a host and an optional port, with nothing after it');
  }
  if (!/^[a-z0-9.-]+$/.test(url.hostname)) throw new Error('the origin names a host of letters, digits, hyphens and dots');
  return url.origin;
}

/** `text` safe inside an XML attribute or element. */
function escapeXml(text) {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&apos;');
}

/**
 * The manifest of `kind` for `origin`.
 * @param {'outlook'|'documents'} kind
 * @param {{origin: string, hosts?: string[], template?: string}} options  `hosts`: which of `Document`, `Workbook` and
 *   `Presentation` a documents manifest names, all three by default; `template` is for tests.
 * @returns {string}
 */
export function render(kind, options) {
  if (!Object.hasOwn(IDS, kind)) throw new Error(`unknown manifest: ${kind} (outlook or documents)`);
  const origin = checkOrigin(options.origin);
  const template = options.template ?? readFileSync(join(here, kind, 'manifest.xml.in'), 'utf8');
  let hostsXml = '';
  if (kind === 'documents') {
    const hosts = options.hosts && options.hosts.length > 0 ? options.hosts : Object.keys(DOCUMENT_HOSTS);
    for (const host of hosts) {
      if (!Object.hasOwn(DOCUMENT_HOSTS, host)) throw new Error(`unknown host: ${host} (${Object.keys(DOCUMENT_HOSTS).join(', ')})`);
    }
    hostsXml = [...new Set(hosts)].map((host) => `    <Host Name="${host}"/>`).join('\n');
  }
  return template
    .replaceAll('{{ORIGIN}}', escapeXml(origin))
    .replaceAll('{{ID}}', IDS[kind])
    .replaceAll('{{VERSION}}', VERSION)
    .replaceAll('{{HOSTS}}', hostsXml);
}

/** Reads the command line: the kind first, then `--origin`, `--hosts` and `--out`, each with a value. */
export function parseArguments(argv) {
  const [kind, ...rest] = argv;
  const options = {};
  for (let at = 0; at < rest.length; at += 2) {
    const name = rest[at];
    const value = rest[at + 1];
    if (!['--origin', '--hosts', '--out'].includes(name) || value === undefined) throw new Error(`unexpected argument: ${name}`);
    options[name.slice(2)] = value;
  }
  return { kind, origin: options.origin, hosts: options.hosts?.split(',').filter(Boolean), out: options.out };
}

function main() {
  const usage = 'usage: node render.mjs outlook|documents --origin https://HOST [--hosts Document,Workbook,Presentation] [--out FILE]';
  let parsed;
  try {
    parsed = parseArguments(process.argv.slice(2));
    if (!parsed.origin) throw new Error('--origin is required');
    const manifest = render(parsed.kind, { origin: parsed.origin, hosts: parsed.hosts });
    if (parsed.out) writeFileSync(parsed.out, manifest);
    else process.stdout.write(manifest);
  } catch (error) {
    process.stderr.write(`${error.message}\n${usage}\n`);
    process.exitCode = 2;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
