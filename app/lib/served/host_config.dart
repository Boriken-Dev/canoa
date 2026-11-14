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

  /// Each table as the host last held it, by name, as JSON text; absent for a table it has none of.
  final _held = <String, String?>{};

  /// canoa.cfg as the host holds it.
  Future<CanoaConfig> load() async {
    final tables = <String, Object?>{};
    for (final section in hostConfigSections) {
      final value = await _files.configGet(section);
      _held[section] = value == null ? null : jsonEncode(value);
      if (value != null) tables[section] = value;
    }
    return CanoaConfig.parse(jsonEncode(tables), name: 'canoa.json');
  }

  /// Writes the tables of [config] that differ from what the host held, and says whether the host reads one of them only when it
  /// starts, so that [HostFiles.restart] is what applies the change.
  Future<bool> save(CanoaConfig config) async {
    final data = decodeConfig(config.encode(), name: 'canoa.json').data;
    var restart = false;
    for (final section in hostConfigSections) {
      final now = data[section];
      final text = now is Map && now.isNotEmpty ? jsonEncode(now) : null;
      if (text == _held[section]) continue;
      restart |= await _files.configSet(section, text == null ? null : (jsonDecode(text) as Map<String, Object?>));
      _held[section] = text;
    }
    return restart;
  }
}
