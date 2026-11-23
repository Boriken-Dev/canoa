/// The catalogs a surface may name: A2UI's basic catalog, Canoa's own (the basic one and a few components for data), and the
/// catalogs an overlay ships, each of which only renames and presets components Canoa already draws. A catalog is data: no
/// catalog brings code, so the widgets are always Canoa's own, in the colours of the look.
library;

import 'messages.dart';

/// The basic catalog of A2UI 1.0.
const String basicCatalogId = 'https://a2ui.org/specification/v1_0/catalogs/basic/catalog.json';

/// Canoa's catalog: the basic components and [canoaOnly].
const String canoaCatalogId = 'urn:dev.boriken.canoa:a2ui:catalog:1';

/// What a component of a catalog is: the widget that draws it, the properties it cannot be without, what it starts with, and which
/// components it may sit in or hold (null for any).
final class A2uiComponentSpec {
  const A2uiComponentSpec(this.native, {this.required = const [], this.defaults = const {}, this.allowedParents, this.allowedChildren});
  final String native;
  final List<String> required;
  final Map<String, Object?> defaults;
  final List<String>? allowedParents, allowedChildren;
}

const Map<String, List<String>> _basic = {
  'Text': ['text'],
  'Image': ['url'],
  'Icon': ['name'],
  'Row': ['children'],
  'Column': ['children'],
  'List': ['children'],
  'Card': ['child'],
  'Tabs': ['tabs'],
  'Modal': ['trigger', 'content'],
  'Divider': [],
  'Button': ['child', 'action'],
  'TextField': ['label', 'value'],
  'CheckBox': ['label', 'value'],
};

/// The components only Canoa's catalog has.
const Map<String, List<String>> canoaOnly = {
  'Badge': ['text'],
  'Metric': ['label', 'value'],
  'Table': ['columns', 'rows'],
  'CodeBlock': ['text'],
  'Progress': ['value'],
};

/// One catalog: its id and its components by name.
final class A2uiCatalog {
  const A2uiCatalog(this.id, this.components);

  final String id;
  final Map<String, A2uiComponentSpec> components;

  A2uiComponentSpec? operator [](String name) => components[name];

  static final A2uiCatalog basic = A2uiCatalog(basicCatalogId, {
    for (final entry in _basic.entries) entry.key: A2uiComponentSpec(entry.key, required: entry.value),
  });

  static final A2uiCatalog canoa = A2uiCatalog(canoaCatalogId, {
    ...basic.components,
    for (final entry in canoaOnly.entries) entry.key: A2uiComponentSpec(entry.key, required: entry.value),
  });

  /// The catalog an overlay's document describes, with what is wrong with it: its `catalogId` (its own, not another catalog's), and
  /// `components`, each `{"extends": NAME, "defaults": {...}, "allowedParents": [...], "allowedChildren": [...]}` over a component
  /// Canoa's catalog has. A catalog with any problem is refused whole.
  static ({A2uiCatalog? catalog, List<String> problems}) fromOverlay(Object? document) {
    final problems = <String>[];
    if (document is! Map) return (catalog: null, problems: ['A catalog is an object']);
    final id = document['catalogId'];
    if (id is! String || id.isEmpty) problems.add('catalogId names the catalog');
    if (id == basicCatalogId || id == canoaCatalogId) problems.add('catalogId is another catalog\'s: an overlay names its own');
    final declared = document['components'];
    if (declared is! Map) problems.add('components is an object');
    final made = <String, A2uiComponentSpec>{};
    if (declared is Map) {
      for (final entry in declared.entries) {
        final name = '${entry.key}';
        final value = entry.value;
        if (canoa.components.containsKey(name)) {
          problems.add('$name is a component Canoa\'s catalog has already');
          continue;
        }
        if (value is! Map || value['extends'] is! String || canoa.components[value['extends']] == null) {
          problems.add('$name extends a component Canoa draws (${canoa.components.keys.join(', ')})');
          continue;
        }
        final base = canoa.components[value['extends']]!;
        List<String>? names(Object? list) => list is List ? [for (final each in list) '$each'] : null;
        final defaults = value['defaults'];
        if (defaults != null && defaults is! Map) {
          problems.add('$name: defaults is an object');
          continue;
        }
        made[name] = A2uiComponentSpec(
          base.native,
          required: [
            for (final key in base.required)
              if (defaults is! Map || !defaults.containsKey(key)) key,
          ],
          defaults: {if (defaults is Map) for (final each in defaults.entries) '${each.key}': each.value},
          allowedParents: names(value['allowedParents']),
          allowedChildren: names(value['allowedChildren']),
        );
      }
    }
    if (problems.isNotEmpty) return (catalog: null, problems: problems);
    return (catalog: A2uiCatalog(id! as String, {...canoa.components, ...made}), problems: const []);
  }

