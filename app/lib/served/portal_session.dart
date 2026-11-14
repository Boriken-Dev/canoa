import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import '../auth/browser_peer.dart';
import 'navigate.dart';

/// The browser is not signed in to the portal (any more); it has been sent to the sign-in page.
final class PortalSignedOut implements Exception {
  const PortalSignedOut();
  @override
  String toString() => 'signed out of the portal';
}

/// The portal could not introduce this browser's key to the person's host.
final class PortalPairingFailed implements Exception {
  const PortalPairingFailed(this.message);
  final String message;
  @override
  String toString() => message;
}

/// A page served by `canoa portal`: who is signed in, how this browser's key is introduced to that person's host, and signing out.
///
/// The session is a cookie the page cannot read (`HttpOnly`): `fetch` and the WebSocket carry it on their own, so nothing here
/// holds a token. A `401` from any request means the session is over, and the browser goes to the sign-in page.
final class PortalSession {
  PortalSession({
    required this.user,
    required this.base,
    http.Client? client,
    void Function(Uri address)? navigate,
    Future<String> Function(PeerMethod method)? localPeer,
  })  : _client = client ?? http.Client(),
        _ownClient = client == null,
        _navigate = navigate ?? navigateTo,
        _localPeer = localPeer ?? localBrowserPeerId;

  /// The account that is signed in.
  final String user;

  /// The portal's own address; the requests go to it.
  final Uri base;

  final http.Client _client;
  final bool _ownClient;
  final void Function(Uri address) _navigate;
  final Future<String> Function(PeerMethod method) _localPeer;
  Future<void>? _pairing;

  Uri get login => base.resolve('/portal/login');

  /// Sends the browser to the sign-in page.
  void signedOut() => _navigate(login);

  /// Whether the session still holds. A `401` sends the browser to the sign-in page; an answer that is neither, or none, is taken
  /// as "yes", since the portal being slow is no reason to leave the page.
  Future<bool> active() async {
    try {
      final response = await _client.get(base.resolve('/portal/session'));
      if (response.statusCode == 401) {
        signedOut();
        return false;
      }
    } on Object {
      // Not an answer.
    }
    return true;
  }

  /// Asks the portal to accept this browser's key, which has connected to the person's host and been turned away.
  Future<void> pair(PeerMethod method) => _pairing ??= _pair(method).whenComplete(() => _pairing = null);

  Future<void> _pair(PeerMethod method) async {
    final peer = await _localPeer(method);
    final http.Response response;
    try {
      response = await _client.post(
        base.resolve('/portal/pair'),
        headers: {'content-type': 'application/json'},
        body: jsonEncode({'peer': peer}),
      );
    } on Object catch (error) {
      throw PortalPairingFailed('the portal did not answer a pairing: $error');
    }
    switch (response.statusCode) {
      case 200:
        return;
      case 401:
        signedOut();
        throw const PortalSignedOut();
      case 404:
        throw const PortalPairingFailed('your host never saw this browser\'s key connect, so there is nothing to accept');
      default:
        throw PortalPairingFailed('the portal refused to pair this browser (${response.statusCode})');
    }
  }

  /// Runs [dial]; a host that has not met this browser's key turns it away, so after a first failure the key is introduced
  /// ([pair]) and [dial] runs again. Throws [PortalSignedOut] when the session is over.
  Future<T> connect<T>(PeerMethod method, Future<T> Function() dial) async {
    try {
      return await dial();
    } on PortalSignedOut {
      rethrow;
    } on Object {
      if (!await active()) throw const PortalSignedOut();
      await pair(method);
      return dial();
    }
  }

  /// Ends the session on the portal and returns the browser to the sign-in page.
  Future<void> signOut() async {
    await _client.post(base.resolve('/portal/logout'));
    signedOut();
  }

  void close() {
    if (_ownClient) _client.close();
  }
}
