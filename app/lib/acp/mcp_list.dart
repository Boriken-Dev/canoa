import 'dart:async';
import 'dart:convert';
import 'dart:math';

/// The host's ACP session-stream method for listing attached MCP servers' tools and resources.
const mcpListMethod = '_canoa/mcp/list';

/// One tool offered by an MCP server attached to a session.
final class McpTool {
  const McpTool({required this.server, required this.name, this.title, this.description});

  final String server;
  final String name;
  final String? title;
  final String? description;

  static McpTool? parse(Object? raw) {
    if (raw is! Map || raw['server'] is! String || raw['name'] is! String) return null;
    final server = (raw['server'] as String).trim();
    final name = (raw['name'] as String).trim();
    if (server.isEmpty || name.isEmpty) return null;
    String? opt(String key) => raw[key] is String && (raw[key] as String).trim().isNotEmpty ? (raw[key] as String).trim() : null;
    return McpTool(server: server, name: name, title: opt('title'), description: opt('description'));
  }
}

/// One resource offered by an MCP server attached to a session.
final class McpResource {
  const McpResource({required this.server, required this.uri, required this.name, this.title, this.description, this.mimeType});

  final String server;
  final String uri;
  final String name;
  final String? title;
  final String? description;
  final String? mimeType;

  static McpResource? parse(Object? raw) {
    if (raw is! Map || raw['server'] is! String || raw['uri'] is! String || raw['name'] is! String) return null;
    final server = (raw['server'] as String).trim();
    final uri = (raw['uri'] as String).trim();
    final name = (raw['name'] as String).trim();
    if (server.isEmpty || uri.isEmpty || name.isEmpty) return null;
    String? opt(String key) => raw[key] is String && (raw[key] as String).trim().isNotEmpty ? (raw[key] as String).trim() : null;
    return McpResource(
      server: server,
      uri: uri,
      name: name,
      title: opt('title'),
      description: opt('description'),
      mimeType: opt('mimeType'),
    );
  }
}

/// Status reported for one attached MCP server (`ok`, `pending` or `error` with reason).
final class McpServerStatus {
  const McpServerStatus({required this.name, required this.ok, this.pending = false, this.tools = 0, this.resources = 0, this.error});

  final String name;
  final bool ok;

  /// The agent starts this server, so its tools are listed when the agent reports them.
  final bool pending;
  final int tools;
  final int resources;
  final String? error;

  static McpServerStatus? parse(Object? raw) {
    if (raw is! Map || raw['name'] is! String) return null;
    final name = (raw['name'] as String).trim();
    if (name.isEmpty) return null;
    return McpServerStatus(
      name: name,
      ok: raw['status'] == 'ok',
      pending: raw['status'] == 'pending',
      tools: raw['tools'] is int ? raw['tools'] as int : 0,
      resources: raw['resources'] is int ? raw['resources'] as int : 0,
      error: raw['error'] is String && (raw['error'] as String).trim().isNotEmpty ? (raw['error'] as String).trim() : null,
    );
  }
}

/// Snapshot of every attached MCP server's tools, resources, and status for a session.
final class McpInventory {
  const McpInventory({this.tools = const [], this.resources = const [], this.servers = const []});

  final List<McpTool> tools;
  final List<McpResource> resources;
  final List<McpServerStatus> servers;
}

/// Session-channel extension client for [`mcpListMethod`], intercepting only replies to its own unguessable request ids.
final class McpListWire {
  McpListWire(this._send);

  final Future<void> Function(String) _send;
  final Map<String, Completer<Map<String, Object?>?>> _waiting = {};
  int _next = 0;
  bool _closed = false;
  static final _random = Random.secure();

  bool takes(String message) {
    if (_waiting.isEmpty) return false;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return false;
    }
    if (decoded is! Map || decoded['method'] != null) return false;
    final id = decoded['id'];
    if (id is! String) return false;
    final waiting = _waiting.remove(id);
    if (waiting == null) return false;
    if (!waiting.isCompleted) {
      if (decoded['error'] != null) {
        waiting.complete(null);
      } else if (decoded['result'] case final Map raw) {
        waiting.complete(Map<String, Object?>.from(raw));
      } else {
        waiting.complete(null);
      }
    }
    return true;
  }

  Future<Map<String, Object?>?> _ask(Map<String, Object?> params) async {
    if (_closed) return null;
    final nonce = [for (var i = 0; i < 8; i++) _random.nextInt(256).toRadixString(16).padLeft(2, '0')].join();
    final id = 'canoa-mcp-${_next++}-$nonce';
    final waiting = Completer<Map<String, Object?>?>();
    _waiting[id] = waiting;
    try {
      await _send(jsonEncode({'jsonrpc': '2.0', 'id': id, 'method': mcpListMethod, 'params': params}));
      return await waiting.future;
    } on Object {
      return null;
    } finally {
      _waiting.remove(id);
    }
  }

  /// Queries all pages of [`mcpListMethod`] for [sessionId]; returns null if the host refused it or the connection closed.
  /// The host bounds how long a server may take (`[host.mcp_list]`), so the wait has no limit of its own.
  Future<McpInventory?> list(String sessionId) async {
    final tools = <McpTool>[];
    final resources = <McpResource>[];
    var servers = const <McpServerStatus>[];
    String? cursor;
    for (var page = 0;; page++) {
      final result = await _ask({'sessionId': sessionId, 'cursor': ?cursor});
      if (result == null) return page == 0 ? null : McpInventory(tools: tools, resources: resources, servers: servers);
      if (result['tools'] case final List rawTools) {
        for (final item in rawTools) {
          if (McpTool.parse(item) case final parsed?) tools.add(parsed);
        }
      }
      if (result['resources'] case final List rawResources) {
        for (final item in rawResources) {
          if (McpResource.parse(item) case final parsed?) resources.add(parsed);
        }
      }
      if (page == 0) {
        if (result['servers'] case final List rawServers) {
          servers = [for (final item in rawServers) ?McpServerStatus.parse(item)];
        }
      }
      final next = result['nextCursor'];
      if (next is! String || next.isEmpty || next == cursor) break;
      cursor = next;
    }
    return McpInventory(tools: tools, resources: resources, servers: servers);
  }

  void close() {
    _closed = true;
    for (final waiting in _waiting.values) {
      if (!waiting.isCompleted) waiting.complete(null);
    }
    _waiting.clear();
  }
}
