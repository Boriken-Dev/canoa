import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart';
import '../forms/schema_fields.dart' show SecretSource;
import 'secrets_host.dart';

/// The secrets table's state: what the host holds, which value the person asked to see, and what is running. A value is held here only
/// while it is shown, and goes when it is hidden, forgotten or the row is gone.
class SecretsController extends ChangeNotifier {
  SecretsController({required this.host});

  final SecretsHost host;

  List<SecretRow> rows = const [];
  List<SecretKeep> keeps = const [];
  List<SecretEntry> entries = const [];
  List<SecretProviderInfo> providers = const [];
  bool loading = false;

  /// The vault (a provider entry's name) the page shows; null shows all of them.
  String? vault;

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
      providers = await host.providers();
      rows = listing.rows;
      keeps = listing.keeps;
      this.entries = entries;
      if (vault != null && !entries.any((entry) => entry.name == vault)) vault = null;
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

  /// Shows the vault [name], or all of them when it is null.
  void select(String? name) {
    if (vault == name) return;
    vault = name;
    notifyListeners();
  }

  /// The entry of the vault the page shows, or null when it shows all of them.
  SecretEntry? get chosen => vault == null ? null : entries.where((entry) => entry.name == vault).firstOrNull;

  /// The secrets of the vault the page shows, or every secret.
  List<SecretRow> get visible => vault == null ? rows : [for (final row in rows) if (row.provider == vault) row];

  /// How many secrets the vault [name] holds.
  int countIn(String name) => rows.where((row) => row.provider == name).length;

  /// The provider the vault [entry] is an entry of, as the host describes it.
  SecretProviderInfo? infoOf(SecretEntry entry) => providers.where((each) => each.id == entry.provider).firstOrNull;

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

  /// Sets up the entry [name] of [provider] and reads the list again. The host's reason when it will not, or null when it did.
  Future<String?> configure({required String name, required String provider, Map<String, Object?> settings = const {}, Map<String, SecretSource> from = const {}}) async {
    try {
      await host.configure(name: name, provider: provider, settings: settings, from: from);
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return '$error';
    }
    await refresh();
    return null;
  }

  /// Takes the entry [name] away, and reads the list again.
  Future<void> removeEntry(String name) async {
    try {
      await host.removeEntry(name);
      problem = null;
    } on ControlFailed catch (error) {
      problem = error.message;
    } on Object catch (error) {
      problem = '$error';
    }
    await refresh();
  }

  /// The entries a value can be set in by hand.
  List<SecretEntry> get settable => [for (final entry in entries) if (entry.acceptsValues) entry];
}
