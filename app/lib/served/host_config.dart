import 'dart:convert';

import '../config/canoa_config.dart';
import '../config/config_layers.dart' show decodeConfig;
import 'host_files.dart';

/// The tables of canoa.cfg a page may read and write on its host, as the host names them (`control::SECTIONS`): the app's own
/// settings. `host`, `agents`, `profile`, `desktop` and `skill_pins` name programs to run or what may run, so they stay the
/// person's to edit in the file; a change to one of them in a configuration saved here is left unsaved.
const hostConfigSections = [
  'appearance',
  'assets',
  'attachments',
  'catalogs',
  'defaults',
  'loadouts',
  'network',
  'notifications',
  'overlays',
  'permissions',
  'sessions',
  'store',
  'usage',
];

/// The person's own canoa.cfg on a host reached over the control channel, as the app's [CanoaConfig] and written back by table:
/// only the tables that changed are sent, each as the host's own words allow.
final class HostConfigStore {
  HostConfigStore(this._files);

  final HostFiles _files;

  /// Each table as this page's own encoding of the configuration it read spells it, by name, as JSON text; absent for a table the
  /// host has none of. Compared with the same spelling at [save], so only a table the person changed is sent, and a table the host
  /// spells another way is left as it is.
  final _held = <String, String?>{};
  bool _loaded = false;

  /// Whether canoa.cfg was read whole: [save] refuses until it has been.
  bool get loaded => _loaded;

  static String? _textOf(Object? table) => table is Map && table.isNotEmpty ? jsonEncode(table) : null;

  static Map<String, Object?> _tablesOf(CanoaConfig config) {
    final text = config.encode();
    return text.trim().isEmpty ? const {} : decodeConfig(text, name: 'canoa.json').data;
  }

  /// canoa.cfg as the host holds it. A table this page cannot read (a key it does not know) fails the whole read, and nothing is held:
  /// a configuration read in part would write the rest over the host's file.
  Future<CanoaConfig> load() async {
    _loaded = false;
    final tables = <String, Object?>{};
    for (final section in hostConfigSections) {
      final value = await _files.configGet(section);
      if (value != null) tables[section] = value;
    }
    final config = CanoaConfig.parse(jsonEncode(tables), name: 'canoa.json');
    final data = _tablesOf(config);
    _held
      ..clear()
      ..addEntries([for (final section in hostConfigSections) MapEntry(section, _textOf(data[section]))]);
    _loaded = true;
    return config;
  }

  /// Writes the tables of [config] that differ from what was read, and says whether the host reads one of them only when it
  /// starts, so that [HostFiles.restart] is what applies the change. A table is removed on the host only when the host had one and
  /// [config] has none now. Throws [StateError] when canoa.cfg was not read whole.
  Future<bool> save(CanoaConfig config) async {
    if (!_loaded) throw StateError('canoa.cfg on the host was not read, so nothing is written to it');
    final data = _tablesOf(config);
    var restart = false;
    for (final section in hostConfigSections) {
      final text = _textOf(data[section]);
      if (text == _held[section]) continue;
      restart |= await _files.configSet(section, text == null ? null : (jsonDecode(text) as Map<String, Object?>));
      _held[section] = text;
    }
    return restart;
  }
}
