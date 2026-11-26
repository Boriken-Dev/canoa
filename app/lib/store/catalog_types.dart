/// What the host says of each Store type that reads catalogs (`store/catalogs`): what it calls them, the catalogs canoa.cfg lists
/// under it, and the catalog sources whose form can add one. The words and the forms are the host's, which got them from the type
/// and the source providers: nothing of them is kept here.
library;

import '../config/catalog_specs.dart';
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../forms/schema_fields.dart' show schemaProperties;

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

/// A catalog source the host offers a type: the form it asks for, a guatiao schema and the guatiao-intake form that shows it.
final class CatalogSourceForm {
  const CatalogSourceForm({required this.id, required this.title, this.help, this.extension, this.schema = const {}, this.form = const {}});

  factory CatalogSourceForm.fromAnswer(Map<Object?, Object?> each) => CatalogSourceForm(
    id: '${each['id']}',
    title: '${each['title'] ?? each['id']}',
    help: each['help'] as String?,
    extension: each['extension'] as String?,
    schema: each['schema'] is Map ? (each['schema'] as Map).cast<String, Object?>() : const {},
    form: each['form'] is Map ? (each['form'] as Map).cast<String, Object?>() : const {},
  );

  final String id, title;
  final String? help;

  /// The extension that brought it, none for one built in.
  final String? extension;
  final Map<String, Object?> schema, form;

  /// Whether it asks for anything.
  bool get hasForm => schemaProperties(schema).isNotEmpty;
}

/// One catalog of a type as the host lists it.
final class CatalogRow {
  const CatalogRow({
    required this.label,
    required this.spec,
    this.isDefault = false,
    this.fixed = false,
    this.problem,
    this.origin = 'added',
    this.enabled = true,
    this.pin,
    this.ttl,
    this.trust,
    this.fetched,
    this.from,
    this.hasSnapshot = false,
  });

  final String label;
  final CatalogSpec? spec;

  /// Whether it is the one the type offers until it is removed.
  final bool isDefault;

  /// Whether the type always offers it: it cannot be removed or replaced.
  final bool fixed;

  /// Why the host reads no catalog from the entry.
  final String? problem;

  /// Where it comes from: `default` (the type's own), `replaced` (canoa.cfg's entry took a default's place) or `added`.
  final String origin;

  /// False for a default canoa.cfg turned off with `label = false`; it stays listed so it can be turned on again.
  final bool enabled;

  /// What fixes its answer (a revision, or `sha256:` and a digest), when the entry or the default says one.
  final String? pin;

  /// Seconds before it is read again, when the entry says.
  final int? ttl;

  /// `pinned` when its answer is fixed to a revision; null otherwise, and for a type the host reads itself.
  final String? trust;

  /// When it was last read, in milliseconds since the epoch (0 for a shipped copy), and where from: `cache` or `snapshot`.
  final int? fetched;
  final String? from;

  /// Whether the extension ships a copy of it, which a pinned catalog answers with before it has been fetched.
  final bool hasSnapshot;

  /// What to show of where it comes from.
  String get shown => spec == null ? '' : (spec!.source ?? '${spec!.provider}: ${spec!.fields.values.map((v) => v is List ? v.join(' ') : '$v').join(', ')}');
}

/// One Store type that reads catalogs.
final class CatalogType {
  const CatalogType({
    required this.id,
    required this.title,
    required this.words,
    this.defaultLabel,
    this.defaultSpec,
    this.skillsSearchDefault,
    this.format,
    this.rows = const [],
    this.sources = const [],
  });

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
      defaultSpec: def is Map<Object?, Object?> ? def['source'] as String? : null,
      skillsSearchDefault: each['skills_search'] as String?,
      format: each['format'] as String?,
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
              origin: row['origin'] is String ? row['origin'] as String : (row['default'] == true ? 'default' : 'added'),
              enabled: row['enabled'] != false,
              pin: row['pin'] as String?,
              ttl: (row['ttl'] as num?)?.toInt(),
              trust: row['trust'] as String?,
              fetched: (row['fetched'] as num?)?.toInt(),
              from: row['from'] as String?,
              hasSnapshot: row['snapshot'] == true,
            ),
      ],
      sources: [for (final source in each['sources'] as List<Object?>? ?? const []) if (source is Map<Object?, Object?>) CatalogSourceForm.fromAnswer(source)],
    );
  }

  final String id, title;
  final CatalogWords words;

  /// The label and source of the catalog the type offers until the person removes it.
  final String? defaultLabel;
  final String? defaultSpec;

  /// The directory address the host uses when `skills_search = true`.
  final String? skillsSearchDefault;

  /// The format its catalogs answer in (`canoa.models/1`, `canoa.releases/1`); null for a type the host reads itself.
  final String? format;
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
