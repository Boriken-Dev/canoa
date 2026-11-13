/// MCP servers from a registry, as the official MCP Registry's API lists them and its `server.json` describes them
/// (https://github.com/modelcontextprotocol/registry, `docs/reference/server-json` and `docs/reference/api`).
///
/// Everything here is reading and planning: a listing is parsed, what installing one would write is worked out from the
/// `server.json` that was fetched, and nothing runs. A registry is advertising: what it says is shown as its claim, and
/// installing writes a server definition that no agent starts until the person allows it.
library;

import 'dart:convert';

import 'package:crypto/crypto.dart';

import '../config/canoa_config.dart' show McpOrigin;
import '../profile/profile.dart' show isEntryName, isEnvName;

/// The official registry, which a person may use or leave: any registry that serves the same API can be added beside it.
const officialRegistry = 'https://registry.modelcontextprotocol.io';

/// The most a listing body may hold, so a registry cannot make the app read without end.
const maxRegistryBytes = 4 * 1024 * 1024;

/// Reads the text at a URI; the app's own implementation uses HTTPS only.
typedef RegistryFetch = Future<String> Function(Uri uri);

/// Why a registry or its answer is refused, said to the person.
final class RegistryRefusal implements Exception {
  const RegistryRefusal(this.message);
  final String message;
  @override
  String toString() => message;
}

/// The list endpoint of [base] for one page: `GET <base>/v0.1/servers?limit=&cursor=&search=`. Only an `https` registry is read,
/// or `http` on the loopback address for a registry run on this computer.
Uri listUri(String base, {String? search, String? cursor, int limit = 30}) {
  final uri = Uri.tryParse(base);
  final loopback = uri != null && (uri.host == 'localhost' || uri.host == '127.0.0.1' || uri.host == '::1' || uri.host == '[::1]');
  if (uri == null || !uri.hasAuthority || uri.userInfo.isNotEmpty || uri.hasQuery || uri.hasFragment ||
      !(uri.scheme == 'https' || (uri.scheme == 'http' && loopback))) {
    throw const RegistryRefusal('A registry is an https address with no user information or query');
  }
  final path = uri.path.endsWith('/') ? uri.path.substring(0, uri.path.length - 1) : uri.path;
  return uri.replace(path: '$path/v0.1/servers', queryParameters: {
    'limit': '$limit',
    if (cursor != null && cursor.isNotEmpty) 'cursor': cursor,
    if (search != null && search.trim().isNotEmpty) 'search': search.trim(),
  });
}

Object? _canonical(Object? value) => switch (value) {
  final Map<dynamic, dynamic> map => {for (final key in (map.keys.map((key) => '$key').toList()..sort())) key: _canonical(map[key])},
  final List<dynamic> list => [for (final item in list) _canonical(item)],
  _ => value,
};

/// `sha256:` and the hex digest of [server] written with its keys in order and no spacing: what is shown and pinned.
String serverDigest(Map<String, Object?> server) =>
    'sha256:${sha256.convert(utf8.encode(jsonEncode(_canonical(server)))).toString()}';

Map<String, Object?> _map(Object? value) =>
    value is Map ? {for (final MapEntry(:key, :value) in value.entries) '$key': value} : const {};

List<Map<String, Object?>> _maps(Object? value) => value is List ? [for (final item in value) if (item is Map) _map(item)] : const [];

String? _text(Object? value) => value is String && value.isNotEmpty ? value : null;

/// One server a registry lists: its `server.json`, and what the registry says about this version of it.
final class McpRegistryEntry {
  McpRegistryEntry(this.server, {this.isLatest = true, this.status}) : digest = serverDigest(server);

  final Map<String, Object?> server;

  /// Whether the registry marks this version as its latest; true when it says nothing.
  final bool isLatest;
  final String? status;

  /// The digest of the `server.json` as fetched.
  final String digest;

