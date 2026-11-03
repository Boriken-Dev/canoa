// The page's peer bridge, web/canoa_peer_bridge.mjs, run by Node over its WebCrypto, for tests on the Dart VM that need the
// browser's proof. One line of JSON per call on standard input, one answer per line on standard output; bytes are base64.
// The key is made for this process and kept in memory, as a browser profile's would be.
import {createInterface} from 'node:readline';
import {createPeer} from '../../web/canoa_peer_bridge.mjs';
import {generateIdentity} from '../../web/vendor/coqui-peer-webcrypto.mjs';

const identity = generateIdentity();
const peer = createPeer(() => identity);
const bytes = text => new Uint8Array(Buffer.from(text, 'base64'));
const text = data => (data == null ? null : Buffer.from(data).toString('base64'));

const calls = {
  localId: () => peer.localId(),
  begin: expected => peer.begin(expected),
  answer: (token, challenge) => peer.answer(token, challenge),
  finish: (token, accepted) => peer.finish(token, accepted),
  abort: token => peer.abort(token),
  seal: async (session, message) => text(await peer.seal(session, bytes(message))),
  open: async (session, record) => text(await peer.open(session, bytes(record))),
  closeRecord: async session => text(await peer.closeRecord(session)),
  closeSession: session => peer.closeSession(session),
};

for await (const line of createInterface({input: process.stdin})) {
  const {id, op, args} = JSON.parse(line);
  try {
    const result = await calls[op](...args);
    process.stdout.write(JSON.stringify({id, result: result ?? null}) + '\n');
  } catch (error) {
    process.stdout.write(JSON.stringify({id, error: String(error?.message ?? error)}) + '\n');
  }
}
