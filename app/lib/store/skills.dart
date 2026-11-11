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

  /// Here from another source or pin than the overlay names now: fetched again, and shown as an update.
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
      this.catalog, this.catalogs, this.fetch, this.previous, this.why});

  final String name;

  /// The overlay naming it, as AOS normalizes the name; empty for a skill the person chose.
  final String overlay;
  final SkillStatus status;

  /// Where it comes from, as shown: no user information and no query.
  final String? source;

  /// The digest it must have, or null for an entry with no pin.
  final String? pin;

  /// The catalog that gave the source, and for an ambiguous name the catalogs that offer it.
  final String? catalog;
  final List<String>? catalogs;

  /// What to fetch, for [SkillStatus.fetch] and [SkillStatus.update].
  final aos.Fetch? fetch;

  /// The copy that is here now, for an update and for a kept skill.
  final SkillCopy? previous;
  final String? why;

  bool get unpinned => pin == null && (status == SkillStatus.fetch || status == SkillStatus.update || status == SkillStatus.unchanged);
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

/// Where the skills of the overlays [requested] come from: [manifests] and [trees] by identity, resolved through
/// [catalogs]. The plan every reader of skills in the app starts from; fetches nothing.
aos.SkillPlan planSkills(Map<String, aos.Document> manifests, List<String> requested, Map<String, aos.OverlayTree> trees,
        {List<(String, aos.Catalog)> catalogs = const [], List<String> order = const []}) =>
    aos.resolveSkills(manifests, requested, trees, catalogs, order, true, aos.defaultSettings);

