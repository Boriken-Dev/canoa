// The WebCrypto method's other side, for browser tests of the dialing side: it admits a client only when `allowClient` says
// so, after the client proved its key.
import 'dart:typed_data';

import 'package:canoa/auth/web/peer_session.dart';
import 'package:canoa/auth/web/webcrypto_method.dart';
import 'package:canoa/auth/web/webcrypto_wire.dart';
import 'package:web/web.dart' as web;

final class WebCryptoResponder {
  WebCryptoResponder._(this._identity, this._allowClient, this._binding);

  final WebCryptoIdentity _identity;
  final Future<bool> Function(String peerId) _allowClient;
  final Uint8List _binding;
  var _state = 'hello';
  Offer? _hello;
  Offer? _challenge;
  web.CryptoKey? _clientKey;
  ({web.CryptoKey privateKey, String encoded})? _ephemeral;

  static WebCryptoResponder create(
    WebCryptoIdentity identity, {
    required Future<bool> Function(String peerId) allowClient,
    Uint8List? binding,
  }) => WebCryptoResponder._(identity, allowClient, binding ?? Uint8List(0));

  Future<String> challenge(Uint8List raw) async {
    if (_state != 'hello') throw const PeerFailure('Unexpected hello');
    _state = 'failed';
    final message = parseMessage(raw, 'hello', const ['publicKey', 'ephemeral', 'nonce'], defaultMaxMessage);
    if (message['publicKey'] == _identity.publicKey) throw const PeerFailure('Invalid peer message');
    final key = await verifyKey(message['publicKey']!);
    await ephemeralKey(message['ephemeral']!);
    un64(message['nonce'], nonceLength);
    final own = await newEphemeral();
    final hello = Offer(publicKey: message['publicKey']!, nonce: message['nonce']!, ephemeral: message['ephemeral']!);
    final answered = Offer(publicKey: _identity.publicKey, nonce: newNonce(), ephemeral: own.encoded);
    final signature = await signFields(_identity.privateKey, 'server', hello, answered, _binding);
    _hello = hello;
    _challenge = answered;
    _clientKey = key;
    _ephemeral = own;
    _state = 'proof';
    return pack('challenge', {
      'publicKey': answered.publicKey,
      'ephemeral': own.encoded,
      'nonce': answered.nonce,
      'signature': signature,
    });
  }

  Future<({String reply, String peerId, PeerSession session})> accept(Uint8List raw) async {
    if (_state != 'proof') throw const PeerFailure('Unexpected client proof');
    _state = 'failed';
    final message = parseMessage(raw, 'proof', const ['signature'], defaultMaxMessage);
    final hello = _hello!;
    final challenge = _challenge!;
    if (!await verifyFields(_clientKey!, message['signature']!, 'client', hello, challenge, _binding)) {
      throw const PeerFailure('Client key proof failed');
    }
    final id = await peerIdOf(un64(hello.publicKey, spkiLength));
    if (!await _allowClient(id)) throw const PeerFailure('Client is not paired');
    final session = await createSession(
      ownEphemeral: _ephemeral!.privateKey,
      peerEphemeral: hello.ephemeral,
      hello: hello,
      challenge: challenge,
      binding: _binding,
      initiator: false,
    );
    final reply = pack('accept', {'signature': await signFields(_identity.privateKey, 'accept', hello, challenge, _binding)});
    _state = 'done';
    return (reply: reply, peerId: id, session: session);
  }
}
