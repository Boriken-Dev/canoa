/// Skills from elsewhere: what installing or syncing an overlay would fetch, shown before anything is, and what fetching
/// it does once the person has said yes to the overlay and its skills together.
///
/// Planning and checking are coqui_aos's (`resolveSkills`, `readFetchedSkill`); this adds the record of what was
/// accepted, the update a changed resolution is, and the removal rules. Nothing here runs anything a skill holds.
library;

import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../profile/aos_support.dart';
import '../profile/overlays.dart';
import '../profile/skill_copies.dart';

/// What installing an overlay does with one skill it names from elsewhere.
enum SkillStatus {
  /// Not on this computer: it is fetched.
  fetch,

  /// Here from another source, pin or catalog than the overlay names now, or damaged: fetched again, and shown as an
  /// update the yes accepts.
  update,

  /// Here from the very source and pin, and kept.
  unchanged,

  /// A name no catalog offers: not delivered, and the overlay applies without it.
  missing,

  /// A name more than one catalog offers: not delivered, since Canoa never picks one.
  ambiguous,

  /// A holder of the name keeps it: not delivered for this overlay.
  shadowed,

  /// Refused before any fetch.
  refused,
}

/// One skill of an overlay that comes from elsewhere, with where from and what installing does with it.
final class SkillItem {
  const SkillItem({required this.name, required this.overlay, required this.status, this.source, this.pin,
      this.configuredPin, this.catalog, this.catalogs, this.fetch, this.previous, this.why, this.warnings = const []});

  final String name;

  /// The overlay naming it, as AOS normalizes the name; empty for a skill the person chose.
  final String overlay;
  final SkillStatus status;

  /// Where it comes from, as shown: no user information and no query.
  final String? source;

  /// The digest it must have: the entry's or catalog's pin, else the one canoa.cfg's `[skill_pins]` gives; null when
  /// neither names one.
  final String? pin;

  /// The pin `[skill_pins]` gives this skill, which the digest fetched must equal as well.
  final String? configuredPin;

  /// The catalog that gave the source, and for an ambiguous name the catalogs that offer it.
  final String? catalog;
  final List<String>? catalogs;

  /// What to fetch, for [SkillStatus.fetch] and [SkillStatus.update].
  final aos.Fetch? fetch;

  /// The copy that is here now, for an update and for a kept skill.
  final SkillCopy? previous;
  final String? why;

  /// `insecure-transport` and `private-source`, the settings of the source that are not the default.
  final List<String> warnings;

  bool get unpinned =>
      pin == null && (status == SkillStatus.fetch || status == SkillStatus.update || status == SkillStatus.unchanged);
}

/// What an overlay's skills from elsewhere would do, from its manifest, the skills here and the catalogs.
final class SkillReview {
  const SkillReview(this.identity, this.plan, this.items, this.orphans);

  /// The overlay, as AOS normalizes its name.
  final String identity;
  final aos.SkillPlan plan;
  final List<SkillItem> items;

  /// Skills only this overlay names that it no longer does: none names them once it is applied.
  final List<SkillCopy> orphans;

  bool get isEmpty => items.isEmpty && orphans.isEmpty;

  /// Whether applying fetches anything: the one thing a yes must cover.
  bool get fetches => items.any((item) => item.status == SkillStatus.fetch || item.status == SkillStatus.update);
}

String? _shown(String key) {
  try {
    return aos.Source.parse(key).shown();
  } on aos.AosError {
    return null;
  }
}

bool _samePin(String? a, String? b) => a == null || b == null ? a == b : sameDigest(a, b);

bool _sound(SkillCopy copy) {
  try {
    aos.readFetchedSkill(copy.name, copy.pairs, copy.digest, skillLimits);
    return true;
  } on aos.AosError {
    return false;
  }
}

/// Where the skills of the overlays [requested] come from: [manifests] and [trees] by identity, resolved through
/// [catalogs]. The plan every reader of skills in the app starts from; fetches nothing.
aos.SkillPlan planSkills(Map<String, aos.Document> manifests, List<String> requested, Map<String, aos.OverlayTree> trees,
        {List<(String, aos.Catalog)> catalogs = const [], List<String> order = const []}) =>
    aos.resolveSkills(manifests, requested, trees, catalogs, order, true, aos.defaultSettings);

