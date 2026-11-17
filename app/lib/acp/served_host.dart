import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import '../auth/peer_proof.dart';
import '../served/app_base.dart';
import '../served/navigate.dart';
import '../served/web_mode.dart';

/// Whether this is a web page; Flutter's own `kIsWeb` would keep the peer test page, which `dart compile js` builds, from
/// compiling.
const _inPage = bool.fromEnvironment('dart.library.js_interop');

/// One entry the host that served this page lists.
final class ServedEntry {
  const ServedEntry({required this.name, required this.kind, required this.url, this.harness, this.title});

  final String name;

  /// `agent`, `provider`, `tool` or `control`.
  final String kind;

  /// Its WebSocket address.
  final String url;

  /// The provider id of a `harness` entry; null for any other.
  final String? harness;

  /// The entry's `title`; null shows its name.
  final String? title;
}

/// What the `canoa` that served this page says it serves, from `canoa.json` beside the page. It only fills the
/// connect dialog: connecting still proves the host holds [peer]'s key, and the host still turns this browser away until
/// its key is accepted there.
final class ServedHost {
  const ServedHost({required this.peer, required this.method, required this.entries, this.control, this.os, this.user, this.web, this.admit});

  /// The account signed in to the `canoa portal` that served this page; null for any other server.
  final String? user;

  /// Where this page asks the host that serves it, which runs on the same computer as its client, to accept its key without a
  /// pairing step (the Office add-in on this computer); null for a host that offers nothing of the kind.
  final String? admit;

  /// The host's peer id.
  final String peer;

  /// The proof the host runs.
  final PeerMethod method;
  final List<ServedEntry> entries;

  /// The operating system the host runs on as Rust names it (`windows`, `linux`, `macos`), when it says; overlays are read for it.
  final String? os;

  /// Whether the person sits at the host's computer (`local`) or elsewhere (`remote`), when the host says.
  final WebMode? web;

  /// The host's control channel (its session list, folders, uploads and overlays), when it listens on an address a page may dial; a
  /// key reaches it only with the grants the host gives it. Not one of [entries]: it is no agent.
  final ServedEntry? control;

  static final _peerId = RegExp(r'^sha256:[0-9a-f]{64}$');

  /// The list `canoa serve` writes, or null for anything else: another server's page, or a list with an entry whose
  /// address is not a WebSocket's.
  static ServedHost? parse(String text) {
    try {
      final value = jsonDecode(text);
      if (value is! Map) return null;
      final [peer, method, entries] = [value['peer'], value['method'], value['entries']];
      if (peer is! String || !_peerId.hasMatch(peer) || method is! String || entries is! List) return null;
      final served = <ServedEntry>[];
      ServedEntry? control;
      for (final entry in entries) {
        if (entry is! Map) return null;
        final [name, kind, url] = [entry['name'], entry['kind'], entry['url']];
        if (name is! String || kind is! String || url is! String) return null;
        final address = Uri.tryParse(url);
        if (address == null || !{'ws', 'wss'}.contains(address.scheme) || !address.hasAuthority) return null;
        final [harness, title] = [entry['harness'], entry['title']];
        final listed = ServedEntry(
          name: name,
          kind: kind,
          url: url,
          harness: harness is String ? harness : null,
          title: title is String ? title : null,
        );
        if (kind == 'control') {
          control ??= listed;
        } else {
          served.add(listed);
        }
      }
      return ServedHost(
        peer: peer, method: PeerMethod.parse(method), entries: served, control: control,
        os: value['os'] is String ? value['os'] as String : null,
        web: WebMode.named(value['web']),
        user: value['user'] is String && (value['user'] as String).isNotEmpty ? value['user'] as String : null,
        admit: value['admit'] is String && (value['admit'] as String).startsWith('/') ? value['admit'] as String : null,
      );
    } on FormatException {
      return null;
    } on ArgumentError {
      return null;
    }
  }
}

/// Reads `canoa.json` beside this page, as `canoa serve --web` serves it: null where the app is no web page or
/// the page came from anything else. A `401` is a portal saying nobody is signed in: [signedOut] is told the sign-in address
/// (in a page, by default, the browser goes there and this never answers, since the page is leaving), and the answer is null.
Future<ServedHost?> loadServedHost({http.Client? client, Uri? base, void Function(Uri login)? signedOut}) async {
  if (!_inPage && base == null) return null;
  final ask = client ?? http.Client();
  try {
    final page = base ?? appBase();
    final response = await ask.get(page.resolve('canoa.json'));
    if (response.statusCode == 401) {
      final login = page.resolve('/portal/login');
      if (signedOut != null) {
        signedOut(login);
      } else if (_inPage) {
        navigateTo(login);
        await Completer<void>().future;
      }
      return null;
    }
    return response.statusCode == 200 ? ServedHost.parse(response.body) : null;
  } catch (_) {
    return null;
  } finally {
    if (client == null) ask.close();
  }
}
