import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart';
import 'secrets_host.dart';

/// The secrets table's state: what the host holds, which value the person asked to see, and what is running. A value is held here only
/// while it is shown, and goes when it is hidden, forgotten or the row is gone.
class SecretsController extends ChangeNotifier {
  SecretsController({required this.host});

  final SecretsHost host;

  List<SecretRow> rows = const [];
  List<SecretKeep> keeps = const [];
  List<SecretEntry> entries = const [];
  bool loading = false;

  /// Why the last call failed, in the host's words, which never hold a value.
  String? problem;
  final _shown = <String, String>{};

  /// What the person asked to see, by row key; the host's answer to `secrets/reveal`.
  String? shown(SecretRow row) => _shown[row.key];

  Future<void> refresh() async {
    loading = true;
    notifyListeners();
    try {
      final listing = await host.list();
      final entries = await host.entries();
      rows = listing.rows;
      keeps = listing.keeps;
      this.entries = entries;
      problem = null;
      _shown.removeWhere((key, _) => !rows.any((row) => row.key == key));
    } on ControlFailed catch (error) {
      problem = error.message;
    } on Object catch (error) {
      problem = '$error';
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  /// Asks the host for the value of [row], or hides it when it is shown.
  Future<void> toggle(SecretRow row) async {
    if (_shown.remove(row.key) != null) {
      notifyListeners();
      return;
    }
    try {
      _shown[row.key] = await host.reveal(row.provider, row.name);
      problem = null;
    } on ControlFailed catch (error) {
      problem = error.message;
    } on Object catch (error) {
      problem = '$error';
    }
    notifyListeners();
  }

  Future<void> forget(SecretRow row) async {
    try {
      await host.forget(row.provider, row.name);
      _shown.remove(row.key);
      problem = null;
    } on ControlFailed catch (error) {
      problem = error.message;
    } on Object catch (error) {
      problem = '$error';
    }
    await refresh();
  }

  /// Sets [name] in the entry [provider]: the host asks for the value, sealed. The table is read again when it ends.
  SecretSet set({required String provider, required String name}) {
    final run = host.set(provider: provider, name: name);
    void ended() {
      if (run.running) return;
      run.removeListener(ended);
      unawaited(refresh());
    }

    run.addListener(ended);
    return run;
  }

  /// The entries a value can be set in by hand.
  List<SecretEntry> get settable => [for (final entry in entries) if (entry.acceptsValues) entry];
}
