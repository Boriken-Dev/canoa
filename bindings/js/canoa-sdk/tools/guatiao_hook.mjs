// Lets `node --import ./tools/guatiao_hook.mjs --test test/` import the SDK without Guatiao installed beside it: `guatiao` and
// `guatiao/<name>` resolve to the `dist` of the Node binding GUATIAO_NODE names, as for tools/typecheck.mjs.
import { existsSync } from 'node:fs';
import { register } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { isMainThread } from 'node:worker_threads';

if (isMainThread) {
  const dist = process.env.GUATIAO_NODE ? path.resolve(process.env.GUATIAO_NODE, 'dist') : '';
  if (!existsSync(path.join(dist, 'index.js'))) {
    console.error("set GUATIAO_NODE to Guatiao's bindings/node/guatiao folder, built");
    process.exit(2);
  }
  register(import.meta.url);
}

/**
 * @param {string} specifier
 * @param {unknown} context
 * @param {(specifier: string, context?: unknown) => unknown} next
 */
export function resolve(specifier, context, next) {
  const dist = path.resolve(process.env.GUATIAO_NODE ?? '', 'dist');
  if (specifier === 'guatiao') return next(pathToFileURL(path.join(dist, 'index.js')).href, context);
  if (specifier.startsWith('guatiao/')) {
    return next(pathToFileURL(path.join(dist, `${specifier.slice('guatiao/'.length)}.js`)).href, context);
  }
  return next(specifier, context);
}
