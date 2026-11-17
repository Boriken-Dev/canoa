// What a session relates to: the page a conversation started from, a mail conversation or a document. Kept with the session by
// the app, so a small client can list the sessions of what is in front of it first; it is never sent to the agent.
import '../workspace/site.dart';

/// What a session relates to, as a kind and a key the client that made it computed.
///
/// The key is opaque to the app: it compares keys of one kind for equality and shows [label]. A client that cannot keep the
/// thing itself in the key (a mail address, a document's path) hashes it before it makes the relation.
final class Relation {
  const Relation({required this.kind, required this.key, this.scope, this.label});

  /// The kind of a webpage's origin, whose key is `https://host[:port]` with a default port left out.
  static const originKind = 'origin';

  /// The kinds an Office task pane relates to: the mail conversation and the open document.
  static const conversationKind = 'conversation', documentKind = 'document';

  /// Which sort of thing it is; two relations of different kinds never relate.
  final String kind;

  /// The thing itself: what the sessions of one place have in common.
  final String key;

  /// A narrower key inside the same thing (the page's path inside its origin), for when the person asks for sessions of that one
  /// only; null where nothing is narrower.
  final String? scope;

  /// What the person calls it where it was made (the page's title), for a list; not part of what relates.
  final String? label;

  /// What the person reads for it: an origin without its `https://`, else the label or the key.
  String get display {
    if (kind == originKind) {
      final uri = Uri.tryParse(key);
      if (uri != null && uri.host.isNotEmpty) {
        final port = uri.hasPort ? ':${uri.port}' : '';
        return uri.scheme == 'https' ? '${uri.host}$port' : '${uri.scheme}://${uri.host}$port';
      }
    }
    final named = label?.trim() ?? '';
    return named.isEmpty ? key : named;
  }

  /// Whether a session recorded with [other] is related to this one. With [narrow] and a [scope] here, only a session recorded
  /// with the same scope is.
  bool relates(Relation? other, {bool narrow = false}) {
    if (other == null || other.kind != kind || other.key != key) return false;
    return !narrow || scope == null || other.scope == scope;
  }

  /// The same relation under another label, as a page's title changes while its address does not.
  Relation labelled(String? name) => Relation(kind: kind, key: key, scope: scope, label: name);

  Map<String, Object?> toJson() => {
    'kind': kind,
    'key': key,
    'scope': ?scope,
    'label': ?label,
  };

  static Relation? fromJson(Object? json) {
    if (json is! Map) return null;
    String? text(String name) => json[name] is String && (json[name] as String).isNotEmpty ? json[name] as String : null;
    final kind = text('kind'), key = text('key');
    if (kind == null || key == null) return null;
    return Relation(kind: kind, key: key, scope: text('scope'), label: text('label'));
  }

  /// The relation `kind:key` names, as a link between two pages of the app writes it; null when it does not.
  static Relation? parse(String? text, {String? scope, String? label}) {
    if (text == null) return null;
    final cut = text.indexOf(':');
    if (cut <= 0 || cut == text.length - 1) return null;
    final name = label?.trim() ?? '';
    return Relation(kind: text.substring(0, cut), key: text.substring(cut + 1), scope: scope, label: name.isEmpty ? null : name);
  }

  /// Written as [parse] reads it, with its scope and label alongside, which a link carries as separate parameters.
  String get link => '$kind:$key';

  @override
  bool operator ==(Object other) =>
      other is Relation && other.kind == kind && other.key == key && other.scope == scope && other.label == label;

  @override
  int get hashCode => Object.hash(kind, key, scope, label);

  @override
  String toString() => 'Relation($kind:$key${scope == null ? '' : ' $scope'})';
}

/// The relation of the webpage at [address]: its origin as the key and its path as the scope, [title] as the label. Null for an
/// address that is not an `http` or `https` page with a host name that [siteFqdn] accepts.
Relation? originRelation(String address, {String? title}) {
  final host = siteFqdn(address);
  final uri = Uri.tryParse(address.trim());
  if (host == null || uri == null) return null;
  final scheme = uri.scheme.toLowerCase();
  final standard = scheme == 'https' ? 443 : 80;
  final port = uri.hasPort && uri.port != standard ? ':${uri.port}' : '';
  final origin = '$scheme://$host$port';
  var path = uri.path;
  while (path.length > 1 && path.endsWith('/')) {
    path = path.substring(0, path.length - 1);
  }
  final name = title?.trim() ?? '';
  return Relation(kind: Relation.originKind, key: origin, scope: '$origin${path.isEmpty ? '/' : path}', label: name.isEmpty ? null : name);
}
