part of 'canoa_config.dart';

/// One `[loadouts.NAME]`: a named collection a session starts with. Its overlays are whole and apply in the order `[overlays]`
/// composes them.
final class Loadout {
  const Loadout({this.description = '', this.overlays = const [], this.skills = const [], this.mcp = const []});

  /// What the loadout is for.
  final String description;

  /// Overlays by the name `[overlays]` gives them.
  final List<String> overlays;

  /// The skills the person chose, by name.
  final List<String> skills;

  /// The servers of `[profile.mcp]` it starts.
  final List<String> mcp;

  static const _keys = {'description', 'overlays', 'skills', 'mcp'};

  /// The loadout [name] as canoa.cfg has it. Throws [FormatException] for a key it does not have or a name that is not one.
  factory Loadout.read(String name, Map<dynamic, dynamic> table) {
    List<String> names(String key, bool Function(String) valid) {
      final raw = table[key];
      if (raw == null) return const [];
      if (raw is! List || raw.any((item) => item is! String || !valid(item))) {
        throw FormatException('${Brand.configFile}: loadouts.$name.$key is a list of names');
      }
      final out = <String>[];
      for (final item in raw.cast<String>()) {
        if (!out.any((held) => held.toLowerCase() == item.toLowerCase())) out.add(item);
      }
      return out;
    }

    for (final key in table.keys) {
      if (!_keys.contains(key)) {
        throw FormatException('${Brand.configFile}: loadouts.$name.$key is not a loadout setting: description, overlays, skills and mcp are');
      }
    }
    final description = table['description'];
    if (description != null && description is! String) throw FormatException('${Brand.configFile}: loadouts.$name.description is text');
    return Loadout(
      description: (description as String?) ?? '',
      overlays: names('overlays', validOverlayName),
      skills: names('skills', validSkillName),
      mcp: names('mcp', isEntryName),
    );
  }

  Loadout copyWith({String? description, List<String>? overlays, List<String>? skills, List<String>? mcp}) => Loadout(
    description: description ?? this.description,
    overlays: overlays ?? this.overlays,
    skills: skills ?? this.skills,
    mcp: mcp ?? this.mcp,
  );

  /// This loadout with the server [name] in it, or without.
  Loadout withMcp(String name, bool on) => copyWith(mcp: [
        for (final each in mcp) if (each != name) each,
        if (on) name,
      ]);

  Map<String, dynamic> toTable() => {
    if (description.isNotEmpty) 'description': description,
    if (overlays.isNotEmpty) 'overlays': overlays,
    if (skills.isNotEmpty) 'skills': skills,
    if (mcp.isNotEmpty) 'mcp': mcp,
  };
}

/// Whether [name] is one a profile may have: 1 to 64 of ASCII letters, digits, spaces, `-`, `_` and `.`, with no space at either end.
/// The name of the profile that is always there and cannot be removed: what a session that names no other profile starts with.
const presetDefault = 'default';

/// A profile is chosen by its name in a footer, so it may read as words.
bool isPresetName(String name) => RegExp(r'^[A-Za-z0-9_.-]([A-Za-z0-9 _.-]{0,62}[A-Za-z0-9_.-])?$').hasMatch(name);

/// What a profile's name may be, in words.
const presetNameRule = 'A profile is named with 1 to 64 letters, digits, spaces, - _ or ., and no space at either end';

/// One `[profiles.NAME]`: the values a new session starts with, chosen by name in its footer. It is the session's layer: above the
/// person's `[profile]` and the agent's entry. The account and the permission mode's rules are never part of one.
final class Preset {
  const Preset({
    this.source,
    this.model,
    this.effort,
    this.mode,
    this.context,
    this.cacheLifetime,
    this.loadout,
    this.access = const ProfileAccess(),
    this.items = const {},
  });

  /// The label of the source it runs on; a session keeps its own when this is null.
  final String? source;
  final String? model, effort, mode;

  /// The starting instructions: text added to the session's initial context after every other layer's.
  final String? context;

  /// The cache lifetime's name (`default`, `off`, `short` or `long`).
  final String? cacheLifetime;

  /// The loadout it starts with; the default loadout when null.
  final String? loadout;

