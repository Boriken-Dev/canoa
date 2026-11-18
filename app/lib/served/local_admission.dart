import 'dart:async';

import 'package:http/http.dart' as http;

import '../auth/browser_peer.dart';

/// The address parameter a manifest rendered for a host that keeps a secret carries it in, and the header it travels in.
const officeKeyParameter = 'office_key';
const officeKeyHeader = 'x-canoa-office-key';

/// The host could not accept this page's key.
final class AdmissionFailed implements Exception {
  const AdmissionFailed(this.message);
  final String message;
  @override
  String toString() => message;
}

/// A page the host serves to a client on its own computer (the Office add-in): its key is not paired by a PIN or by hand, the
/// page asks the host that served it to accept the key, and the host does so when the request comes from this computer, from the
/// page's own origin, for a key that has already connected once and been turned away.
final class LocalAdmission {
  LocalAdmission({
    required this.base,
    required this.path,
    http.Client? client,
    Future<String> Function(PeerMethod method)? localPeer,
  })  : _client = client ?? http.Client(),
        _ownClient = client == null,
        _localPeer = localPeer ?? localBrowserPeerId;

  /// The page's own address; the request goes to its origin.
  final Uri base;

  /// The path `canoa.json` names for it.
  final String path;

  final http.Client _client;
  final bool _ownClient;
  final Future<String> Function(PeerMethod method) _localPeer;
  Future<void>? _admitting;

  /// The home's secret when the page was opened with one (`?office_key=`, as the pane gives it): only the 64 hexadecimal digits the
  /// host makes are sent on.
  Map<String, String> get _secretHeader {
    final key = base.queryParameters[officeKeyParameter];
    return key != null && RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(key) ? {officeKeyHeader: key.toLowerCase()} : const {};
  }

  /// Asks the host to accept this key, which has connected and been turned away.
  Future<void> admit(PeerMethod method) => _admitting ??= _admit(method).whenComplete(() => _admitting = null);

  Future<void> _admit(PeerMethod method) async {
    final peer = await _localPeer(method);
    final http.Response response;
    try {
      response = await _client.post(
        base.resolve(path).replace(queryParameters: {'peer': peer}),
        headers: _secretHeader,
      );
    } on Object catch (error) {
      throw AdmissionFailed('the host did not answer: $error');
    }
    switch (response.statusCode) {
      case 200:
        return;
      case 404:
        throw const AdmissionFailed('the host never saw this key connect, so there is nothing to accept');
      default:
        throw AdmissionFailed('the host would not accept this key (${response.statusCode})');
    }
  }

  /// Runs [dial]; a host that has not met this key turns it away, so after a first failure the key is admitted and [dial] runs
  /// again.
  Future<T> connect<T>(PeerMethod method, Future<T> Function() dial) async {
    try {
      return await dial();
    } on Object {
      await admit(method);
      return dial();
    }
  }

  void close() {
    if (_ownClient) _client.close();
  }
}
