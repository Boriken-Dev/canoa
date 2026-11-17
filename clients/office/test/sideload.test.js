// sideload.ps1 against a scratch registry key, never the one Office reads. Skipped where PowerShell or the Windows registry is not there.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { IDS, render } from '../render.mjs';

const script = join(dirname(fileURLToPath(import.meta.url)), '..', 'sideload.ps1');
const shell = ['pwsh', 'powershell'].find((name) => spawnSync(name, ['-NoProfile', '-Command', '$PSVersionTable.PSVersion.Major']).status === 0);
const skip = process.platform !== 'win32' || !shell ? 'needs PowerShell on Windows' : false;

/** A key nobody reads: the test refuses to touch any other. */
const scratch = `HKCU:\\Software\\CanoaTest\\office-sideload-${process.pid}`;
assert.ok(scratch.startsWith('HKCU:\\Software\\CanoaTest\\'));

/** Runs PowerShell on `args` and answers its status and output. */
function run(args) {
  const done = spawnSync(shell, ['-NoProfile', '-NonInteractive', ...args], { encoding: 'utf8' });
  return { status: done.status, out: `${done.stdout}${done.stderr}`.trim() };
}

/** Removes the scratch key, and the folder above it when that holds nothing else. */
function cleanup() {
  const parent = scratch.slice(0, scratch.lastIndexOf('\\'));
  run([
    '-Command',
    `Remove-Item -LiteralPath '${scratch}' -Recurse -ErrorAction SilentlyContinue; if ((Test-Path -LiteralPath '${parent}') -and -not (Get-ChildItem -LiteralPath '${parent}')) { Remove-Item -LiteralPath '${parent}' }`,
  ]);
}

const sideload = (action, manifest, key = scratch) => run(['-File', script, action, '-Manifest', manifest, '-Key', key]);

/** The values of the scratch key as `name=data` lines, or null when the key is not there. */
function values() {
  const read = run([
    '-Command',
    `if (Test-Path -LiteralPath '${scratch}') { $k = Get-Item -LiteralPath '${scratch}'; $k.GetValueNames() | ForEach-Object { $_ + '=' + $k.GetValue($_) } } else { 'NOKEY' }`,
  ]);
  return read.out === 'NOKEY' ? null : read.out.split(/\r?\n/).filter(Boolean);
}

test('add and remove are idempotent, write one value named by the add-in id, and touch nothing else', { skip }, () => {
  const dir = mkdtempSync(join(tmpdir(), 'canoa-sideload-'));
  try {
    const manifest = join(dir, 'documents.xml');
    writeFileSync(manifest, render('documents', { origin: 'https://localhost:7800', hosts: ['Document', 'Workbook'] }));
    const other = join(dir, 'other.xml');
    writeFileSync(other, render('outlook', { origin: 'https://localhost:7800' }));

    assert.equal(values(), null, 'the scratch key starts absent');
    assert.match(sideload('status', manifest).out, /Not registered/);

    // add, twice: the same one value.
    for (let again = 0; again < 2; again += 1) {
      const added = sideload('add', manifest);
      assert.equal(added.status, 0, added.out);
      assert.deepEqual(values(), [`${IDS.documents}=${manifest}`]);
    }
    assert.match(sideload('status', manifest).out, new RegExp(`Registered: ${IDS.documents}`));

    // a value that is not this add-in's survives the removal of this one.
    assert.equal(sideload('add', other).status, 0);
    assert.equal(values().length, 2);
    // remove, twice: only this add-in's value goes, and the second removal finds nothing and is not an error.
    for (let again = 0; again < 2; again += 1) {
      const removed = sideload('remove', manifest);
      assert.equal(removed.status, 0, removed.out);
      assert.deepEqual(values(), [`${IDS.outlook}=${other}`]);
    }
    assert.match(sideload('remove', manifest).out, /Nothing registered/);

    // -WhatIf writes nothing.
    assert.equal(run(['-File', script, 'add', '-Manifest', manifest, '-Key', scratch, '-WhatIf']).status, 0);
    assert.deepEqual(values(), [`${IDS.outlook}=${other}`]);
  } finally {
    cleanup();
    rmSync(dir, { recursive: true, force: true });
  }
  assert.equal(values(), null, 'the scratch key is gone afterwards');
});

test('a value an older tool named by the manifest path is removed with the add-in', { skip }, () => {
  const dir = mkdtempSync(join(tmpdir(), 'canoa-sideload-'));
  try {
    const manifest = join(dir, 'documents.xml');
    writeFileSync(manifest, render('documents', { origin: 'https://localhost:7800' }));
    run(['-Command', `New-Item -Path '${scratch}' -Force | Out-Null; Set-ItemProperty -LiteralPath '${scratch}' -Name '${manifest}' -Value '${manifest}'`]);
    assert.equal(sideload('remove', manifest).status, 0);
    assert.deepEqual(values(), []);
  } finally {
    cleanup();
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a manifest that is no file or has no id is refused before anything is written', { skip }, () => {
  const dir = mkdtempSync(join(tmpdir(), 'canoa-sideload-'));
  try {
    assert.notEqual(sideload('add', join(dir, 'missing.xml')).status, 0);
    const bad = join(dir, 'bad.xml');
    writeFileSync(bad, '<OfficeApp><Id>not-a-guid</Id></OfficeApp>');
    assert.notEqual(sideload('add', bad).status, 0);
    assert.equal(values(), null);
  } finally {
    cleanup();
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the script defaults to the key Office reads', () => {
  const source = readFileSync(script, 'utf8');
  assert.ok(source.includes(String.raw`[string]$Key = 'HKCU:\Software\Microsoft\Office\16.0\WEF\Developer'`));
});