  String get name => _text(server['name']) ?? '';
  String get title => _text(server['title']) ?? name;
  String get description => _text(server['description']) ?? '';
  String get version => _text(server['version']) ?? '';
  String? get websiteUrl => _text(server['websiteUrl']);
  String? get repositoryUrl => _text(_map(server['repository'])['url']);

  /// The name an install gives it in `[profile.mcp]`: the part after the last `/`, in the characters a name may have.
  String get installName {
    var base = name.contains('/') ? name.substring(name.lastIndexOf('/') + 1) : name;
    base = base.replaceAll(RegExp(r'[^A-Za-z0-9_-]'), '-');
    base = base.replaceAll(RegExp(r'-{2,}'), '-').replaceAll(RegExp(r'^-+|-+$'), '');
    if (base.length > 64) base = base.substring(0, 64);
    return base;
  }

  McpOrigin origin(String registry) => McpOrigin(registry: registry, server: name, version: version, digest: digest);
}

/// One page of a registry's listing and the cursor of the next, if any.
final class McpRegistryPage {
  const McpRegistryPage(this.entries, this.next);
  final List<McpRegistryEntry> entries;
  final String? next;
}

/// Reads a list answer: `{"servers": [{"server": {...}, "_meta": {...}}], "metadata": {"nextCursor": ...}}`. Entries the
/// registry marks as other than its latest version, and entries with no name, are left out; a body that is not a list is refused.
McpRegistryPage parseRegistryPage(String body) {
  if (body.length > maxRegistryBytes) throw const RegistryRefusal('The registry\'s answer is larger than Canoa reads');
  final Object? decoded;
  try {
    decoded = jsonDecode(body);
  } on FormatException {
    throw const RegistryRefusal('The registry did not answer with JSON');
  }
  final servers = decoded is Map ? decoded['servers'] : null;
  if (servers is! List) throw const RegistryRefusal('The registry\'s answer is not a list of servers');
  final entries = <McpRegistryEntry>[];
  for (final item in servers) {
    final server = _map(_map(item)['server']);
    if (_text(server['name']) == null) continue;
    final official = _map(_map(_map(item)['_meta'])['io.modelcontextprotocol.registry/official']);
    final latest = official['isLatest'] != false;
    if (!latest) continue;
    entries.add(McpRegistryEntry(server, isLatest: latest, status: _text(official['status'])));
  }
  final next = _text(_map(_map(decoded)['metadata'])['nextCursor']);
  return McpRegistryPage(entries, next);
}

/// What an install needs from the person's own computer, and how it is satisfied: never a value, only the name of the
/// variable Canoa reads when a session starts.
final class McpNeed {
  const McpNeed(this.name, this.where, {this.secret = false, this.required = true, this.description, this.reads});

  /// The variable, header or argument.
  final String name;

  /// `environment variable`, `header` or `argument`.
  final String where;
  final bool secret, required;
  final String? description;

  /// The environment variable Canoa reads for it, when it reads one.
  final String? reads;

  String describe() => reads != null
      ? '$where $name: read from your environment variable $reads when a session starts'
          '${secret ? '; it holds a secret, which Canoa never writes to canoa.cfg' : ''}'
      : '$where $name${required ? ' (required)' : ''}';
}

/// One way to run a server, from its `packages` or `remotes`, and what installing it would write.
final class McpOption {
  const McpOption({required this.label, required this.kind, this.definition, this.needs = const [], this.runtime,
      this.commandLine, this.refusal});

  /// `npm package @scope/name 1.2.3`, `remote https://...`.
  final String label;

  /// `package` or `remote`.
  final String kind;

  /// The `[profile.mcp.NAME]` table this would write, in the profile's own schema; null when [refusal] is set.
  final Map<String, dynamic>? definition;
  final List<McpNeed> needs;

  /// The program a package needs on this computer (`npx`, `uvx`, `docker`); null for a remote.
  final String? runtime;

  /// The program and arguments, or the URL, as shown.
  final String? commandLine;

  /// Why Canoa will not install this one, naming what it does not do.
  final String? refusal;
}

