import 'dart:async';

import 'package:flutter/foundation.dart';

import '../config/canoa_config.dart';
import 'setting_layers.dart';

/// Takes a changed configuration, applies and writes it, and gives back the one the app now holds: the same unless part of
/// the change was refused, such as sources a restart was declined for.
typedef ApplyConfig = Future<CanoaConfig> Function(CanoaConfig changed);

/// The configuration Settings shows and edits. Each change is applied as it is made: there is nothing to save.
///
/// A change is an edit of the newest configuration, never a replacement of an older copy, so two quick changes (a switch and
/// a field) cannot undo each other, and changes are applied one after another.
class SettingsController extends ChangeNotifier {
  SettingsController({required CanoaConfig config, required this.apply}) : _config = config;

  final ApplyConfig apply;
  CanoaConfig _config;
  Future<void> _last = Future.value();
  String? _problem;
  bool _closed = false;

  CanoaConfig get config => _config;

  /// Which layer the value at [path] comes from when it is not the person's own, with a way back to it where the person's
  /// file overrides one.
  SettingLayer? layerOf(List<String> path) =>
      settingLayer(_config, path, onReset: () => change((c) => c.resetKey(path)));

  /// Why the last change could not be written, or null.
  String? get problem => _problem;

  /// Edits the configuration: the value shows at once, and is then applied.
  Future<void> change(CanoaConfig Function(CanoaConfig) edit) {
    _config = edit(_config);
    _problem = null;
    notifyListeners();
    final changed = _config;
    return _last = _last.then((_) async {
      try {
        final held = await apply(changed);
        // The app may have kept something back (a refused restart): show what it holds, unless a newer edit is waiting.
        if (!_closed && identical(_config, changed) && !identical(held, changed)) {
          _config = held;
          notifyListeners();
        }
      } on Object catch (error) {
        _problem = 'Could not save canoa.cfg: $error';
        if (!_closed) notifyListeners();
      }
    });
  }

  /// Takes a configuration changed elsewhere while Settings is open (Hosting, Loadout), without applying it.
  void adopt(CanoaConfig other) {
    _config = other;
    notifyListeners();
  }

  @override
  void dispose() {
    _closed = true;
    super.dispose();
  }
}
