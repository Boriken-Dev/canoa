/// One shape for the catalogs of every Store type in canoa.cfg: `[store.<type>.catalogs.<label>]`. A label's value is a source the
/// `read` provider reads (a bare string), or a table naming a provider and what its form asked for, or `false`, which takes a
/// default catalog away. Which providers exist, and what each one's form asks for, is the host's to say.
library;

/// The provider that reads the place a Store type reads itself (a git source, an address, a folder): the one a label that names
/// none has.
const readProvider = 'read';

/// One catalog of a Store type, as canoa.cfg has it.
final class CatalogSpec {
  const CatalogSpec(this.provider, [this.fields = const {}]);

  /// A source the `read` provider reads.
  CatalogSpec.read(String source) : this(readProvider, {'source': source});

  /// Reads one label's value. Throws [FormatException] saying what is wrong, naming [where].
  factory CatalogSpec.parse(String where, Object? raw) {
    switch (raw) {
      case final String source when source.isNotEmpty:
        return CatalogSpec.read(source);
      case final Map<dynamic, dynamic> table:
        final provider = table['provider'] ?? readProvider;
        if (provider is! String || provider.isEmpty) throw FormatException('canoa.cfg: $where.provider is the id of a catalog source');
        return CatalogSpec(provider, {
          for (final MapEntry(:key, :value) in table.entries)
            if (key != 'provider') '$key': value,
        });
      default:
        throw FormatException('canoa.cfg: $where is a source, a table naming a provider and its fields, or false');
    }
  }

  /// The id of the `catalog_source` provider that reads it.
  final String provider;

  /// What the provider's form holds, by field name.
  final Map<String, Object?> fields;

  /// The `source` field of a catalog the `read` provider reads, or null.
  String? get source => provider == readProvider && fields['source'] is String ? fields['source']! as String : null;

  /// What canoa.cfg holds: the bare source for `read` with nothing else, else a table.
  Object toRaw() => provider == readProvider && fields.length == 1 && source != null ? source! : {'provider': provider, ...fields};

  @override
  bool operator ==(Object other) => other is CatalogSpec && other.provider == provider && _same(other.fields, fields);

  @override
  int get hashCode => Object.hash(provider, fields.length);

  static bool _same(Object? a, Object? b) {
    if (a is Map && b is Map) return a.length == b.length && a.keys.every((key) => b.containsKey(key) && _same(a[key], b[key]));
    if (a is List && b is List) return a.length == b.length && [for (var i = 0; i < a.length; i++) _same(a[i], b[i])].every((same) => same);
    return a == b;
  }

  @override
  String toString() => source ?? '$provider $fields';
}