String _envName(String name) {
  final shaped = name.toUpperCase().replaceAll(RegExp(r'[^A-Z0-9_]'), '_');
  return RegExp(r'^[0-9]').hasMatch(shaped) ? '_$shaped' : shaped;
}

/// The `{name}` placeholders in [text] replaced from [variables]' defaults; the first with none is returned in [missing].
({String text, String? missing}) _fill(String text, Map<String, Object?> variables) {
  String? missing;
  final out = text.replaceAllMapped(RegExp(r'\{([^{}]+)\}'), (match) {
    final value = _text(_map(variables[match[1]])['default']);
    if (value == null) {
      missing ??= match[1];
      return match[0]!;
    }
    return value;
  });
  return (text: out, missing: missing);
}

({List<String> words, String? refusal}) _arguments(List<Map<String, Object?>> items, List<McpNeed> needs) {
  final words = <String>[];
  for (final item in items) {
    final variables = _map(item['variables']);
    final raw = _text(item['value']) ?? _text(item['default']);
    final hint = _text(item['valueHint']);
    final isNamed = item['type'] == 'named';
    final name = _text(item['name']);
    if (isNamed && name == null) return (words: words, refusal: 'an argument with a type named and no name');
    if (raw == null) {
      if (item['isRequired'] == true) {
        return (words: words, refusal: 'it needs ${isNamed ? name : (hint ?? 'a positional argument')}, a value Canoa cannot choose for you');
      }
      if (isNamed && item['format'] == 'boolean') words.add(name!);
      continue;
    }
    final filled = _fill(raw, variables);
    if (filled.missing != null) {
      return (words: words, refusal: 'its argument names {${filled.missing}}, which the registry gives no default for');
    }
    if (isNamed) words.add(name!);
    words.add(filled.text);
  }
  return (words: words, refusal: null);
}

/// What installing [server]'s `packages` and `remotes` would write, one option each, in the registry's order. A server with
/// neither has no option.
List<McpOption> optionsOf(Map<String, Object?> server) => [
  for (final remote in _maps(server['remotes'])) _remote(remote),
  for (final package in _maps(server['packages'])) _package(package),
];

McpOption _remote(Map<String, Object?> remote) {
  final type = _text(remote['type']) ?? '';
  final url = _text(remote['url']) ?? '';
  final label = 'remote $url';
  McpOption refuse(String why) => McpOption(label: label, kind: 'remote', refusal: why);
  if (type != 'streamable-http' && type != 'sse') return refuse('Canoa connects to a remote server over streamable HTTP or SSE, and this one is $type');
  final filled = _fill(url, _map(remote['variables']));
  if (filled.missing != null) return refuse('its address has {${filled.missing}}, which the registry gives no default for');
  final uri = Uri.tryParse(filled.text);
  if (uri == null || !uri.hasAuthority || uri.userInfo.isNotEmpty || !(uri.scheme == 'https' || uri.scheme == 'http')) {
    return refuse('its address is not an http or https address without user information');
  }
  if (uri.scheme == 'http') {
    final loopback = uri.host == 'localhost' || uri.host == '127.0.0.1' || uri.host == '::1';
    if (!loopback) return refuse('its address is plain http to another computer, which Canoa does not connect to');
  }
  final needs = <McpNeed>[];
  final headers = <String, dynamic>{};
  for (final header in _maps(remote['headers'])) {
    final name = _text(header['name']);
    if (name == null || !isEntryName(name)) return refuse('a header of the registry\'s is not a name Canoa writes');
    final literal = _text(header['value']) ?? _text(header['default']);
    if (literal != null && header['isSecret'] != true) {
      final filledHeader = _fill(literal, _map(header['variables']));
      if (filledHeader.missing != null) return refuse('header $name has {${filledHeader.missing}}, which the registry gives no default for');
      headers[name] = {'value': filledHeader.text};
    } else if (header['isRequired'] == true || header['isSecret'] == true) {
      final reads = _envName(name);
      headers[name] = {'from_env': reads};
      needs.add(McpNeed(name, 'header', secret: header['isSecret'] == true, required: header['isRequired'] == true,
          description: _text(header['description']), reads: reads));
    }
  }
  return McpOption(
    label: label,
    kind: 'remote',
    commandLine: filled.text,
    needs: needs,
    definition: {'type': type == 'sse' ? 'sse' : 'http', 'url': filled.text, if (headers.isNotEmpty) 'headers': headers},
  );
}

