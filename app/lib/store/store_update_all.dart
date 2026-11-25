import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart' show hostWords;
import 'programs.dart';

// "Update all" without the screen: what one update is, planned and run one after another through the same per-item paths the Store's
// single Update buttons take. The host has no batch method, and none is needed: each item is planned (nothing written), the person
// confirms all the plans at once, and the items then run in order, one at a time, so the host's own locks are never contended.

/// What one planned update does, in the words the plan dialog shows: nothing here has been applied.
final class UpdateItem {
  const UpdateItem({required this.run, this.from, this.to, this.changes = const [], this.details = const [], this.attention = const []});

  /// The version or commit it leaves and the one it goes to, as the item's own page words them.
  final String? from, to;

  /// What it changes, one line each, always shown.
  final List<String> changes;

  /// Longer lists (the files an overlay adds, changes and removes), shown when the row is opened.
  final List<String> details;

  /// What needs the person's eye before they say yes: a download nothing checks, a notice of the publisher, a manifest that changes.
  final List<String> attention;

  /// Carries it out. It reports through [UpdateAllRun.say] and [UpdateAllRun.attach], and throws what the single update would show.
  final Future<void> Function(UpdateAllRun run) run;
}

/// Something that holds an update: its name, and how to plan it. Planning writes nothing.
final class UpdateCandidate {
  const UpdateCandidate({required this.id, required this.title, required this.plan, this.group, this.from, this.to});

  /// A name for keys, unique among the candidates.
  final String id, title;

  /// What the Store type it belongs to is called, which the plan dialog groups by when it holds more than one.
  final String? group;

  /// What the page already knows of the versions, shown until the plan says them.
  final String? from, to;

  /// Asks the host (or the source) what updating would do.
  final Future<UpdateItem> Function() plan;

  UpdateCandidate inGroup(String name) => UpdateCandidate(id: '$name/$id', title: title, plan: plan, group: name, from: from, to: to);
}

/// Where one item stands.
enum UpdateStatus { pending, running, done, failed, notRun }

/// One candidate in a run: its plan once made, where it stands and, when it failed, why.
final class UpdateEntry {
  UpdateEntry(this.candidate);

  final UpdateCandidate candidate;
  UpdateItem? item;
  UpdateStatus status = UpdateStatus.pending;

  /// Why it failed, in the host's words.
  String? reason;

  /// Whether it is being planned.
  bool planning = false;

  /// What it said while it ran, and the install running it when it is a program.
  List<String> lines = [];
  ProgramRun? program;

  String? get from => item?.from ?? candidate.from;
  String? get to => item?.to ?? candidate.to;
}

/// Where a run is.
enum UpdatePhase { planning, review, running, ended }

/// A run of updates: planned together, confirmed once, then applied one at a time. One failing item does not stop the others, it is
/// recorded with its reason; [stop] ends the run after the item being applied; [retry] runs only the failed ones again.
class UpdateAllRun extends ChangeNotifier {
  UpdateAllRun(Iterable<UpdateCandidate> candidates) : entries = [for (final each in candidates) UpdateEntry(each)];

  final List<UpdateEntry> entries;

  UpdatePhase _phase = UpdatePhase.planning;
  UpdatePhase get phase => _phase;

  /// The item being applied now.
  UpdateEntry? current;
  bool _stop = false;
  int _passTotal = 0, _passDone = 0;
  bool _disposed = false;

  /// Whether [stop] was asked and the run has not ended yet.
  bool get stopping => _stop && _phase == UpdatePhase.running;

  /// How many items this pass applies, and how many of them have ended.
  int get total => _passTotal;
  int get finished => _passDone;

  List<UpdateEntry> get ready => [for (final each in entries) if (each.status == UpdateStatus.pending && each.item != null) each];
  List<UpdateEntry> get failed => [for (final each in entries) if (each.status == UpdateStatus.failed) each];
  int get updated => entries.where((each) => each.status == UpdateStatus.done).length;
  int get notRun => entries.where((each) => each.status == UpdateStatus.notRun).length;

  /// Plans every candidate, one after another; one that cannot be planned is marked failed with the reason and the rest go on.
  Future<void> plan() async {
    for (final entry in entries) {
      if (_disposed) return;
      entry.planning = true;
      notifyListeners();
      try {
        entry.item = await entry.candidate.plan();
      } on Object catch (error) {
        entry.status = UpdateStatus.failed;
        entry.reason = hostWords(error);
      }
      entry.planning = false;
    }
    _phase = UpdatePhase.review;
    notifyListeners();
  }

  /// Applies what was planned, in order. Ends when every item has.
  Future<void> start() => _pass(ready);

  /// Plans again, where it was never planned, and applies only the items that failed.
  Future<void> retry() {
    final again = failed;
    for (final entry in again) {
      entry.status = UpdateStatus.pending;
      entry.reason = null;
      entry.lines = [];
    }
    return _pass(again);
  }

  /// Stops after the item being applied: the ones not yet started are left as they were.
  void stop() {
    if (_phase != UpdatePhase.running || _stop) return;
    _stop = true;
    notifyListeners();
  }

  Future<void> _pass(List<UpdateEntry> list) async {
    _stop = false;
    _passTotal = list.length;
    _passDone = 0;
    _phase = UpdatePhase.running;
    notifyListeners();
    for (final entry in list) {
      if (_stop) {
        entry.status = UpdateStatus.notRun;
        continue;
      }
      current = entry;
      entry.status = UpdateStatus.running;
      notifyListeners();
      try {
        entry.item ??= await entry.candidate.plan();
        await entry.item!.run(this);
        entry.status = UpdateStatus.done;
      } on Object catch (error) {
        entry.status = UpdateStatus.failed;
        entry.reason = hostWords(error);
      } finally {
        entry.program?.removeListener(notifyListeners);
        entry.program = null;
        current = null;
        _passDone++;
        notifyListeners();
      }
    }
    _phase = UpdatePhase.ended;
    notifyListeners();
  }

  /// Said by the item being applied: one line of what it is doing.
  void say(String line) {
    current?.lines.add(line);
    notifyListeners();
  }

  /// Follows [run], the install a program's update is: its lines, its bar and its questions show under the item.
  void attach(ProgramRun run) {
    final entry = current;
    if (entry == null) return;
    entry.program = run;
    entry.lines = run.lines;
    run.addListener(notifyListeners);
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