  /// The access it runs the agent with, part by part.
  final ProfileAccess access;

  /// What it holds for each item a provider adds (a proxy, an identity, whatever an extension offers), by item id: the values the
  /// person gave the item's form, by field name. The host applies them when a session starts; an item no provider offers any more
  /// keeps its values here and is not applied.
  final Map<String, Map<String, Object?>> items;

  static const _keys = {'source', 'model', 'effort', 'mode', 'access', 'context', 'cache_lifetime', 'loadout', 'items'};
  static const _lifetimes = {'default', 'off', 'short', 'long'};

  /// The preset [name] as canoa.cfg has it. Throws [FormatException] for a key it does not have, a value that is not text or a name
  /// that is not one; a loadout it names that the file no longer holds is not refused.
  factory Preset.read(String name, Map<dynamic, dynamic> table) {
    for (final MapEntry(:key, :value) in table.entries) {
      if (!_keys.contains(key)) {
        throw FormatException('${Brand.configFile}: profiles.$name.$key is not a profile setting: source, model, effort, mode, access, context, '
            'cache_lifetime, loadout and items are');
      }
      if (key == 'access' || key == 'items') continue;
      if (value is! String) throw FormatException('${Brand.configFile}: profiles.$name.$key is not text');
      if (value.isEmpty || value.contains('\u0000')) throw FormatException('${Brand.configFile}: profiles.$name.$key is empty or holds a NUL');
    }
    final lifetime = table['cache_lifetime'];
    if (lifetime != null && !_lifetimes.contains(lifetime)) {
      throw FormatException('${Brand.configFile}: profiles.$name.cache_lifetime is not default, off, short or long');
    }
    final loadout = table['loadout'];
    if (loadout != null && !isEntryName(loadout as String)) {
      throw FormatException('${Brand.configFile}: profiles.$name.loadout is not a loadout\'s name');
    }
    // The model, effort and mode are held to what a profile document holds them to.
    final Profile own;
    try {
      own = Profile.fromMap({
        for (final key in const ['model', 'effort', 'mode'])
          if (table[key] != null) key: table[key],
        if (table['access'] != null) 'access': table['access'],
      }, LayerKind.session);
    } on ProfileException catch (error) {
      throw FormatException('${Brand.configFile}: profiles.$name: $error');
    }
    final items = <String, Map<String, Object?>>{};
    final rawItems = table['items'];
    if (rawItems != null) {
      if (rawItems is! Map) throw FormatException('${Brand.configFile}: profiles.$name.items is a table');
      for (final MapEntry(:key, :value) in rawItems.entries) {
        if (!RegExp(r'^[a-z0-9_]{1,32}$').hasMatch('$key')) {
          throw FormatException('${Brand.configFile}: profiles.$name.items.$key is not an item\'s id');
        }
        if (value is! Map) throw FormatException('${Brand.configFile}: profiles.$name.items.$key is a table');
        items['$key'] = {for (final MapEntry(:key, :value) in value.entries) '$key': value};
      }
    }
    return Preset(
      items: items,
      source: table['source'] as String?,
      model: own.model,
      effort: own.effort,
      mode: own.mode,
      context: table['context'] as String?,
      cacheLifetime: lifetime as String?,
      loadout: loadout,
      access: own.access,
    );
  }

  /// This preset with the given values in place of its own; [clear] names the ones to take out.
  Preset copyWith({
    String? source,
    String? model,
    String? effort,
    String? mode,
    String? context,
    String? cacheLifetime,
    String? loadout,
    ProfileAccess? access,
    Map<String, Map<String, Object?>>? items,
    Set<String> clear = const {},
  }) => Preset(
    items: clear.contains('items') ? const {} : items ?? this.items,
    source: clear.contains('source') ? null : source ?? this.source,
    model: clear.contains('model') ? null : model ?? this.model,
    effort: clear.contains('effort') ? null : effort ?? this.effort,
    mode: clear.contains('mode') ? null : mode ?? this.mode,
    context: clear.contains('context') ? null : context ?? this.context,
    cacheLifetime: clear.contains('cache_lifetime') ? null : cacheLifetime ?? this.cacheLifetime,
    loadout: clear.contains('loadout') ? null : loadout ?? this.loadout,
    access: clear.contains('access') ? const ProfileAccess() : access ?? this.access,
  );