McpOption _package(Map<String, Object?> package) {
  final registryType = _text(package['registryType']) ?? '';
  final identifier = _text(package['identifier']) ?? '';
  final version = _text(package['version']);
  final label = '$registryType package $identifier${version == null ? '' : ' $version'}';
  McpOption refuse(String why) => McpOption(label: label, kind: 'package', refusal: why);
  final transport = _text(_map(package['transport'])['type']) ?? 'stdio';
  if (transport != 'stdio') return refuse('Canoa starts a package that speaks MCP on its standard streams, and this one is $transport');
  final hint = _text(package['runtimeHint']);
  late final String runtime;
  late final List<String> lead, trail;
  switch (registryType) {
    case 'npm':
      runtime = hint ?? 'npx';
      lead = runtime == 'npx' ? const ['-y'] : const [];
      trail = [version == null ? identifier : '$identifier@$version'];
    case 'pypi':
      runtime = hint ?? 'uvx';
      lead = const [];
      trail = [version == null ? identifier : '$identifier==$version'];
    case 'oci':
      runtime = 'docker';
      lead = const ['run', '-i', '--rm'];
      trail = const [];
    case 'mcpb':
      return refuse('it is an MCP bundle (mcpb), which Canoa does not unpack or run');
    default:
      return refuse('Canoa builds no command for a $registryType package');
  }
  if (!RegExp(r'^@?[A-Za-z0-9][A-Za-z0-9._/@:+=~-]*$').hasMatch(identifier) || identifier.contains('..')) {
    return refuse('its identifier has characters Canoa does not put on a command line');
  }
  final needs = <McpNeed>[];
  final env = <String, dynamic>{};
  final dockerEnv = <String>[];
  for (final variable in _maps(package['environmentVariables'])) {
    final name = _text(variable['name']);
    if (name == null || !isEnvName(name)) return refuse('an environment variable of the registry\'s is not a name Canoa sets');
    final secret = variable['isSecret'] == true, required = variable['isRequired'] == true;
    final literal = _text(variable['value']) ?? _text(variable['default']);
    if (literal != null && !secret) {
      final filled = _fill(literal, _map(variable['variables']));
      if (filled.missing != null) return refuse('$name has {${filled.missing}}, which the registry gives no default for');
      env[name] = {'value': filled.text};
    } else if (required || secret) {
      env[name] = {'from_env': name};
      needs.add(McpNeed(name, 'environment variable', secret: secret, required: required,
          description: _text(variable['description']), reads: name));
    } else {
      continue;
    }
    if (registryType == 'oci') dockerEnv.addAll(['-e', name]);
  }
  final runtimeArgs = _arguments(_maps(package['runtimeArguments']), needs);
  if (runtimeArgs.refusal != null) return refuse(runtimeArgs.refusal!);
  final packageArgs = _arguments(_maps(package['packageArguments']), needs);
  if (packageArgs.refusal != null) return refuse(packageArgs.refusal!);
  final args = registryType == 'oci'
      ? [...lead, ...runtimeArgs.words, ...dockerEnv, identifier, ...packageArgs.words]
      : [...lead, ...runtimeArgs.words, ...trail, ...packageArgs.words];
  return McpOption(
    label: label,
    kind: 'package',
    runtime: runtime,
    commandLine: [runtime, ...args].join(' '),
    needs: needs,
    definition: {'type': 'stdio', 'command': runtime, 'args': args, if (env.isNotEmpty) 'env': env},
  );
}
