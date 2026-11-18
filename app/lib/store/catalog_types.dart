/// What the host says of each Store type that reads catalogs (`store/catalogs`): what it calls them, the catalogs canoa.cfg lists
/// under it, and the catalog sources whose form can add one. The words and the forms are the host's, which got them from the type
/// and the source providers: nothing of them is kept here.
library;

import '../config/catalog_specs.dart';
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// What a Store type calls its catalogs, and what they list.
final class CatalogWords {
  const CatalogWords({required this.noun, required this.plural, required this.lists});

  /// One catalog, in the type's word: `catalog`, `source`, `registry`.
  final String noun;
  final String plural;

  /// What the type lists: `overlays`, `skills`.
  final String lists;

  /// What the words are before the host has said any.
  static const generic = CatalogWords(noun: 'catalog', plural: 'catalogs', lists: 'entries');
}

/// One field of a catalog source's form.
final class SourceField {
  const SourceField({required this.name, required this.label, required this.shape, this.required = false, this.defaultText, this.help});

  factory SourceField.fromAnswer(Map<Object?, Object?> each) => SourceField(
    name: '${each['name']}',
    label: '${each['label'] ?? each['name']}',
    shape: '${each['shape'] ?? 'text'}',
    required: each['required'] == true,
    defaultText: each['default'] as String?,
    help: each['help'] as String?,
  );

  final String name, label;

  /// `text`, `list` (one word a line), `number` or `path`.
  final String shape;
  final bool required;
  final String? defaultText, help;
}

/// A catalog source the host offers a type: the form its fields make.
final class CatalogSourceForm {
  const CatalogSourceForm({required this.id, required this.title, this.help, this.extension, this.fields = const []});

  factory CatalogSourceForm.fromAnswer(Map<Object?, Object?> each) => CatalogSourceForm(
    id: '${each['id']}',
    title: '${each['title'] ?? each['id']}',
    help: each['help'] as String?,
    extension: each['extension'] as String?,
    fields: [for (final field in each['fields'] as List<Object?>? ?? const []) if (field is Map<Object?, Object?>) SourceField.fromAnswer(field)],
  );

  final String id, title;
  final String? help;

  /// The extension that brought it, none for one built in.
  final String? extension;
  final List<SourceField> fields;
}

/// One catalog of a type as the host lists it.
final class CatalogRow {
  const CatalogRow({required this.label, required this.spec, this.isDefault = false, this.fixed = false, this.problem});

  final String label;
  final CatalogSpec? spec;

  /// Whether it is the one the type offers until it is removed.
  final bool isDefault;

  /// Whether the type always offers it: it cannot be removed or replaced.
  final bool fixed;

  /// Why the host reads no catalog from the entry.
  final String? problem;

  /// What to show of where it comes from.
  String get shown => spec == null ? '' : (spec!.source ?? '${spec!.provider}: ${spec!.fields.values.map((v) => v is List ? v.join(' ') : '$v').join(', ')}');
}

/// One Store type that reads catalogs.
final class CatalogType {
  const CatalogType({required this.id, required this.title, required this.words, this.defaultLabel, this.rows = const [], this.sources = const []});

  factory CatalogType.fromAnswer(Map<Object?, Object?> each) {
    final words = each['words'];
    final def = each['default'];
    return CatalogType(
      id: '${each['type']}',
      title: '${each['title'] ?? each['type']}',
      words: words is Map<Object?, Object?>
          ? CatalogWords(noun: '${words['noun']}', plural: '${words['plural']}', lists: '${words['lists']}')
          : CatalogWords.generic,
      defaultLabel: def is Map<Object?, Object?> ? def['label'] as String? : null,
      rows: [
        for (final row in each['catalogs'] as List<Object?>? ?? const [])
          if (row is Map<Object?, Object?>)
            CatalogRow(
              label: '${row['label']}',
              spec: row['provider'] is String
                  ? CatalogSpec('${row['provider']}', {for (final MapEntry(:key, :value) in ((row['fields'] as Map?) ?? const {}).entries) '$key': value})
                  : null,
              isDefault: row['default'] == true,
              fixed: row['fixed'] == true,
              problem: row['problem'] as String?,
            ),
      ],
      sources: [for (final source in each['sources'] as List<Object?>? ?? const []) if (source is Map<Object?, Object?>) CatalogSourceForm.fromAnswer(source)],
    );
  }

  final String id, title;
  final CatalogWords words;

  /// The label of the catalog the type offers until the person removes it.
  final String? defaultLabel;
  final List<CatalogRow> rows;

  /// The catalog sources that serve the type, the one that reads what the type reads itself first.
  final List<CatalogSourceForm> sources;
}

/// Reads every Store type that has catalogs.
typedef CatalogTypesRead = Future<List<CatalogType>> Function();

/// Why the host could not read a catalog of Skills or MCP servers, in its words.
final class CatalogReadFailed implements Exception {
  const CatalogReadFailed(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Reads one catalog of the type [type] (`skills` or `mcps`) through the host's `store/catalog`: the answer as the host gives it.
/// [search] and [cursor] ask a registry for its next page. Throws [CatalogReadFailed].
typedef CatalogRead = Future<Map<Object?, Object?>> Function(String type, String label, {String? search, String? cursor});

/// `store/catalog` over [control], or null where the app has no host.
CatalogRead? controlCatalogRead(HostControl? control) => control == null
    ? null
    : (type, label, {search, cursor}) async {
        try {
          final client = await control.ready();
          final answer = await client.call(
            'store/catalog',
            {'type': type, 'label': label, if (search != null && search.isNotEmpty) 'search': search, 'cursor': ?cursor},
            null,
            const Duration(seconds: 120),
          );
          return answer is Map<Object?, Object?> ? answer : const {};
        } on ControlFailed catch (error) {
          throw CatalogReadFailed(error.message);
        } on CatalogReadFailed {
          rethrow;
        } on Object catch (error) {
          throw CatalogReadFailed('The host could not be reached: $error');
        }
      };

/// The host's `store/catalogs`, asked over its control channel.
final class ControlCatalogTypes {
  ControlCatalogTypes(this._call);

  static ControlCatalogTypes? over(HostControl? control) => control == null
      ? null
      : ControlCatalogTypes((method, params) async {
          final client = await control.ready();
          return client.call(method, params, null, const Duration(seconds: 30));
        });

  final Future<Object?> Function(String method, Map<String, Object?> params) _call;

  Future<List<CatalogType>> read() async {
    try {
      return catalogTypesFromAnswer(await _call('store/catalogs', const {}));
    } on ControlFailed catch (error) {
      throw StateError(error.message);
    }
  }
}

/// The types in the answer of `store/catalogs`.
List<CatalogType> catalogTypesFromAnswer(Object? answer) => [
  if (answer is Map<Object?, Object?>)
    for (final each in answer['types'] as List<Object?>? ?? const [])
      if (each is Map<Object?, Object?>) CatalogType.fromAnswer(each),
];