  /// The session's layer this preset makes: its model, effort and mode, and its starting instructions added to the context.
  Profile get sessionLayer => Profile(
    model: model,
    effort: effort,
    mode: mode,
    access: access,
    contexts: [if (context case final text?) ProfileContext.text(text)],
  );

  Map<String, dynamic> toTable() => {
    'source': ?source,
    'model': ?model,
    'effort': ?effort,
    'mode': ?mode,
    'context': ?context,
    'cache_lifetime': ?cacheLifetime,
    'loadout': ?loadout,
    if (!access.isEmpty)
      'access': {
        'sandbox': ?access.sandbox,
        'network': ?access.network,
        'approvals': ?access.approvals,
      },
    if (items.isNotEmpty) 'items': {for (final MapEntry(:key, :value) in items.entries) key: value},
  };

  @override
  bool operator ==(Object other) =>
      other is Preset &&
      other.source == source &&
      other.model == model &&
      other.effort == effort &&
      other.mode == mode &&
      other.context == context &&
      other.cacheLifetime == cacheLifetime &&
      other.loadout == loadout &&
      other.access.sandbox == access.sandbox &&
      other.access.network == access.network &&
      other.access.approvals == access.approvals &&
      _sameItems(other.items, items);

  static bool _sameItems(Map<String, Map<String, Object?>> a, Map<String, Map<String, Object?>> b) {
    if (a.length != b.length) return false;
    for (final MapEntry(:key, :value) in a.entries) {
      final other = b[key];
      if (other == null || value.length != other.length) return false;
      for (final MapEntry(key: field, value: held) in value.entries) {
        if (!other.containsKey(field) || '${other[field]}' != '$held') return false;
      }
    }
    return true;
  }

  @override
  int get hashCode => Object.hash(
    source, model, effort, mode, context, cacheLifetime, loadout, access.sandbox, access.network, access.approvals,
    Object.hashAll([for (final MapEntry(:key, :value) in items.entries) Object.hash(key, Object.hashAll([for (final e in value.entries) Object.hash(e.key, '${e.value}')]))]),
  );
}

/// What a session starts with once its loadout is resolved, as the host resolves it when the session starts.
final class ResolvedLoadout {
  const ResolvedLoadout({required this.name, required this.overlays, required this.missingOverlays, required this.skills, required this.mcp});

  final String name;

  /// Its installed overlays, in the order `[overlays]` composes them.
  final List<String> overlays;

  /// Overlays it names that `[overlays]` does not list.
  final List<String> missingOverlays;
  final List<String> skills, mcp;

  Map<String, Object?> toJson() =>
      {'name': name, 'overlays': overlays, 'missing_overlays': missingOverlays, 'skills': skills, 'mcp': mcp};
}

/// A session named a loadout `[loadouts]` does not hold.
final class UnknownLoadout implements Exception {
  const UnknownLoadout(this.name);

  final String name;

  @override
  String toString() => 'there is no loadout named $name';
}

/// Where an MCP server the store installed came from: a registry, the server's name and version there, and the digest of the
/// `server.json` that was shown before it was installed.
final class McpOrigin {
  const McpOrigin({required this.registry, required this.server, required this.version, required this.digest});

  /// The registry's address, as the person added it.
  final String registry;
  final String server, version;

  /// `sha256:` and 64 hex digits of the canonical `server.json`.
  final String digest;

  static McpOrigin? fromMap(Map<dynamic, dynamic> map) {
    if (map.keys.any((key) => !const {'registry', 'server', 'version', 'digest'}.contains(key))) return null;
    final registry = map['registry'], server = map['server'], version = map['version'], digest = map['digest'];
    if (registry is! String || server is! String || version is! String || digest is! String) return null;
    if (!RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(digest)) return null;
    return McpOrigin(registry: registry, server: server, version: version, digest: digest);
  }

  Map<String, dynamic> toMap() => {'registry': registry, 'server': server, 'version': version, 'digest': digest};
}