/// An installed overlay, to plan its skills with the others': its manifest and files.
typedef InstalledManifest = (aos.Document, aos.OverlayTree);

/// The pin that holds for a skill whose entry gives [entryPin] and canoa.cfg [configured]: both when they agree, the
/// one given when only one is, and null with [conflict] when they differ, since no tree has two digests.
({String? pin, bool conflict}) effectivePin(String? entryPin, String? configured) {
  if (entryPin != null && configured != null) {
    return sameDigest(entryPin, configured) ? (pin: entryPin, conflict: false) : (pin: null, conflict: true);
  }
  return (pin: entryPin ?? configured, conflict: false);
}

/// What installing or syncing the overlay [identity], whose manifest is [manifest] and files [tree], would do with the
/// skills it names from elsewhere. The skills are planned across [others], the overlays installed besides it, so a name
/// two overlays offer has one holder: a skill from elsewhere never takes one another holds. [catalogs] are
/// `(label, catalog)`, [order] ranks their labels where more than one offers a name, and [pins] are canoa.cfg's
/// `[skill_pins]`. Fetches nothing and writes nothing.
Future<SkillReview> reviewSkills(OverlayManager overlays, {
  required String identity,
  required aos.Document manifest,
  required aos.OverlayTree tree,
  List<(String, aos.Catalog)> catalogs = const [],
  List<String> order = const [],
  Map<String, InstalledManifest> others = const {},
  Map<String, String> pins = const {},
}) async {
  final id = aos.normalizeName(identity);
  final manifests = <String, aos.Document>{
    for (final MapEntry(:key, :value) in others.entries) if (aos.normalizeName(key) != id) aos.normalizeName(key): value.$1,
    id: manifest,
  };
  final trees = <String, aos.OverlayTree>{
    for (final MapEntry(:key, :value) in others.entries) if (aos.normalizeName(key) != id) aos.normalizeName(key): value.$2,
    id: tree,
  };
  aos.SkillPlan plan;
  try {
    plan = planSkills(manifests, manifests.keys.toList(), trees, catalogs: catalogs, order: order);
  } on aos.AosError {
    // A cycle among the others' requires: this overlay is planned alone.
    plan = planSkills({id: manifest}, [id], {id: tree}, catalogs: catalogs, order: order);
  }
  final copies = await overlays.skillCopies();
  final fetches = {
    for (final fetch in plan.toFetch)
      if (fetch.overlay == id) skillKey(fetch.name): fetch,
  };
  String? holderOf(String name) {
    for (final MapEntry(:key, :value) in plan.delivered.entries) {
      if (skillKey(key) == skillKey(name)) return value;
    }
    return null;
  }

  final items = <SkillItem>[];
  final named = <String>{};
  for (final skill in plan.skills.where((skill) => skill.overlay == id)) {
    if (skill.origin == 'bundled') continue;
    final key = skillKey(skill.name);
    final copy = copies[key];
    final configured = pins[key];
    final both = effectivePin(skill.pin, configured);
    final warnings = [
      for (final (setting, name, overlay) in plan.nonDefaults)
        if (overlay == id && skillKey(name) == key) setting,
    ];
    SkillItem item(SkillStatus status, {aos.Fetch? fetch, String? why}) => SkillItem(name: skill.name, overlay: id,
        status: status, source: skill.source, pin: both.pin, configuredPin: configured, catalog: skill.catalog,
        catalogs: skill.catalogs, fetch: fetch, previous: copy, why: why, warnings: warnings);
    if (skill.origin == 'missing' || skill.origin == 'ambiguous') {
      if (copy != null && copy.overlays.contains(id) && copy.catalog != null) {
        // Resolved through a catalog when it was accepted: a catalog that is not here now does not unmake that.
        named.add(key);
        items.add(SkillItem(name: skill.name, overlay: id, status: SkillStatus.unchanged, source: _shown(copy.source),
            pin: copy.pin, configuredPin: configured, catalog: copy.catalog, previous: copy,
            why: 'resolved through the catalog ${copy.catalog} when it was installed, and kept'));
      } else {
        items.add(item(skill.origin == 'missing' ? SkillStatus.missing : SkillStatus.ambiguous));
      }
      continue;
    }
    if (both.conflict) {
      items.add(item(SkillStatus.refused,
          why: 'canoa.cfg [skill_pins] and the entry give different pins, and no tree has both'));
      continue;
    }
    final planned = fetches[key];
    if (planned == null) {
      final holder = holderOf(skill.name);
      items.add(item(SkillStatus.shadowed,
          why: holder == null || holder == id
              ? 'a skill of the same name is already delivered'
              : 'the skill of this name in the overlay $holder is delivered, and a skill from elsewhere never takes a name'));
      continue;
    }
    final fetch = aos.Fetch(name: planned.name, overlay: planned.overlay, from: planned.from, pin: both.pin);
    final wanted = fetch.from.keyWith(null);
    if (copy == null) {
      named.add(key);
      items.add(item(SkillStatus.fetch, fetch: fetch));
      continue;
    }
    final alike = copy.source == wanted && _samePin(copy.pin, both.pin);
    final shared = copy.overlays.any((other) => other != id);
    if (copy.chosen && !alike) {
      items.add(item(SkillStatus.refused,
          why: 'you chose this skill from another source: remove it first, or keep it as it is'));
    } else if (alike && (copy.catalog == skill.catalog || shared)) {
      named.add(key);
      items.add(_sound(copy)
          ? item(SkillStatus.unchanged, fetch: fetch)
          : item(SkillStatus.update,
              fetch: fetch, why: 'its copy here is not what was accepted: fetched again if you say yes'));
    } else if (shared) {
      items.add(item(SkillStatus.refused,
          why: 'another overlay has this skill from another source or pin, and the first to install it keeps the name'));
    } else {
      named.add(key);
      items.add(item(SkillStatus.update, fetch: fetch));
    }
  }
  final orphans = [
    for (final MapEntry(:key, value: copy) in copies.entries)
      if (copy.overlays.contains(id) && !named.contains(key) && copy.overlays.length == 1 && !copy.chosen) copy,
  ];
  return SkillReview(id, plan, items, orphans);
}

