import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show FilteringTextInputFormatter;

import '../config/canoa_config.dart';
import '../store/store_settings.dart';
import '../theme/look.dart';
import 'settings_controller.dart';
import 'settings_row.dart';

/// The catalogs the store lists, and how much of a catalog and of an icon it reads.
class CatalogsSection extends StatefulWidget {
  const CatalogsSection({super.key, required this.controller});

  final SettingsController controller;

  @override
  State<CatalogsSection> createState() => _CatalogsSectionState();
}

class _CatalogsSectionState extends State<CatalogsSection> {
  final _label = TextEditingController();
  final _spec = TextEditingController();
  String? _problem;

  @override
  void dispose() {
    _label.dispose();
    _spec.dispose();
    super.dispose();
  }

  void _add() {
    final label = _label.text.trim();
    final spec = _spec.text.trim();
    try {
      if (!CanoaConfig.isCatalogLabel(label)) {
        throw const FormatException('A label is a letter, then letters, digits, - or _, at most 64.');
      }
      widget.controller.config.withCatalog(label, spec);
    } on FormatException catch (error) {
      setState(() => _problem = error.message.replaceFirst('canoa.cfg: ', ''));
      return;
    }
    setState(() => _problem = null);
    _label.clear();
    _spec.clear();
    unawaited(widget.controller.change((c) => c.withCatalog(label, spec)));
  }

  static const _limits = [
    ('catalog_max_bytes', 'Largest catalog', 'Bytes read from one catalog file'),
    ('catalog_max_depth', 'Deepest nesting', 'Levels of nesting in a catalog'),
    ('catalog_max_entries', 'Most entries', 'Entries one catalog may list'),
    ('catalog_max_icons', 'Most icons', 'Distinct icon files read for one catalog'),
    ('icon_max_bytes', 'Largest icon', 'Bytes of one icon file'),
    ('icon_max_width', 'Widest icon', 'Pixels'),
    ('icon_max_height', 'Tallest icon', 'Pixels'),
  ];

  int _value(StoreSettings s, String key) => switch (key) {
    'catalog_max_bytes' => s.catalogMaxBytes,
    'catalog_max_depth' => s.catalogMaxDepth,
    'catalog_max_entries' => s.catalogMaxEntries,
    'catalog_max_icons' => s.catalogMaxIcons,
    'icon_max_bytes' => s.iconMaxBytes,
    'icon_max_width' => s.iconMaxWidth,
    _ => s.iconMaxHeight,
  };

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final c = widget.controller;
      final catalogs = c.config.catalogs;
      final store = c.config.store;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        SettingsGroup(title: 'Catalogs', children: [
          if (catalogs.isEmpty) const SettingsRow(title: 'No catalog', description: 'Loadout lists what the catalogs offer; add one below'),
          for (final MapEntry(key: label, value: source) in catalogs.entries)
            SettingsRow(
              key: Key('catalog-$label'),
              title: label,
              configKey: ['catalogs', label],
              description: source.spec,
              control: IconButton(
                key: Key('catalog-remove-$label'),
                tooltip: 'Remove $label',
                icon: const Icon(Icons.delete_outline),
                onPressed: () => c.change((x) => x.withoutCatalog(label)),
              ),
            ),
        ]),
        SettingsGroup(title: 'Add a catalog', children: [
          SettingsRow(
            title: 'Label',
            below: TextField(key: const Key('catalog-label'), controller: _label, decoration: const InputDecoration(isDense: true, border: OutlineInputBorder())),
          ),
          SettingsRow(
            title: 'Source',
            description: '<repository>@<ref>#<catalog file>',
            below: TextField(
              key: const Key('catalog-spec'),
              controller: _spec,
              decoration: InputDecoration(isDense: true, border: const OutlineInputBorder(), errorText: _problem, errorMaxLines: 3),
              onSubmitted: (_) => _add(),
            ),
          ),
          SettingsRow(title: 'Add it', control: FilledButton(key: const Key('catalog-add'), onPressed: _add, child: const Text('Add'))),
        ]),
        SettingsGroup(title: 'Limits', children: [
          for (final (key, title, description) in _limits)
            SettingsRow(
              title: title,
              configKey: ['store', key],
              description: '$description. Default ${_value(const StoreSettings(), key)}',
              control: _LimitField(
                key: Key('limit-$key'),
                value: _value(store, key),
                onChanged: (n) => c.change((x) => x.withStore(x.store.withLimit(key, n))),
              ),
            ),
        ]),
      ]);
    },
  );
}

/// A whole number, 0 or more, applied as it is typed once it reads as one.
class _LimitField extends StatefulWidget {
  const _LimitField({super.key, required this.value, required this.onChanged});

  final int value;
  final ValueChanged<int> onChanged;

  @override
  State<_LimitField> createState() => _LimitFieldState();
}

class _LimitFieldState extends State<_LimitField> {
  late final _text = TextEditingController(text: '${widget.value}');

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SizedBox(
    width: 140,
    child: TextField(
      controller: _text,
      keyboardType: TextInputType.number,
      inputFormatters: [FilteringTextInputFormatter.digitsOnly],
      textAlign: TextAlign.end,
      style: const TextStyle(fontFamily: CanoaLook.monospaceFont),
      decoration: const InputDecoration(isDense: true, border: OutlineInputBorder()),
      onChanged: (text) {
        final n = int.tryParse(text);
        if (n != null) widget.onChanged(n);
      },
    ),
  );
}

/// What this build is.
class AboutSection extends StatelessWidget {
  const AboutSection({super.key, this.version = ''});

  final String version;

  @override
  Widget build(BuildContext context) => SettingsGroup(children: [
    SettingsRow(
      title: 'Canoa',
      description: 'A client for agents and the tools they use',
      control: Text(version.isEmpty ? 'development build' : version, key: const Key('about-version')),
    ),
    const SettingsRow(title: 'Publisher', description: 'Boriken Dev, boriken.dev'),
  ]);
}
