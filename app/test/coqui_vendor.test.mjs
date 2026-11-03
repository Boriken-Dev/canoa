import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';
import {createInitiator, createResponder, generateIdentity} from '../web/vendor/coqui-peer-webcrypto.mjs';

test('Canoa vendors the exact pinned Coqui method source blobs', async () => {
  const meta = JSON.parse(readFileSync(new URL('../web/vendor/COQUI-SOURCE.json', import.meta.url)));
  assert.deepEqual(Object.keys(meta).sort(), ['coqui-peer-noise.mjs', 'coqui-peer-webcrypto.mjs']);
  for (const [name, pinned] of Object.entries(meta)) {
    const source = readFileSync(new URL(`../web/vendor/${name}`, import.meta.url));
    const prefix = Buffer.from('blob ' + source.length + '\0', 'utf8');
    const blobSha = createHash('sha1').update(prefix).update(source).digest('hex');
    assert.equal(blobSha, pinned.git_blob_sha1, name);
    const {profile} = await import(new URL(`../web/vendor/${name}`, import.meta.url));
    assert.equal(profile.id, pinned.profile, name);
  }
});

test('Canoa can use the pinned method for mutual auth without auto-pairing', async () => {
  const client = await generateIdentity();
  const server = await generateIdentity();
  const a = await createInitiator(client, server.peerId);
  const b = await createResponder(server, {allowClient: async id => id === client.peerId});
  const challenge = await b.challenge(a.hello());
  const clientProof = await a.answer(challenge);
  const accepted = await b.accept(clientProof);
  const finished = await a.finish(accepted.reply);
  assert.equal(finished.peerId, server.peerId);
  const record = await finished.session.seal('hello');
  assert.equal(new TextDecoder().decode(await accepted.session.open(record)), 'hello');
});
