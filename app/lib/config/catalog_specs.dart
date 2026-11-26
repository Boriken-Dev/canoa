/// One shape for the catalogs of every Store type in canoa.cfg: `[store.<type>.catalogs.<label>]`. A label's value is a source (a
/// bare string: a URI or a path), or a table with a `source` and the `generator` that makes the catalog from it and what the
/// generator's form asked for, or a table naming a `provider` of an extension that has not moved to a generator, or `false`, which
/// takes a default catalog away. Which generators and providers exist, and what each one's form asks for, is the host's to say.
library;

/// The provider that reads the place a Store type reads itself (a git source, an address, a folder): the one a label that names
/// none has.
const readProvider = 'read';

/// One catalog of a Store type, as canoa.cfg has it.
final class CatalogSpec {
  const CatalogSpec(this.provider, [this.fields = const {}]) : generator = null;

  /// A catalog the `catalog_generator` [generator] makes from [fields], whose `source` is the URI it reads.
  const CatalogSpec.generated(this.generator, [this.fields = const {}]) : provider = readProvider;

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
        final generator = table['generator'];
        if (generator != null && (generator is! String || generator.isEmpty)) throw FormatException('canoa.cfg: $where.generator is the id of a catalog generator');
        if (generator != null && provider != readProvider) throw FormatException('canoa.cfg: $where names a generator or a provider, not both');
        final fields = {
          for (final MapEntry(:key, :value) in table.entries)
            if (key != 'provider' && key != 'generator') '$key': value,
        };
        return generator is String ? CatalogSpec.generated(generator, fields) : CatalogSpec(provider, fields);
      default:
        throw FormatException('canoa.cfg: $where is a source, a table naming a provider and its fields, or false');
    }
  }

  /// The id of the `catalog_source` provider that reads it.
  final String provider;

  /// The id of the `catalog_generator` that makes it from its source, or null.
  final String? generator;

  /// What the form holds, by field name, `source` among them.
  final Map<String, Object?> fields;

  /// Whether the source is the catalog itself: neither a generator nor a provider makes it from something.
  bool get isDocument => provider == readProvider && generator == null;

  /// The `source` field, a URI or a path, or null.
  String? get source => provider == readProvider && fields['source'] is String ? fields['source']! as String : null;

  /// What canoa.cfg holds: the bare source for a document with nothing else, else a table.
  Object toRaw() => generator != null
      ? {'generator': generator, ...fields}
      : isDocument && fields.length == 1 && source != null
      ? source!
      : {'provider': provider, ...fields};

  @override
  bool operator ==(Object other) => other is CatalogSpec && other.provider == provider && other.generator == generator && _same(other.fields, fields);

  @override
  int get hashCode => Object.hash(provider, generator, fields.length);

  static bool _same(Object? a, Object? b) {
    if (a is Map && b is Map) return a.length == b.length && a.keys.every((key) => b.containsKey(key) && _same(a[key], b[key]));
    if (a is List && b is List) return a.length == b.length && [for (var i = 0; i < a.length; i++) _same(a[i], b[i])].every((same) => same);
    return a == b;
  }

  @override
  String toString() => generator != null ? '${source ?? ''} by $generator' : source ?? '$provider $fields';
}