  /// What is wrong with [components] against this catalog: an unknown component, a missing property, a parent or a child the
  /// component's spec does not allow. Each is a problem with the path of the component in the update.
  List<A2uiProblem> check(String surfaceId, Map<String, Map<String, Object?>> nodes) {
    final out = <A2uiProblem>[];
    for (final entry in nodes.entries) {
      final node = entry.value;
      final name = '${node['component']}';
      final spec = components[name];
      final path = '/components/${entry.key}';
      if (spec == null) {
        out.add(A2uiProblem(A2uiProblem.validationFailed, '"$name" is not a component of this catalog', surfaceId: surfaceId, path: path));
        continue;
      }
      for (final key in spec.required) {
        if (!node.containsKey(key) && !spec.defaults.containsKey(key)) {
          out.add(A2uiProblem(A2uiProblem.validationFailed, '$name needs "$key"', surfaceId: surfaceId, path: '$path/$key'));
        }
      }
      final merged = {...spec.defaults, ...node};
      for (final child in childIds(merged)) {
        final held = nodes[child];
        if (held == null) continue;
        final childName = '${held['component']}';
        final childSpec = components[childName];
        if (spec.allowedChildren != null && !spec.allowedChildren!.contains(childName)) {
          out.add(A2uiProblem(A2uiProblem.unallowedChild, '$name does not hold a $childName', surfaceId: surfaceId, path: path));
        }
        if (childSpec?.allowedParents != null && !childSpec!.allowedParents!.contains(name)) {
          out.add(A2uiProblem(A2uiProblem.unallowedParent, '$childName does not sit in a $name', surfaceId: surfaceId, path: '/components/$child'));
        }
      }
    }
    return out;
  }
}

/// The ids of the components [node] holds: `children` (a list), `child`, `trigger`, `content`, and each tab's `child`. A template's
/// component is named too, since it is drawn in the node's place.
List<String> childIds(Map<String, Object?> node) {
  final out = <String>[];
  final children = node['children'];
  if (children is List) out.addAll([for (final each in children) if (each is String) each]);
  if (children is Map && children['componentId'] is String) out.add(children['componentId'] as String);
  for (final key in ['child', 'trigger', 'content']) {
    if (node[key] is String) out.add(node[key] as String);
  }
  final tabs = node['tabs'];
  if (tabs is List) {
    for (final tab in tabs) {
      if (tab is Map && tab['child'] is String) out.add(tab['child'] as String);
    }
  }
  return out;
}

/// The catalogs a window can draw: Canoa's two, and each an overlay brought. An id that is not here is refused, never guessed.
final class A2uiCatalogs {
  A2uiCatalogs([Iterable<A2uiCatalog> more = const []]) {
    for (final each in [A2uiCatalog.basic, A2uiCatalog.canoa, ...more]) {
      _byId[each.id] = each;
    }
  }

  final Map<String, A2uiCatalog> _byId = {};

  A2uiCatalog? operator [](String id) => _byId[id];

  /// The catalog a surface that names none is drawn with.
  A2uiCatalog get fallback => A2uiCatalog.canoa;

  /// The ids this window says it supports.
  List<String> get ids => [..._byId.keys];

  void register(A2uiCatalog catalog) => _byId[catalog.id] = catalog;
}