/// What installing or syncing the overlay [identity], whose manifest is [manifest] and files [tree], would do with the
/// skills it names from elsewhere. [catalogs] are `(label, catalog)` and [order] ranks their labels where more than one
/// offers a name. Fetches nothing and writes nothing.
Future<SkillReview> reviewSkills(OverlayManager overlays, {
  required String identity,
  required aos.Document manifest,
  required aos.OverlayTree tree,
  List<(String, aos.Catalog)> catalogs = const [],
  List<String> order = const [],
}) async {
  final id = aos.normalizeName(identity);
  final plan = planSkills({id: manifest}, [id], {id: tree}, catalogs: catalogs, order: order);
  final copies = await overlays.skillCopies();
  final fetches = {for (final fetch in plan.toFetch) skillKey(fetch.name): fetch};
  final items = <SkillItem>[];
  final named = <String>{};
  for (final skill in plan.skills) {
    if (skill.origin == 'bundled') continue;
    final key = skillKey(skill.name);
    final copy = copies[key];
    SkillItem item(SkillStatus status, {aos.Fetch? fetch, String? why}) => SkillItem(name: skill.name, overlay: id,
        status: status, source: skill.source, pin: skill.pin, catalog: skill.catalog, catalogs: skill.catalogs,
        fetch: fetch, previous: copy, why: why);
    if (skill.origin == 'missing' || skill.origin == 'ambiguous') {
      if (copy != null && copy.overlays.contains(id) && copy.catalog != null) {
        // Resolved through a catalog when it was accepted: a catalog that is not here now does not unmake that.
        named.add(key);
        items.add(SkillItem(name: skill.name, overlay: id, status: SkillStatus.unchanged, source: _shown(copy.source),
            pin: copy.pin, catalog: copy.catalog, previous: copy,
            why: 'resolved through the catalog ${copy.catalog} when it was installed, and kept'));
      } else {
        items.add(item(skill.origin == 'missing' ? SkillStatus.missing : SkillStatus.ambiguous));
      }
      continue;
    }
    final fetch = fetches[key];
    if (fetch == null) {
      items.add(item(SkillStatus.shadowed, why: 'a skill of the same name is already delivered'));
      continue;
    }
    final wanted = fetch.from.keyWith(null);
    if (copy == null) {
      named.add(key);
      items.add(item(SkillStatus.fetch, fetch: fetch));
    } else if (copy.source == wanted && _samePin(copy.pin, fetch.pin)) {
      named.add(key);
      items.add(item(SkillStatus.unchanged, fetch: fetch));
    } else if (copy.overlays.any((other) => other != id)) {
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

Future<SkillOutcome> _fetchAndSave(OverlayManager overlays, SkillItem item, Set<String> naming,
    {required bool chosen}) async {
  final fetch = item.fetch!;
  final List<(String, Uint8List)> files;
  try {
    files = await overlays.fetchSkillFiles(fetch.from);
  } on OverlayException catch (error) {
    return SkillOutcome(item.name, SkillResult.failed, 'it could not be fetched: ${error.message}');
  }
  final aos.FetchedSkill read;
  try {
    read = aos.readFetchedSkill(item.name, files, item.pin, skillLimits);
  } on aos.AosError catch (error) {
    return SkillOutcome(item.name, SkillResult.refused, _refusedBecause(error));
  }
  final bytes = {for (final (path, data) in files) path: data};
  await overlays.saveSkill(SkillCopy(
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
        final copy = copies[key]!;
        var sound = true;
        try {
          aos.readFetchedSkill(copy.name, copy.pairs, copy.digest, skillLimits);
        } on aos.AosError {
          sound = false;
        }
        if (sound) {
          if (!copy.overlays.contains(id)) await overlays.saveSkill(copy.copyWith(overlays: [...copy.overlays, id]..sort()));
          report.outcomes.add(SkillOutcome(item.name, SkillResult.kept, item.why ?? 'kept as it was accepted'));
        } else if (item.fetch != null) {
          // What is on disk is not what was recorded: fetched again, and held to the same pin.
          report.outcomes.add(await _fetchAndSave(overlays, item, {...copy.overlays, id}, chosen: copy.chosen));
        } else {
          report.outcomes.add(SkillOutcome(item.name, SkillResult.failed, 'its copy is not what was accepted'));
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
    await overlays.saveSkill(rest);
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
SkillItem reviewChosenSkill(OverlayManager overlays, Map<String, SkillCopy> copies, String label, String name,
    aos.CatalogEntry entry) {
  final from = entry.source;
  final copy = copies[skillKey(name)];
  final same = copy != null && copy.source == from.keyWith(null) && _samePin(copy.pin, entry.pin);
  final fetch = aos.Fetch(name: name, overlay: '', from: from, pin: entry.pin);
  if (!validSkillName(name)) {
    return SkillItem(name: name, overlay: '', status: SkillStatus.refused, source: from.shown(), pin: entry.pin,
        catalog: label, why: 'it is not a skill name');
  }
  if (copy != null && !same && copy.overlays.isNotEmpty) {
    return SkillItem(name: name, overlay: '', status: SkillStatus.refused, source: from.shown(), pin: entry.pin,
        catalog: label, previous: copy, why: 'an overlay already has a skill of this name from another source or pin');
  }
  return SkillItem(name: name, overlay: '', status: same ? SkillStatus.unchanged : copy == null ? SkillStatus.fetch : SkillStatus.update,
      source: from.shown(), pin: entry.pin, catalog: label, fetch: fetch, previous: copy);
}

/// Installs the skill [item] reviewed as the person's own choice: fetched and checked as an overlay's skill is, recorded
/// as chosen, so no overlay's removal takes it away. An overlay that names it later adds itself to its record.
Future<SkillOutcome> installChosenSkill(OverlayManager overlays, SkillItem item) async {
  if (item.status == SkillStatus.refused || item.fetch == null) {
    return SkillOutcome(item.name, SkillResult.notDelivered, item.why ?? 'not installed');
  }
  final previous = item.previous;
  if (item.status == SkillStatus.unchanged && previous != null) {
    await overlays.saveSkill(previous.copyWith(chosen: true));
    return SkillOutcome(item.name, SkillResult.kept, 'already here, and now yours by choice');
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
  await overlays.saveSkill(copy.copyWith(chosen: false));
  return false;
}
