/// The store's catalogs as the host indexes them (`store/index`): one call to the host the app is connected to, on a desktop's
/// own host and on a served or portal host alike, and nothing fetched or parsed here.
library;

import 'dart:convert';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../profile/overlays.dart';
import '../profile/profile.dart';
import 'store.dart';

/// Reads every catalog the host lists; [refresh] makes the host ask its sources again before answering.
typedef CatalogIndex = Future<List<LoadedCatalog>> Function({bool refresh});

/// The host's `store/index`, asked over its control channel.
final class ControlStoreIndex {
  ControlStoreIndex(this._call);

  /// The index of the host [control] reaches, or null where the app has no host.
  static ControlStoreIndex? over(HostControl? control) => control == null
      ? null
      : ControlStoreIndex((method, params) async {
          final client = await control.ready();
          return client.call(method, params, null, const Duration(minutes: 6));
        });

  final Future<Object?> Function(String method, Map<String, Object?> params) _call;

  /// The catalogs, with each entry's icon. A refusal of the host is an [OverlayException].
  Future<List<LoadedCatalog>> catalogs({bool refresh = false}) async {
    try {
      return catalogsFromIndex(await _call('store/index', {'icons': true, if (refresh) 'refresh': true}));
    } on ControlFailed catch (error) {
      throw OverlayException(error.message);
    }
  }
}

/// The catalogs in the answer of `store/index`: the overlays and skills of each source, in the order the host gave them.
/// An entry the catalog or the host refused is listed as refused, with the host's reason; a source that failed is a catalog
/// with that problem, and keeps the entries the host still had of it.
List<LoadedCatalog> catalogsFromIndex(Object? answer) {
  if (answer is! Map<Object?, Object?>) return const [];
  final entries = [for (final each in answer['entries'] as List<Object?>? ?? const []) if (each is Map<Object?, Object?>) each];
  final found = <LoadedCatalog>[];
  for (final each in answer['sources'] as List<Object?>? ?? const []) {
    if (each is! Map<Object?, Object?> || each['kind'] != 'catalog') continue;
    final label = each['label'];
    final spec = each['spec'];
    if (label is! String || spec is! String) continue;
    final GitOverlay source;
    try {
      source = OverlaySource.parseGit(spec.startsWith('git+') ? spec.substring(4) : spec);
    } on ProfileException {
      continue;
    }
    final mine = [for (final entry in entries) if (entry['catalog'] == label) entry];
    final overlays = <String, CatalogEntry>{};
    final skills = <String, aos.CatalogEntry>{};
    final refused = <(String, String)>[];
    final icons = <String, CatalogIcon>{};
    for (final entry in mine) {
      final name = entry['name'];
      if (name is! String) continue;
      final origin = entry['origin'];
      if (origin is! Map<Object?, Object?>) {
        refused.add((name, '${entry['problem'] ?? 'refused'}'));
        continue;
      }
      final about = <String, Object>{
        if (entry['summary'] case final String summary) 'summary': summary,
        if (entry['tags'] case final List<Object?> tags when tags.isNotEmpty) 'tags': [for (final tag in tags) '$tag'],
        for (final field in const ['homepage', 'repository', 'license'])
          if (entry[field] case final String text) field: text,
        if (entry['authors'] case final List<Object?> authors when authors.isNotEmpty) 'authors': [for (final author in authors) '$author'],
      };
      final icon = entry['icon'];
      if (icon is Map<Object?, Object?> && icon['file'] is String) {
        about['icon'] = icon['file']! as String;
        icons[name] = icon['data'] is String
            ? (bytes: base64Decode(icon['data']! as String), refused: null)
            : (bytes: null, refused: 'icon-unavailable');
      }
      final from = aos.Source('${origin['kind']}', '${origin['location']}', origin['ref'] as String?, origin['path'] as String?);
      final pin = entry['pin'] as String?;
      if (entry['kind'] == 'skill') {
        skills[name] = aos.CatalogEntry(source: from, pin: pin, description: null, about: about.isEmpty ? null : about);
      } else {
        overlays[name] = CatalogEntry(name, from, pin: pin, about: about);
      }
    }
    final problem = each['problem'];
    final failed = problem is String && mine.isEmpty;
    final names = {for (final name in overlays.keys.toList()..sort()) name: overlays[name]!};
    found.add(LoadedCatalog(
      label: label,
      source: source,
      commit: each['commit'] is String && (each['commit']! as String).isNotEmpty ? each['commit']! as String : null,
      problem: failed ? problem : null,
      catalog: failed
          ? null
          : StoreCatalog(
              name: each['name'] as String?,
              description: each['description'] as String?,
              entries: names,
              skills: {for (final name in skills.keys.toList()..sort()) name: skills[name]!},
              refused: refused,
              ignored: const [],
            ),
      icons: icons,
    ));
  }
  return found;
}
