/// The state A2UI messages build: a surface (its components and its data model) and the processor that takes messages for a
/// session's surfaces, bounded by [A2uiLimits] against an agent that sends more than anyone could read.
library;

import 'dart:convert';

import 'package:flutter/foundation.dart';

import 'catalog.dart';
import 'limits.dart';
import 'messages.dart';

/// One surface: the flat map of its components by id (one is `root`), its data model, and what was cut or refused.
class A2uiSurface extends ChangeNotifier {
  A2uiSurface(this.id, this.catalog, this.limits, {this.sendDataModel = false});

  final String id;
  final A2uiCatalog catalog;
  final A2uiLimits limits;

  /// Whether the agent asked for the data model to go with each action, and it was allowed to.
  final bool sendDataModel;

  final Map<String, Map<String, Object?>> components = {};
  Object? _data = <String, Object?>{};

  /// What the surface's tree or data had to lose, one line each; the view says them above the surface.
  final List<String> cuts = [];

  /// What is wrong with the tree now, from the catalog's check.
  List<A2uiProblem> problems = const [];

  Object? get data => _data;

  /// Whether the tree can be drawn: its component `root` has arrived.
  bool get hasRoot => components.containsKey('root');

  void _cut(String line) {
    if (!cuts.contains(line)) cuts.add(line);
  }

  /// Takes [list] into the tree: each component by id, replacing one with the same id. Returns what is wrong with them.
  List<A2uiProblem> putComponents(List<Object?> list) {
    final out = <A2uiProblem>[];
    for (var i = 0; i < list.length; i++) {
      final raw = list[i];
      final path = '/updateComponents/components/$i';
      if (raw is! Map || raw['id'] is! String || (raw['id'] as String).isEmpty || raw['component'] is! String) {
        out.add(A2uiProblem(A2uiProblem.validationFailed, 'A component has an id and a component name', surfaceId: id, path: path));
        continue;
      }
      final node = {for (final entry in raw.entries) '${entry.key}': entry.value};
      final key = node['id']! as String;
      if (!components.containsKey(key) && components.length >= limits.maxNodes) {
        _cut('The surface has more than ${limits.maxNodes} components: the rest are not drawn.');
        continue;
      }
      components[key] = node;
    }
    problems = catalog.check(id, components);
    out.addAll(problems);
    notifyListeners();
    return out;
  }

  /// Sets or removes (a null [value]) what [path] names. A write that would pass the data limit, or name nothing writable, is refused.
  A2uiProblem? putData(String path, Object? value, {bool silent = false}) {
    final next = pointerWrite(_data, path, value, removed: value == null);
    if (next == null) {
      return A2uiProblem(A2uiProblem.validationFailed, 'The path "$path" cannot be written', surfaceId: id, path: path);
    }
    if (utf8.encode(jsonEncode(next)).length > limits.maxDataBytes) {
      _cut('The data model would pass ${limits.maxDataBytes} bytes: the update was not taken.');
      notifyListeners();
      return A2uiProblem(A2uiProblem.validationFailed, 'The data model holds at most ${limits.maxDataBytes} bytes', surfaceId: id, path: path);
    }
    _data = next;
    if (!silent) notifyListeners();
    return null;
  }

  /// The data model as the person's own edits left it.
  Map<String, Object?> get dataSnapshot => _data is Map ? {for (final entry in (_data! as Map).entries) '${entry.key}': entry.value} : const {};
}

/// Takes the messages of one session (or one overlay's view) and keeps its surfaces.
class A2uiProcessor extends ChangeNotifier {
  A2uiProcessor({this.limits = const A2uiLimits(), A2uiCatalogs? catalogs, DateTime Function()? clock, this.allowSendDataModel = false})
    : catalogs = catalogs ?? A2uiCatalogs(),
      _clock = clock ?? DateTime.now;

  final A2uiLimits limits;
  final A2uiCatalogs catalogs;
  final DateTime Function() _clock;

  /// Whether a surface that asks for its data model to go with its actions is believed. Off: what a person typed is not sent by
  /// an agent's say-so.
  final bool allowSendDataModel;

