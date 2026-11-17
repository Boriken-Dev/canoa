import 'dart:convert';

import '../config/canoa_config.dart';
import '../config/config_layers.dart' show decodeConfig;
import '../desktop/control_client.dart' show ControlFailed;
import 'host_files.dart';

/// The tables of canoa.cfg a page may read and write on its host, as the host names them (`control::SECTIONS`): the app's own
/// settings. `host`, `agents`, `profile`, `desktop` and `skill_pins` name programs to run or what may run, so they stay the
/// person's to edit in the file; a change to one of them in a configuration saved here is left unsaved.
const hostConfigSections = [
  'appearance',
  'assets',
  'attachments',
  'catalogs',
  'connections',
  'defaults',
  'loadouts',
  'network',
  'notifications',
  'overlays',
  'permissions',
  'profiles',
  'sessions',
  'store',
  'usage',
];

/// The tables only the app the host runs in reads and writes: the commands, the window and the profile it holds, and what pins a
/// skill. No paired key edits them.
const ownerSections = ['agents', 'desktop', 'profile', 'skill_pins', 'voice'];

/// The host's own table, which only a key that holds `hosting` reads or writes: the agents it serves, who may connect and the
/// knowledge settings.
const hostTable = 'host';

/// What [HostConfigStore.save] throws when the host's `[host]` table is no longer the one this page read, as another page or key
/// wrote it meanwhile: nothing was written, and [config] is canoa.cfg read again.
final class HostConfigChanged implements Exception {
  const HostConfigChanged(this.config);

  final CanoaConfig config;

  String get message =>
      "The host's own settings changed since this page read them, so nothing was saved. They were read again: make the change again.";

  @override
  String toString() => message;
}

/// The person's own canoa.cfg on a host reached over the control channel, as the app's [CanoaConfig] and written back by table:
/// only the tables that changed are sent, each as the host's own words allow.
final class HostConfigStore {
  HostConfigStore(this._files) : _desktop = false;

  /// The store of the app that runs the host: it holds [config] as the file holds it, so only a table it changes later is written,
  /// and it may write every table of canoa.cfg the app edits, the ones only that app reaches included.
  HostConfigStore.ofDesktop(this._files, CanoaConfig config) : _desktop = true {
    adopt(config);
  }

  final HostFiles _files;
  final bool _desktop;

  /// Each table as this page's own encoding of the configuration it read spells it, by name, as JSON text; absent for a table the
  /// host has none of. Compared with the same spelling at [save], so only a table the person changed is sent, and a table the host
  /// spells another way is left as it is.
  final _held = <String, String?>{};
  bool _loaded = false;
  bool _withHost = false;

  /// What the host named `[host]` by when it was read; sent back with a write of it.
  String? _hostDigest;

  /// Whether canoa.cfg was read whole: [save] refuses until it has been.
  bool get loaded => _loaded;

  /// Whether `[host]` was read too, which the host gives a key that holds `hosting`: only then is it written back.
  bool get includesHost => _withHost;

  List<String> get _sections => [...hostConfigSections, if (_desktop) ...ownerSections, if (_withHost) hostTable];

  /// Takes [config] as what the file holds now, so that the next [save] writes only what differs from it. For the app that runs the
  /// host, which wrote the file itself since it last read it.
  void adopt(CanoaConfig config) {
    _withHost = _desktop;
    final data = _tablesOf(config);
    _held
      ..clear()
      ..addEntries([for (final section in _sections) MapEntry(section, _textOf(data[section]))]);
    _loaded = true;
  }

  /// What is wrong with the `[host]` table of [config], in the host's words, or null when the host accepts it. Nothing is written.
  Future<String?> checkHost(CanoaConfig config) async {
    if (!_withHost) return "this key may not change the host's own settings";
    final table = _tablesOf(config)[hostTable];
    return _files.configCheck(hostTable, table is Map ? {for (final MapEntry(:key, :value) in table.entries) '$key': value} : null);
  }

  static String? _textOf(Object? table) => table is Map && table.isNotEmpty ? jsonEncode(table) : null;

  static Map<String, Object?> _tablesOf(CanoaConfig config) {
    final text = config.encode();
    if (text.trim().isEmpty) return const {};
    // In the format the configuration was read in: a page's is JSON, a desktop's is its own file's.
    final source = config.source;
    return decodeConfig(text, name: source.marked ? null : 'canoa.${source.format.name}').data;
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
    _withHost = false;
    try {
      final host = await _files.configRead(hostTable);
      if (host.value != null) tables[hostTable] = host.value!;
      _hostDigest = host.digest;
      _withHost = true;
    } on ControlFailed catch (error) {
      // A key without the grant has no host table here, which is no failure; any other refusal is.
      if (error.code != -32001) rethrow;
    }
    final config = CanoaConfig.parse(jsonEncode(tables), name: 'canoa.json');
    final data = _tablesOf(config);
    _held
      ..clear()
      ..addEntries([for (final section in _sections) MapEntry(section, _textOf(data[section]))]);
    _loaded = true;
    return config;
  }

  /// Writes the tables of [config] that differ from what was read, and says whether the host reads one of them only when it
  /// starts, so that [HostFiles.restart] is what applies the change. A table is removed on the host only when the host had one and
  /// [config] has none now. A write of `[host]` names the table it was read as: [HostConfigChanged] is thrown when the host holds
  /// another by now. Throws [StateError] when canoa.cfg was not read whole.
  Future<bool> save(CanoaConfig config) async {
    if (!_loaded) throw StateError('canoa.cfg on the host was not read, so nothing is written to it');
    final data = _tablesOf(config);
    var restart = false;
    for (final section in _sections) {
      final text = _textOf(data[section]);
      if (text == _held[section]) continue;
      final value = text == null ? null : (jsonDecode(text) as Map<String, Object?>);
      if (section == hostTable) {
        try {
          final written = await _files.configWrite(section, value, digest: _hostDigest);
          restart |= written.restart;
          _hostDigest = written.digest ?? _hostDigest;
        } on ControlFailed catch (error) {
          if (error.code != hostChanged) rethrow;
          throw HostConfigChanged(await load());
        }
      } else {
        restart |= await _files.configSet(section, value);
      }
      _held[section] = text;
    }
    return restart;
  }
}
