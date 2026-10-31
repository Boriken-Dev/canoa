import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';
import {createInitiator, createResponder, generateIdentity, profile} from '../web/vendor/coqui-peer-webcrypto.mjs';

test('Canoa vendors the exact pinned Coqui method source blob', () => {
  const source = readFileSync(new URL('../web/vendor/coqui-peer-webcrypto.mjs', import.meta.url));
  const meta = JSON.parse(readFileSync(new URL('../web/vendor/COQUI-SOURCE.json', import.meta.url)));
  const prefix = Buffer.from('blob ' + source.length + '\0', 'utf8');
  const blobSha = createHash('sha1').update(prefix).update(source).digest('hex');
  assert.equal(blobSha, meta.git_blob_sha1);
  assert.equal(profile.id, meta.profile);
});

test('Canoa can use the pinned method for mutual auth without auto-pairing', async () => {
  const client = await generateIdentity();
  const server = await generateIdentity();
  const a = await createInitiator(client, server.peerId);
  const b = await createResponder(server, {allowClient: async id => id === client.peerId});
  const challenge = await b.challenge(a.hello());
  const clientProof = await a.answer(challenge);
  const accepted = await b.accept(clientProof);
  assert.equal(await a.finish(accepted.reply), server.peerId);
});
