import '../brand.dart';

/// A saved connection to a host or agent elsewhere, as `[connections.NAME]` in canoa.cfg holds it. It names where to connect and
/// which key to expect there, never a key, a password or a token: the proof at connect is what decides whether the other end holds
/// the pinned id.
final class Connection {
  const Connection({
    required this.name,
    required this.kind,
    required this.address,
    required this.peer,
    required this.method,
    this.transport = ConnectionTransport.websocket,
  });

  /// What the person calls it; also its key in the file.
  final String name;
  final ConnectionKind kind;

  /// A Canoa host's web address (what `canoa serve` lists the entries at), or a direct agent's WebSocket or WebTransport endpoint.
  final String address;

  /// The pinned peer id (`sha256:` and 64 hex digits).
  final String peer;

  /// The proof the other end runs: `noise` or `webcrypto`.
  final String method;

  /// How [address] is dialled for a direct agent; a Canoa host lists its own.
  final ConnectionTransport transport;

  static final _peerId = RegExp(r'^sha256:[0-9a-f]{64}$');
  static const methods = {'noise', 'webcrypto'};
  static const keys = {'kind', 'address', 'transport', 'peer', 'method'};

  /// Whether [name] can name a connection: 1 to 64 characters, no control characters, no leading or trailing space.
  static bool isName(String name) =>
      name.isNotEmpty && name.length <= 64 && name == name.trim() && !name.runes.any((rune) => rune < 0x20 || rune == 0x7f);

  /// The reason [address] cannot be one for [kind], or null.
  static String? addressProblem(ConnectionKind kind, ConnectionTransport transport, String address) {
    final uri = Uri.tryParse(address.trim());
    if (uri == null || !uri.hasAuthority || uri.host.isEmpty) return 'Enter an address with a host, such as wss://host:port/acp.';
    if (uri.userInfo.isNotEmpty) return 'An address holds no user name or password.';
    final schemes = switch ((kind, transport)) {
      (ConnectionKind.canoa, _) => const {'http', 'https'},
      (ConnectionKind.acp, ConnectionTransport.websocket) => const {'ws', 'wss'},
      (ConnectionKind.acp, ConnectionTransport.webtransport) => const {'https'},
    };
    if (!schemes.contains(uri.scheme)) return 'That address starts with ${schemes.map((s) => '$s://').join(' or ')}.';
    return null;
  }

  /// A connection read from [table], or a [FormatException] naming what is wrong.
  factory Connection.read(String name, Object? table) {
    if (!isName(name)) throw FormatException('${Brand.configFile}: connections.$name: a name is 1 to 64 characters, with no control characters');
    if (table is! Map) throw FormatException('${Brand.configFile}: connections.$name must be a table');
    for (final key in table.keys) {
      if (!keys.contains(key)) throw FormatException('${Brand.configFile}: connections.$name: invalid key $key');
    }
    final kind = ConnectionKind.named(table['kind']);
    if (kind == null) throw FormatException('${Brand.configFile}: connections.$name: kind is "${ConnectionKind.canoa.name}" or "${ConnectionKind.acp.name}"');
    final transport = table['transport'] == null ? ConnectionTransport.websocket : ConnectionTransport.named(table['transport']);
    if (transport == null) throw FormatException('${Brand.configFile}: connections.$name: transport is "websocket" or "webtransport"');
    final address = table['address'];
    if (address is! String) throw FormatException('${Brand.configFile}: connections.$name: address is text');
    final problem = addressProblem(kind, transport, address);
    if (problem != null) throw FormatException('${Brand.configFile}: connections.$name: $problem');
    final peer = table['peer'];
    if (peer is! String || !_peerId.hasMatch(peer)) {
      throw FormatException('${Brand.configFile}: connections.$name: peer is sha256: and 64 hex digits');
    }
    final method = table['method'];
    if (method is! String || !methods.contains(method)) {
      throw FormatException('${Brand.configFile}: connections.$name: method is "noise" or "webcrypto"');
    }
    return Connection(name: name, kind: kind, address: address.trim(), peer: peer, method: method, transport: transport);
  }

  Map<String, Object> toTable() => {
    'kind': kind.name,
    'address': address,
    if (kind == ConnectionKind.acp && transport != ConnectionTransport.websocket) 'transport': transport.name,
    'peer': peer,
    'method': method,
  };

  Connection renamed(String to) => Connection(name: to, kind: kind, address: address, peer: peer, method: method, transport: transport);
}

/// What the other end is.
enum ConnectionKind {
  /// A Canoa host, reached over a paired peer: it lists its agents and what it can do.
  canoa('${Brand.name} host'),

  /// One agent speaking ACP at an endpoint: it lists nothing and offers nothing beyond the session.
  acp('Direct ACP endpoint');

  const ConnectionKind(this.label);
  final String label;

  static ConnectionKind? named(Object? name) => values.where((kind) => kind.name == name).firstOrNull;
}

/// How a direct agent's endpoint is dialled.
enum ConnectionTransport {
  websocket,
  webtransport;

  static ConnectionTransport? named(Object? name) => values.where((transport) => transport.name == name).firstOrNull;
}
