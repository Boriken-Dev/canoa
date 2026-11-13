// The web build's peer bridge: each method's own key, a proof or a PIN pairing run a step at a time, and the session a proof
// ends in. Application UI uses it only for that. It exposes no signing, no browser API, no credential and no change to the
// trusted identities to an agent, and no unpaired peer is silently accepted.
import 'dart:convert';
import 'dart:typed_data';

import '../peer_proof.dart';
import 'noise_method.dart';
import 'peer_session.dart';
import 'subtle.dart';
import 'webcrypto_method.dart';

/// Each method's key in a database of its own, so one key is never used by both. The extension's UI page shares them with the
/// extension, which keeps the same two records.
const webcryptoStore = 'dev.boriken.canoa.peer';
const noiseStore = 'dev.boriken.canoa.peer-noise';

const _proofTtl = Duration(seconds: 30);
// A pairing waits for the person at the other end only as long as its PIN does.
const _pairTtl = Duration(seconds: 120);
const _maxActive = 16;

Uint8List _needs(Uint8List? received) => received ?? (throw const PeerFailure('Unexpected step without a message'));

typedef _RunStep = ({List<Uint8List> send, String? peerId, PeerSession? session, String? failure});

/// One proof or pairing, advanced a message at a time.
abstract interface class _Run {
  Future<_RunStep> step(Uint8List? received);
  void abort();
}

/// A WebCrypto proof: hello, the answer to the challenge, then the acceptance. Its messages are text.
final class _WebCryptoRun implements _Run {
  _WebCryptoRun(this._started);

  final WebCryptoInitiator _started;
  int _stage = 0;

  @override
  Future<_RunStep> step(Uint8List? received) async {
    switch (_stage++) {
      case 0:
        return (send: [Uint8List.fromList(utf8.encode(_started.hello()))], peerId: null, session: null, failure: null);
      case 1:
        final answer = await _started.answer(_needs(received));
        return (send: [Uint8List.fromList(utf8.encode(answer))], peerId: null, session: null, failure: null);
      case 2:
        final done = await _started.finish(_needs(received));
        return (send: const <Uint8List>[], peerId: done.peerId, session: done.session, failure: null);
      default:
        throw const PeerFailure('Unexpected message after the proof');
    }
  }

  @override
  void abort() {}
}

/// A Noise proof or pairing: the module holds the state; the initiator holds the session once it sends its third message,
/// before the responder decides.
final class _NoiseRunStep implements _Run {
  _NoiseRunStep(this._run);

  final NoiseRun _run;

  @override
  Future<_RunStep> step(Uint8List? received) async {
    final step = await _run.step(received);
    return (send: step.send, peerId: step.peerId, session: step.session, failure: step.failure);
  }

  @override
  void abort() => _run.abort();
}

final class _Active {
  _Active(this.run, this.ttl) : created = DateTime.now();
  final _Run run;
  final Duration ttl;
  final DateTime created;
}

/// The calls the app makes, over the identities [loadWebCrypto] and [loadNoise] give: the page's own, kept in this origin's
/// IndexedDB.
final class WebPeerBridge implements PeerBridge {
  WebPeerBridge({
    Future<WebCryptoIdentity> Function()? loadWebCrypto,
    Future<NoiseIdentity> Function()? loadNoise,
    Uri? base,
  }) : _base = base,
       _loadWebCrypto = loadWebCrypto ?? (() => WebCryptoIdentity.loadOrCreate(webcryptoStore)),
       _loadNoise = loadNoise ?? (() => NoiseIdentity.loadOrCreate(noiseStore, base: base));

  final Uri? _base;
  final Future<WebCryptoIdentity> Function() _loadWebCrypto;
  final Future<NoiseIdentity> Function() _loadNoise;
  final _active = <String, _Active>{};
  final _sessions = <String, PeerSession>{};
  Future<WebCryptoIdentity>? _webcrypto;
  Future<NoiseIdentity>? _noise;

