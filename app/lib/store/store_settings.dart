import 'package:coqui_aos/coqui_aos.dart' as aos;

/// canoa.cfg `[store]`: how much of a catalog and of an icon the store reads. Each is a setting with the AOS
/// specification's recommended default and no floor; a value above the default is configuration the store lists.
final class StoreSettings {
  const StoreSettings({
    this.catalogMaxBytes = defaultCatalogMaxBytes,
    this.catalogMaxDepth = defaultCatalogMaxDepth,
    this.iconMaxBytes = defaultIconMaxBytes,
    this.iconMaxWidth = defaultIconMaxSide,
    this.iconMaxHeight = defaultIconMaxSide,
    this.catalogMaxEntries = defaultCatalogMaxEntries,
    this.catalogMaxIcons = defaultCatalogMaxIcons,
  });

  static const defaultCatalogMaxBytes = 4 * 1024 * 1024;
  static const defaultCatalogMaxDepth = 64;
  static const defaultIconMaxBytes = 512 * 1024;
  static const defaultIconMaxSide = 1024;
  static const defaultCatalogMaxEntries = 2000;
  static const defaultCatalogMaxIcons = 256;

  /// The keys `[store]` takes.
  static const keys = {'catalog_max_bytes', 'catalog_max_depth', 'catalog_max_entries', 'catalog_max_icons',
      'icon_max_bytes', 'icon_max_width', 'icon_max_height'};

  final int catalogMaxBytes, catalogMaxDepth, iconMaxBytes, iconMaxWidth, iconMaxHeight;

  /// The most entries one catalog lists, and the most distinct icon files read for one catalog.
  final int catalogMaxEntries, catalogMaxIcons;

  /// What a catalog is read within: its size and nesting.
  aos.Settings get catalogLimits => aos.defaultSettings.copyWith({'catalogBytes': catalogMaxBytes, 'manifestDepth': catalogMaxDepth});

  /// What an icon is checked against before it decodes.
  aos.Settings get iconLimits =>
      aos.defaultSettings.copyWith({'iconBytes': iconMaxBytes, 'iconWidth': iconMaxWidth, 'iconHeight': iconMaxHeight});

  /// Each limit set above its default, as `<key> = <value> (default <default>)`.
  List<String> get nonDefaults => [
    if (catalogMaxBytes > defaultCatalogMaxBytes) 'catalog_max_bytes = $catalogMaxBytes (default $defaultCatalogMaxBytes)',
    if (catalogMaxDepth > defaultCatalogMaxDepth) 'catalog_max_depth = $catalogMaxDepth (default $defaultCatalogMaxDepth)',
    if (catalogMaxEntries > defaultCatalogMaxEntries) 'catalog_max_entries = $catalogMaxEntries (default $defaultCatalogMaxEntries)',
    if (catalogMaxIcons > defaultCatalogMaxIcons) 'catalog_max_icons = $catalogMaxIcons (default $defaultCatalogMaxIcons)',
    if (iconMaxBytes > defaultIconMaxBytes) 'icon_max_bytes = $iconMaxBytes (default $defaultIconMaxBytes)',
    if (iconMaxWidth > defaultIconMaxSide) 'icon_max_width = $iconMaxWidth (default $defaultIconMaxSide)',
    if (iconMaxHeight > defaultIconMaxSide) 'icon_max_height = $iconMaxHeight (default $defaultIconMaxSide)',
  ];

  /// `[store]` as canoa.cfg has it. Throws [FormatException] for an unknown key or a value that is not a whole number,
  /// 0 or more.
  static StoreSettings parse(Map<dynamic, dynamic> table) {
    for (final key in table.keys) {
      if (!keys.contains(key)) throw FormatException('canoa.cfg: store: invalid key $key');
    }
    int read(String key, int fallback) {
      final value = table[key] ?? fallback;
      if (value is! int || value < 0) throw FormatException('canoa.cfg: store.$key must be a whole number, 0 or more');
      return value;
    }

    return StoreSettings(
      catalogMaxBytes: read('catalog_max_bytes', defaultCatalogMaxBytes),
      catalogMaxDepth: read('catalog_max_depth', defaultCatalogMaxDepth),
      iconMaxBytes: read('icon_max_bytes', defaultIconMaxBytes),
      iconMaxWidth: read('icon_max_width', defaultIconMaxSide),
      iconMaxHeight: read('icon_max_height', defaultIconMaxSide),
      catalogMaxEntries: read('catalog_max_entries', defaultCatalogMaxEntries),
      catalogMaxIcons: read('catalog_max_icons', defaultCatalogMaxIcons),
    );
  }
}
