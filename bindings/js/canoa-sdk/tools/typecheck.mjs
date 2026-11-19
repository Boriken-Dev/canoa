// Typechecks the SDK and the JavaScript examples (`tsc --noEmit`), and with `--types [DIR]` also writes the SDK's declaration files
// (`.d.ts`) into DIR (default: a folder that is removed afterwards, so the run only proves they can be generated).
//
// The SDK imports `guatiao`, `guatiao/kinds` and `guatiao/provide`, which come from Guatiao's Node binding. GUATIAO_NODE names its
// folder (bindings/node/guatiao, built so that dist/ holds the declarations), as for tools/write_kinds.mjs. Run from this folder with
// `npm install` done: TypeScript is a devDependency.
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const pkg = path.resolve(here, '..');
const examples = path.resolve(pkg, '../../../examples/providers/js');
const guatiao = process.env.GUATIAO_NODE;
const tsc = path.join(pkg, 'node_modules/typescript/bin/tsc');

if (!guatiao) {
  console.error("set GUATIAO_NODE to Guatiao's bindings/node/guatiao folder, built");
  process.exit(2);
}
const dist = path.resolve(guatiao, 'dist');
if (!existsSync(path.join(dist, 'index.d.ts'))) {
  console.error(`${dist} holds no index.d.ts: build Guatiao's Node binding first`);
  process.exit(2);
}
if (!existsSync(tsc)) {
  console.error('TypeScript is not installed: run `npm install` in bindings/js/canoa-sdk');
  process.exit(2);
}

const args = process.argv.slice(2);
const typesAt = args.indexOf('--types');
const wantTypes = typesAt >= 0;
const typesDir = wantTypes && args[typesAt + 1] && !args[typesAt + 1].startsWith('-') ? path.resolve(args[typesAt + 1]) : null;
const unknown = args.filter((arg, at) => arg !== '--types' && !(at === typesAt + 1 && typesDir !== null));
if (unknown.length > 0) {
  console.error(`unknown argument ${unknown[0]}; usage: node tools/typecheck.mjs [--types [DIR]]`);
  process.exit(2);
}

// A glob in a tsconfig is written with forward slashes on every platform.
const slashed = (file) => file.split(path.sep).join('/');
const work = mkdtempSync(path.join(tmpdir(), 'canoa-sdk-tsc-'));
// The package's own configuration, with Guatiao's declarations found where GUATIAO_NODE says.
const paths = { guatiao: [path.join(dist, 'index.d.ts')], 'guatiao/*': [path.join(dist, '*.d.ts')] };
const run = (name, config) => {
  const file = path.join(work, name);
  writeFileSync(file, JSON.stringify({ extends: path.join(pkg, config.extends), ...config.body }, null, 2));
  const result = spawnSync(process.execPath, [tsc, '-p', file], { cwd: pkg, stdio: 'inherit' });
  return result.status ?? 1;
};

let status = 0;
try {
  status = run('check.json', {
    extends: 'tsconfig.json',
    body: {
      compilerOptions: { paths },
      include: [slashed(path.join(pkg, 'src')), `${slashed(examples)}/**/*.js`],
    },
  });
  if (status === 0 && wantTypes) {
    const out = typesDir ?? path.join(work, 'types');
    status = run('types.json', {
      extends: 'tsconfig.types.json',
      body: {
        compilerOptions: { paths, outDir: out, rootDir: path.join(pkg, 'src') },
        include: [slashed(path.join(pkg, 'src'))],
      },
    });
    if (status === 0) {
      // The kinds' declarations are written by hand-off, not emitted from the JavaScript beside them.
      mkdirSync(out, { recursive: true });
      copyFileSync(path.join(pkg, 'src/kinds.d.ts'), path.join(out, 'kinds.d.ts'));
      for (const name of ['index.d.ts', 'speech.d.ts', 'kinds.d.ts']) {
        if (!existsSync(path.join(out, name))) {
          console.error(`${name} was not generated`);
          status = 1;
        }
      }
    }
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}
process.exit(status);