/// What became of one skill when its overlay was installed.
enum SkillResult { installed, kept, notDelivered, refused, failed }

final class SkillOutcome {
  const SkillOutcome(this.name, this.result, this.message);
  final String name;
  final SkillResult result;
  final String message;
}

/// What applying a [SkillReview] did.
final class SkillInstallReport {
  final outcomes = <SkillOutcome>[];

  /// Skills no overlay names now and the person did not choose: still here until removed, which is offered.
  final orphans = <SkillCopy>[];

  /// The outcome for [name], or null.
  SkillOutcome? of(String name) => outcomes.where((outcome) => skillKey(outcome.name) == skillKey(name)).firstOrNull;
}

String _refusedBecause(aos.AosError error) => switch (error.errorClass) {
  'pin-mismatch' => 'the files fetched do not have the pin: refused, nothing of it was kept',
  'not-a-skill' => 'it has no SKILL.md: refused',
  'skill-unreadable' => 'its SKILL.md has no front matter in YAML: refused',
  'skill-name-mismatch' => "its SKILL.md names another skill than the one the overlay asks for: refused",
  'too-large' => 'it holds more than a skill may: refused',
  _ => 'refused (${error.errorClass})',
};

/// Writes [copy]; the reason it could not be, or null.
Future<String?> _save(OverlayManager overlays, SkillCopy copy) async {
  try {
    await overlays.saveSkill(copy);
    return null;
  } on OverlayException catch (error) {
    return error.message;
  } on Object catch (error) {
    return 'it could not be written (${error.runtimeType})';
  }
}