  // A key that failed to load is asked for again on the next call.
  Future<WebCryptoIdentity> get _webcryptoIdentity => _webcrypto ??= _loadWebCrypto().onError<Object>((error, trace) {
    _webcrypto = null;
    Error.throwWithStackTrace(error, trace);
  });
  Future<NoiseIdentity> get _noiseIdentity => _noise ??= _loadNoise().onError<Object>((error, trace) {
    _noise = null;
    Error.throwWithStackTrace(error, trace);
  });

  /// Asked only of the local runtime, before any connection, so nothing on the network can change the answer, and a false
  /// answer never selects another method.
  @override
  Future<String?> unreachable(PeerMethod method) async => switch (method) {
    PeerMethod.webcrypto => subtleMissing(),
    PeerMethod.noise => noiseMissing(base: _base),
  };

  Future<void> _reachable(PeerMethod method) async {
    final why = await unreachable(method);
    if (why != null) throw PeerFailure('unreachable from this browser: $why');
  }

  /// This page's WebCrypto key as a signer for secret bindings: its SubjectPublicKeyInfo and a function that signs only a secret
  /// binding's statement. The key is not handed over.
  Future<({Uint8List spki, Future<Uint8List> Function(Uint8List message) sign})> webcryptoSigner() async {
    await _reachable(PeerMethod.webcrypto);
    final identity = await _webcryptoIdentity;
    return (spki: identity.spki, sign: identity.signSecretStatement);
  }

  @override
  Future<String> localId(PeerMethod method) async {
    await _reachable(method);
    return switch (method) {
      PeerMethod.webcrypto => (await _webcryptoIdentity).peerId,
      PeerMethod.noise => (await _noiseIdentity).peerId,
    };
  }

  String _start(_Run run, Duration ttl) {
    if (_active.length >= _maxActive) throw const PeerFailure('Too many concurrent peer handshakes');
    final id = randomToken(24);
    _active[id] = _Active(run, ttl);
    return id;
  }

  _Run _running(String id) {
    final current = _active[id];
    if (current == null || DateTime.now().difference(current.created) > current.ttl) {
      current?.run.abort();
      _active.remove(id);
      throw const PeerFailure('Peer proof expired or unknown');
    }
    return current.run;
  }

  PeerSession _established(String id) => _sessions[id] ?? (throw const PeerFailure('Session closed or unknown'));

  @override
  Future<String> begin(PeerMethod method, String expectedPeerId) async {
    await _reachable(method);
    final _Run run = switch (method) {
      PeerMethod.webcrypto => _WebCryptoRun(await WebCryptoInitiator.create(await _webcryptoIdentity, expectedPeerId)),
      PeerMethod.noise => _NoiseRunStep(await (await _noiseIdentity).prove(expectedPeerId, base: _base)),
    };
    return _start(run, _proofTtl);
  }

  @override
  Future<String> pair(String pin) async {
    await _reachable(PeerMethod.noise);
    return _start(_NoiseRunStep(await (await _noiseIdentity).pair(pin, base: _base)), _pairTtl);
  }

  /// `send`: messages to send in order; once the peer is proved its id, and for a proof the session's token.
  @override
  Future<PeerStep> step(String token, Uint8List? received) async {
    try {
      final done = await _running(token).step(received == null ? null : Uint8List.fromList(received));
      final session = done.session;
      if (done.failure != null) {
        _active.remove(token)?.run.abort();
        return PeerStep(send: done.send, failure: done.failure);
      }
      if (session == null) return PeerStep(send: done.send, peerId: done.peerId);
      final sealing = randomToken(24);
      _sessions[sealing] = session;
      _active.remove(token)?.run.abort();
      return PeerStep(send: done.send, peerId: done.peerId, session: sealing);
    } catch (_) {
      _active.remove(token)?.run.abort();
      rethrow;
    }
  }

  @override
  void abort(String token) => _active.remove(token)?.run.abort();

  // Calls take their place in the session in call order, so the caller makes them in order.
  @override
  Future<Uint8List> seal(String session, Uint8List message) async => _established(session).seal(message);

  /// Null for the peer's close.
  @override
  Future<Uint8List?> open(String session, Uint8List record) async => _established(session).open(record);

  @override
  Future<Uint8List> closeRecord(String session) async => _established(session).close();

  @override
  void closeSession(String session) => _sessions.remove(session)?.destroy();
}