  final Map<String, A2uiSurface> surfaces = {};

  /// What was refused or wrong, newest last, kept to a few lines.
  final List<A2uiProblem> problems = [];

  /// Told each problem, to say it back to the agent.
  void Function(A2uiProblem problem)? onProblem;

  /// Told when a surface is made, deleted, or changed.
  void Function(String surfaceId, A2uiSurfaceChange change)? onChange;

  DateTime? _windowStart;
  var _inWindow = 0;
  var _dropped = false;

  void _problem(A2uiProblem problem) {
    problems.add(problem);
    if (problems.length > 20) problems.removeAt(0);
    onProblem?.call(problem);
  }

  /// Takes [messages] (decoded JSON), each on its own: one that is refused leaves the rest as they were.
  void receive(Iterable<Object?> messages) {
    var changed = false;
    for (final raw in messages) {
      final now = _clock();
      if (_windowStart == null || now.difference(_windowStart!) >= const Duration(seconds: 1)) {
        _windowStart = now;
        _inWindow = 0;
        _dropped = false;
      }
      if (++_inWindow > limits.maxUpdatesPerSecond) {
        if (!_dropped) {
          _dropped = true;
          _problem(A2uiProblem(A2uiProblem.validationFailed, 'More than ${limits.maxUpdatesPerSecond} messages in a second: the rest of it was dropped'));
        }
        continue;
      }
      final read = readA2uiMessage(raw, limits);
      if (read.problem != null) {
        _problem(read.problem!);
        continue;
      }
      changed = _apply(read.message!) || changed;
    }
    if (changed) notifyListeners();
  }

  bool _apply(A2uiMessage message) {
    final id = message.surfaceId;
    switch (message) {
      case CreateSurface():
        final existing = surfaces[id];
        if (existing == null && surfaces.length >= limits.maxSurfaces) {
          _problem(A2uiProblem(A2uiProblem.validationFailed, 'A session keeps at most ${limits.maxSurfaces} surfaces', surfaceId: id));
          return false;
        }
        final catalog = message.catalogId == null ? catalogs.fallback : catalogs[message.catalogId!];
        if (catalog == null) {
          _problem(A2uiProblem(A2uiProblem.validationFailed, 'No catalog "${message.catalogId}" here (${catalogs.ids.join(', ')})', surfaceId: id, path: '/createSurface/catalogId'));
          return false;
        }
        final surface = A2uiSurface(id, catalog, limits, sendDataModel: allowSendDataModel && message.sendDataModel);
        surfaces[id] = surface;
        existing?.dispose();
        if (message.dataModel != null) {
          final bad = surface.putData('/', message.dataModel);
          if (bad != null) _problem(bad);
        }
        if (message.components.isNotEmpty) {
          for (final bad in surface.putComponents(message.components)) {
            _problem(bad);
          }
        }
        onChange?.call(id, existing == null ? A2uiSurfaceChange.created : A2uiSurfaceChange.updated);
        return true;
      case UpdateComponents():
        final surface = surfaces[id];
        if (surface == null) return _unknown(id);
        for (final bad in surface.putComponents(message.components)) {
          _problem(bad);
        }
        onChange?.call(id, A2uiSurfaceChange.updated);
        return true;
      case UpdateDataModel():
        final surface = surfaces[id];
        if (surface == null) return _unknown(id);
        final bad = surface.putData(message.path, message.value);
        if (bad != null) {
          _problem(bad);
          return true;
        }
        onChange?.call(id, A2uiSurfaceChange.updated);
        return true;
      case DeleteSurface():
        final gone = surfaces.remove(id);
        if (gone == null) return false;
        gone.dispose();
        onChange?.call(id, A2uiSurfaceChange.deleted);
        return true;
    }
  }

  bool _unknown(String id) {
    _problem(A2uiProblem(A2uiProblem.validationFailed, 'No surface "$id": createSurface comes first', surfaceId: id));
    return false;
  }

  @override
  void dispose() {
    for (final each in surfaces.values) {
      each.dispose();
    }
    super.dispose();
  }
}

enum A2uiSurfaceChange { created, updated, deleted }