Future<SkillOutcome> _fetchAndSave(OverlayManager overlays, SkillItem item, Set<String> naming,
    {required bool chosen}) async {
  final fetch = item.fetch!;
  final List<(String, Uint8List)> files;
  try {
    files = await overlays.fetchSkillFiles(fetch.from);
  } on OverlayException catch (error) {
    return SkillOutcome(item.name, SkillResult.failed, 'it could not be fetched: ${error.message}');
  } on Object catch (error) {
    return SkillOutcome(item.name, SkillResult.failed, 'it could not be fetched (${error.runtimeType})');
  }
  final aos.FetchedSkill read;
  try {
    // The digest must equal the pin of the entry and the one canoa.cfg gives: they were made one before this.
    read = aos.readFetchedSkill(item.name, files, item.pin, skillLimits);
  } on aos.AosError catch (error) {
    return SkillOutcome(item.name, SkillResult.refused, _refusedBecause(error));
  }
  if (item.configuredPin case final configured? when !sameDigest(configured, '${read.digest}')) {
    return SkillOutcome(item.name, SkillResult.refused, 'the files fetched do not have the pin in canoa.cfg [skill_pins]: refused');
  }
  final bytes = {for (final (path, data) in files) path: data};
  final failure = await _save(overlays, SkillCopy(
    name: item.name,
    source: fetch.from.keyWith(null),
    ref: fetch.from.reference,
    pin: item.pin,
    digest: '${read.digest}',
    catalog: item.catalog,
    overlays: naming.toList()..sort(),
    chosen: chosen,
    files: {for (final path in read.files) path: bytes[path]!},
  ));
  if (failure != null) return SkillOutcome(item.name, SkillResult.failed, failure);
  final keys = read.reported.isEmpty ? '' : ', with front matter keys a harness may take as a grant: ${read.reported.join(', ')}';
  return SkillOutcome(item.name, SkillResult.installed,
      'fetched${item.pin == null ? ', unpinned' : ' at its pin'}$keys');
}

/// Applies [review]: fetches each skill it names to fetch, checks its digest against its pin before reading any of its
/// files, writes what passed and records what was accepted, and leaves out what did not, saying why. A skill that fails
/// leaves the others, and the overlay, as they were. Call this only after the person said yes to the overlay and its
/// skills together; it is the one place a skill is fetched.
Future<SkillInstallReport> installSkills(OverlayManager overlays, SkillReview review) async {
  final report = SkillInstallReport();
  final id = review.identity;
  final copies = await overlays.skillCopies();
  for (final item in review.items) {
    final key = skillKey(item.name);
    switch (item.status) {
      case SkillStatus.missing:
        report.outcomes.add(SkillOutcome(item.name, SkillResult.notDelivered, 'named skill; no catalog resolves it'));
      case SkillStatus.ambiguous:
        report.outcomes.add(SkillOutcome(item.name, SkillResult.notDelivered,
            'offered by ${item.catalogs?.join(' and ')}, and Canoa never picks one'));
      case SkillStatus.shadowed || SkillStatus.refused:
        report.outcomes.add(SkillOutcome(item.name, SkillResult.notDelivered, item.why ?? 'not delivered'));
      case SkillStatus.unchanged:
        final copy = copies[key];
        if (copy == null || !_sound(copy)) {
          // It changed since the review, which showed it kept: said, and not fetched without being asked again.
          report.outcomes.add(SkillOutcome(item.name, SkillResult.failed,
              'its copy is not what was accepted: review the overlay again to fetch it'));
        } else if (copy.overlays.contains(id)) {
          report.outcomes.add(SkillOutcome(item.name, SkillResult.kept, item.why ?? 'kept as it was accepted'));
        } else {
          final failure = await _save(overlays, copy.copyWith(overlays: [...copy.overlays, id]..sort()));
          report.outcomes.add(failure == null
              ? SkillOutcome(item.name, SkillResult.kept, item.why ?? 'kept as it was accepted')
              : SkillOutcome(item.name, SkillResult.failed, failure));
        }
      case SkillStatus.fetch || SkillStatus.update:
        final previous = copies[key];
        report.outcomes.add(await _fetchAndSave(overlays, item, {...?previous?.overlays, id},
            chosen: previous?.chosen ?? false));
    }
  }
  report.orphans.addAll(await detachOverlay(overlays, id, keep: {
    for (final item in review.items)
      if (item.status == SkillStatus.fetch || item.status == SkillStatus.update || item.status == SkillStatus.unchanged)
        skillKey(item.name),
  }));
  return report;
}

/// Takes [identity] out of every fetched skill's record except those named in [keep], and gives the skills that no
/// overlay names then and the person did not choose: those are offered for removal, never removed here.
Future<List<SkillCopy>> detachOverlay(OverlayManager overlays, String identity, {Set<String> keep = const {}}) async {
  final id = aos.normalizeName(identity);
  final orphans = <SkillCopy>[];
  for (final MapEntry(:key, value: copy) in (await overlays.skillCopies()).entries) {
    if (keep.contains(key) || !copy.overlays.contains(id)) continue;
    final rest = copy.copyWith(overlays: [for (final overlay in copy.overlays) if (overlay != id) overlay]);
    if (await _save(overlays, rest) != null) continue;
    if (rest.overlays.isEmpty && !rest.chosen) orphans.add(rest);
  }
  return orphans;
}

