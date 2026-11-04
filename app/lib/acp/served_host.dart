import 'dart:convert';

import 'package:http/http.dart' as http;

import '../auth/peer_proof.dart';

/// Whether this is a web page; Flutter's own `kIsWeb` would keep the peer test page, which `dart compile js` builds, from
/// compiling.
const _inPage = bool.fromEnvironment('dart.library.js_interop');

/// One entry the host that served this page lists.
final class ServedEntry {
  const ServedEntry({required this.name, required this.kind, required this.url});

  final String name;

  /// `agent`, `provider` or `tool`.
  final String kind;

  /// Its WebSocket address.
  final String url;
}

/// What the `canoa-host` that served this page says it serves, from `canoa-host.json` beside the page. It only fills the
/// connect dialog: connecting still proves the host holds [peer]'s key, and the host still turns this browser away until
/// its key is accepted there.
final class ServedHost {
  const ServedHost({required this.peer, required this.method, required this.entries});

  /// The host's peer id.
  final String peer;

  /// The proof the host runs.
  final PeerMethod method;
  final List<ServedEntry> entries;

  static final _peerId = RegExp(r'^sha256:[0-9a-f]{64}$');

  /// The list `canoa-host serve` writes, or null for anything else: another server's page, or a list with an entry whose
  /// address is not a WebSocket's.
  static ServedHost? parse(String text) {
    try {
      final value = jsonDecode(text);
      if (value is! Map) return null;
      final [peer, method, entries] = [value['peer'], value['method'], value['entries']];
      if (peer is! String || !_peerId.hasMatch(peer) || method is! String || entries is! List) return null;
      final served = <ServedEntry>[];
      for (final entry in entries) {
        if (entry is! Map) return null;
        final [name, kind, url] = [entry['name'], entry['kind'], entry['url']];
        if (name is! String || kind is! String || url is! String) return null;
        final address = Uri.tryParse(url);
        if (address == null || !{'ws', 'wss'}.contains(address.scheme) || !address.hasAuthority) return null;
        served.add(ServedEntry(name: name, kind: kind, url: url));
      }
      return ServedHost(peer: peer, method: PeerMethod.parse(method), entries: served);
    } on FormatException {
      return null;
    } on ArgumentError {
      return null;
    }
  }
}

/// Reads `canoa-host.json` beside this page, as `canoa-host serve --web` serves it: null where the app is no web page or
/// the page came from anything else.
Future<ServedHost?> loadServedHost({http.Client? client, Uri? base}) async {
  if (!_inPage && base == null) return null;
  final ask = client ?? http.Client();
  try {
    final response = await ask.get((base ?? Uri.base).resolve('canoa-host.json'));
    return response.statusCode == 200 ? ServedHost.parse(response.body) : null;
  } catch (_) {
    return null;
  } finally {
    if (client == null) ask.close();
  }
}
