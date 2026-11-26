import 'package:canoa/desktop/control_client.dart' show ControlFailed;

/// A host that merges the catalogs of two scopes as the real one does: the extension's default (a pinned `acme` of the type `whisper`,
/// with a shipped snapshot), canoa.cfg's entries by label (replace, add, `false`), and the types the host reads itself. It answers
/// `store/catalogs` and `store/catalog_set` and keeps every call.
final class ScriptedCatalogs {
  ScriptedCatalogs({this.refuse});

  /// What `store/catalog_set` is refused with, when set.
  String? refuse;

  final calls = <(String, Map<String, Object?>)>[];

  /// canoa.cfg's `[store.<type>.catalogs]`, by type then label: a string, a table, or `false`.
  final table = <String, Map<String, Object?>>{
    'overlays': {'team': 'https://example.org/o.git@main#catalog.toml'},
  };

  static const _acme = {'label': 'acme', 'provider': 'whisper_hf', 'fields': <String, Object?>{'repo': 'acme/models'}, 'pin': '5359861c'};

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    switch (method) {
      case 'store/catalogs':
        return {
          'schema': 1,
          'types': [_overlays(), _whisper()],
        };
      case 'store/catalog_set':
        if (refuse != null) throw ControlFailed(-32602, refuse!);
        final type = params['type']! as String, label = params['label']! as String, entry = params['entry'];
        final own = table.putIfAbsent(type, () => {});
        own.removeWhere((key, _) => key.toLowerCase() == label.toLowerCase());
        if (entry != null) own[label] = entry;
        return {'ok': true, 'type': type, 'label': label};
    }
    throw ControlFailed(-32601, 'no method $method');
  }

  Map<String, Object?> _overlays() => {
    'type': 'overlays',
    'title': 'Overlays',
    'words': {'noun': 'catalog', 'plural': 'catalogs', 'lists': 'overlays'},
    'default': {'label': 'boriken', 'source': 'https://example.org/boriken.git@main#catalog.toml'},
    'format': null,
    'catalogs': _merge('overlays', [
      {'label': 'boriken', 'provider': 'read', 'fields': {'source': 'https://example.org/boriken.git@main#catalog.toml'}, 'source': 'https://example.org/boriken.git@main#catalog.toml'},
    ], format: false),
    'sources': [_read],
  };

  Map<String, Object?> _whisper() => {
    'type': 'whisper',
    'title': 'Whisper',
    'words': {'noun': 'source', 'plural': 'sources', 'lists': 'models'},
    'format': 'canoa.models/1',
    'catalogs': _merge('whisper', [_acme], format: true),
    'sources': [_read, _hub],
  };

  static const _read = {
    'id': 'read',
    'title': 'Read an address or a file',
    'schema': {
      'type': 'object',
      'required': ['source'],
      'properties': {
        'source': {'type': 'string', 'title': 'Source'},
      },
    },
  };

  static const _hub = {
    'id': 'whisper_hf',
    'title': 'A Hugging Face repository',
    'extension': 'whisper',
    'schema': {
      'type': 'object',
      'required': ['repo'],
      'properties': {
        'repo': {'type': 'string', 'title': 'Repository'},
      },
    },
  };

  List<Map<String, Object?>> _merge(String type, List<Map<String, Object?>> defaults, {required bool format}) {
    final entries = table[type] ?? const {};
    Map<String, Object?>? held(String label) {
      for (final MapEntry(:key, :value) in entries.entries) {
        if (key.toLowerCase() == label.toLowerCase()) return {'label': key, 'value': value};
      }
      return null;
    }

    final rows = <Map<String, Object?>>[];
    for (final base in defaults) {
      final entry = held(base['label']! as String);
      if (entry == null) {
        rows.add({...base, 'default': true, 'origin': 'default', 'enabled': true, if (format) 'trust': 'pinned', if (format) 'snapshot': true, if (format) 'fetched': 0, if (format) 'from': 'snapshot'});
      } else if (entry['value'] == false) {
        rows.add({...base, 'default': true, 'origin': 'default', 'enabled': false, if (format) 'trust': 'pinned'});
      } else {
        rows.add(_row(entry['label']! as String, entry['value'], 'replaced', format: format));
      }
    }
    for (final MapEntry(:key, :value) in entries.entries) {
      if (value == false || defaults.any((base) => (base['label']! as String).toLowerCase() == key.toLowerCase())) continue;
      rows.add(_row(key, value, 'added', format: format));
    }
    return rows;
  }

  Map<String, Object?> _row(String label, Object? value, String origin, {required bool format}) {
    final table = value is Map ? value.cast<String, Object?>() : {'provider': 'read', 'source': value};
    final provider = table['provider'] as String? ?? 'read';
    final fields = {for (final MapEntry(:key, :value) in table.entries) if (!{'provider', 'pin', 'ttl', 'keys'}.contains(key)) key: value};
    return {
      'label': label,
      'provider': provider,
      'fields': fields,
      if (fields['source'] != null) 'source': fields['source'],
      'default': false,
      'origin': origin,
      'enabled': true,
      if (table['pin'] != null) 'pin': table['pin'],
      if (format && table['pin'] != null) 'trust': 'pinned',
    };
  }
}