/// The fetched skills no overlay names and the person did not choose.
Future<List<SkillCopy>> orphanedSkills(OverlayManager overlays) async => [
  for (final copy in (await overlays.skillCopies()).values)
    if (copy.overlays.isEmpty && !copy.chosen) copy,
];

/// Removes the fetched skills [names] that nothing names and the person did not choose; one that is still named or
/// chosen is left, however it is asked for. Gives the names removed.
Future<List<String>> removeOrphans(OverlayManager overlays, Iterable<String> names) async {
  final copies = await overlays.skillCopies();
  final removed = <String>[];
  for (final name in names) {
    final copy = copies[skillKey(name)];
    if (copy == null || copy.overlays.isNotEmpty || copy.chosen) continue;
    await overlays.removeSkill(copy.name);
    removed.add(copy.name);
  }
  return removed;
}

/// What installing the skill [name] that the catalog [label] offers as [entry] would do, by the person's own choice.
/// [pins] are canoa.cfg's `[skill_pins]`.
SkillItem reviewChosenSkill(OverlayManager overlays, Map<String, SkillCopy> copies, String label, String name,
    aos.CatalogEntry entry, {Map<String, String> pins = const {}}) {
  final from = entry.source;
  final copy = copies[skillKey(name)];
  final configured = pins[skillKey(name)];
  final both = effectivePin(entry.pin, configured);
  SkillItem refused(String why) => SkillItem(name: name, overlay: '', status: SkillStatus.refused, source: from.shown(),
      pin: both.pin, configuredPin: configured, catalog: label, previous: copy, why: why);
  if (!validSkillName(name)) return refused('it is not a skill name');
  if (both.conflict) return refused('canoa.cfg [skill_pins] and the catalog give different pins, and no tree has both');
  final same = copy != null && copy.source == from.keyWith(null) && _samePin(copy.pin, both.pin);
  if (copy != null && !same && copy.overlays.isNotEmpty) {
    return refused('an overlay already has a skill of this name from another source or pin');
  }
  final fetch = aos.Fetch(name: name, overlay: '', from: from, pin: both.pin);
  final status = same && _sound(copy) ? SkillStatus.unchanged : copy == null ? SkillStatus.fetch : SkillStatus.update;
  return SkillItem(name: name, overlay: '', status: status, source: from.shown(), pin: both.pin,
      configuredPin: configured, catalog: label, fetch: fetch, previous: copy,
      why: same && status == SkillStatus.update ? 'its copy here is not what was accepted: fetched again if you say yes' : null);
}

/// Installs the skill [item] reviewed as the person's own choice: fetched and checked as an overlay's skill is, recorded
/// as chosen, so no overlay's removal takes it away. An overlay that names it later adds itself to its record.
Future<SkillOutcome> installChosenSkill(OverlayManager overlays, SkillItem item) async {
  if (item.status == SkillStatus.refused || item.fetch == null) {
    return SkillOutcome(item.name, SkillResult.notDelivered, item.why ?? 'not installed');
  }
  final previous = item.previous;
  if (item.status == SkillStatus.unchanged && previous != null) {
    final failure = await _save(overlays, previous.copyWith(chosen: true));
    return failure == null
        ? SkillOutcome(item.name, SkillResult.kept, 'already here, and now yours by choice')
        : SkillOutcome(item.name, SkillResult.failed, failure);
  }
  return _fetchAndSave(overlays, item, {...?previous?.overlays}, chosen: true);
}

/// Gives up the person's own choice of the skill [name]: it stays while an overlay names it, and goes when none does.
/// Gives whether it was removed.
Future<bool> releaseChosenSkill(OverlayManager overlays, String name) async {
  final copy = (await overlays.skillCopies())[skillKey(name)];
  if (copy == null || !copy.chosen) return false;
  if (copy.overlays.isEmpty) {
    await overlays.removeSkill(copy.name);
    return true;
  }
  final failure = await _save(overlays, copy.copyWith(chosen: false));
  return failure != null ? throw OverlayException(failure) : false;
}
