import 'dart:async';

import 'package:coqui_acp/secret.dart' show AcpIdentityId;
import 'package:coqui_web/coqui_web.dart';

import '../acp/client.dart' show LinkSecretIdentity, VerifyAcpPeer;
import '../acp/noise_secret.dart' show NoiseSecrets;
import '../acp/secret_identity.dart' show browserSecretIdentity;
import '../acp/connect.dart' show chatEndpoints;
import '../acp/served_host.dart';
import '../auth/browser_peer.dart';
import '../desktop/host_control.dart';
import '../desktop/host_tool.dart';

/// Why a page cannot reach its host's control channel.
final class ServedControlUnavailable implements Exception {
  const ServedControlUnavailable(this.message);
  final String message;
  @override
  String toString() => message;
}

/// A link over a proven channel: the control channel's messages, one a line, once the host has shown it holds the key this page
/// pinned. Only a key the host's `[host.control]` grants is served after the proof.
final class _ProvenLink implements HostLink, HostLinkBinding {
  _ProvenLink(this._channel, this._host, this._noise);

  final WebChannel _channel;
  final String _host;
  final NoiseSecrets? _noise;

  /// This page's key bound to the host it proved: by Noise the identity the proof made, else the WebCrypto key of this page.
  @override
  Future<LinkSecretIdentity?> secretBinding() async {
    try {
      final noise = _noise;
      final identity = noise != null ? noise.identity : await browserSecretIdentity();
      return identity == null ? null : (identity: identity, host: AcpIdentityId.parse(_host));
    } on Object {
      return null;
    }
  }

  @override
  Stream<String> get messages => _channel.messages;

  @override
  Future<void> send(String message) => _channel.send(message);

  @override
  Future<void> close() => _channel.close();
}

/// Dials [host]'s control channel and proves the host's key by the method it names before anything is sent. [verify] stands in
/// for the browser's proof where there is none (a test in the Dart VM). Throws [ServedControlUnavailable] when the host lists no
/// control address.
Future<HostLink> dialServedControl(
  ServedHost host, {
  VerifyAcpPeer? verify,
  Duration timeout = const Duration(seconds: 10),
}) async {
  final control = host.control;
  if (control == null) throw const ServedControlUnavailable('this host listens for no control channel');
  if (verify == null) {
    final why = await browserMethodUnavailable(host.method);
    if (why != null) throw MethodUnavailable(host.method, why);
  }
  final source = await connectWebChannel(chatEndpoints(webSocket: control.url, preferWebTransport: false), timeout: timeout);
  final staged = StagedChannel(source);
  try {
    final proven = await (verify ?? (channel, expected) => verifyBrowserPeer(channel, expected, host.method))(staged, host.peer)
        .timeout(timeout);
    if (proven.peerId != host.peer) {
      await proven.channel.close();
      throw const ServedControlUnavailable('the host proved another key than the one this page pinned');
    }
    staged.allowApplicationMessages();
    return _ProvenLink(proven.channel, proven.peerId, proven.noise);
  } catch (_) {
    await staged.close();
    rethrow;
  }
}

/// The control channel of the host that served this page, shared by whatever needs it: the session list, folders, uploads,
/// canoa.cfg and overlays.
///
/// [failed] runs after a dial that did not succeed, before the failure goes on: a portal's page uses it to find out whether the
/// session is over.
HostControl servedControl(
  ServedHost host, {
  VerifyAcpPeer? verify,
  Duration timeout = const Duration(seconds: 10),
  Future<void> Function()? failed,
}) =>
    HostControl.dialing(() async {
      try {
        return await dialServedControl(host, verify: verify, timeout: timeout);
      } on Object {
        await failed?.call();
        rethrow;
      }
    }, wait: timeout * 2);
