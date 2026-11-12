import 'dart:convert';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import 'aos_support.dart';
import 'overlay_layer.dart';
import 'profile.dart';
import 'profile_home.dart';
import 'project_trust.dart';
import 'skill_copies.dart';

/// Canoa's own layer, lowest of all. It sets nothing: every value comes from a layer the person can see and change.
const builtInProfile = Profile();

/// The AOS harness id of each of Canoa's agent presets, whose `[harness.<id>]` hints apply to it.
const presetHarnesses = {'Claude': 'claude-code', 'Codex': 'codex', 'Grok': 'grok', 'Pi': 'pi'};

/// Why a layer is not in the merge, or what to know about one that is.
final class LayerNote {
  const LayerNote(this.layer, this.text);
  final LayerId layer;
  final String text;
}

/// A project's directory and what became of its profile.
final class ProjectLayer {
  const ProjectLayer(this.directory, this.state, [this.profile]);

  /// The working directory, links resolved.
  final String directory;
  final TrustState state;

  /// The profile's bytes, which trusting holds: what the person trusts is what they were shown.
  final List<int>? profile;
}

/// Every layer for one agent and directory, lowest first, and what was left out and why.
final class ProfileLayers {
  const ProfileLayers(this.layers, this.notes, {this.project});
  final List<Layer> layers;
  final List<LayerNote> notes;
  final ProjectLayer? project;

  EffectiveProfile get effective => mergeProfiles(layers);
}

/// The layers for [agent]: built-in, each AOS overlay of [overlays] in composition order (by `priority`, then name),
/// the [user]'s own, the project's in [workingDirectory] once trusted, the agent entry's from [agents], then
/// [session]. An overlay's files are digested each time and checked against its record and its pin; the hints of
/// [harness] (by default the preset's, from [presetHarnesses]) apply, and an `[about]` icon is kept within
/// [iconLimits]. With no [home], as in a web build, overlays and projects are left out and say so.
Future<ProfileLayers> profileLayers({
  required Map<String, OverlaySource> overlays,
  required Profile user,
  required Map<String, Profile> agents,
  required String agent,
  ProfileHome? home,
  String? workingDirectory,
  Profile? session,
  String? harness,
  aos.Settings? iconLimits,
  Map<String, String> skillPins = const {},
}) async {
  harness ??= presetHarnesses[agent];
  final layers = <Layer>[const Layer(LayerId(LayerKind.builtIn), builtInProfile)];
  final notes = <LayerNote>[];
  final found = <String, Layer>{};
  final manifests = <String, aos.Document>{};
  final names = <String, String>{};
  final fetched = home == null ? const <String, SkillCopy>{} : await home.overlays.skillCopies();
  for (final MapEntry(key: name, value: source) in overlays.entries) {
    final id = LayerId(LayerKind.overlay, name);
    if (home == null) {
      notes.add(LayerNote(id, 'Overlays are read by the desktop app'));
      continue;
    }
    if (!aos.validOverlayName(name)) {
      notes.add(LayerNote(id, 'Not a valid overlay identity: a letter, then letters, digits, _, . or -, ending in a '
          'letter or digit, and not user, project or a device name'));
      continue;
    }
    final normalized = aos.normalizeName(name);
    if (names[normalized] case final other?) {
      notes.add(LayerNote(id, 'The same overlay as $other, once names are compared as AOS compares them: left out'));
      continue;
    }
    names[normalized] = name;
    final String directory;
    String? expected;
    switch (source) {
      case GitOverlay():
        final installed = home.overlays.installed(name);
        if (installed == null) {
          notes.add(LayerNote(id, 'Not installed: install it to use it'));
          continue;
        }
        if (installed.source != source.spec) {
          notes.add(LayerNote(id, 'Installed from ${installed.source}, not the source canoa.cfg names: sync it'));
          continue;
        }
        final digest = installed.digest;
        if (digest == null) {
          notes.add(LayerNote(id, 'Installed with no tree digest: sync it'));
          continue;
        }
        if (source.pin case final pin? when !sameDigest(pin, digest)) {
          notes.add(LayerNote(id, 'canoa.cfg pins it to $pin, and $digest is installed: sync it'));
          continue;
        }
        directory = home.overlays.directoryOf(name);
        expected = digest;
        final commit = installed.commit;
        notes.add(LayerNote(id, 'At commit ${commit.length > 12 ? commit.substring(0, 12) : commit}'));
      case DirOverlay(:final dir, :final pin):
        final there = home.canonicalDirectory(isAbsolutePath(dir) ? dir : home.join(home.configDir, dir));
        if (there == null) {
          notes.add(LayerNote(id, 'Its directory is not there'));
          continue;
        }
        directory = there;
        expected = pin;
    }
    final ReadTree read;
    try {
      read = await home.readTree(directory);
    } on aos.AosError catch (error) {
      notes.add(LayerNote(id, 'Its files cannot be read (${error.message}): left out'));
      continue;
    } on Object catch (error) {
      notes.add(LayerNote(id, 'Its files cannot be read ($error): left out'));
      continue;
    }
    final Layer layer;
    try {
      layer = overlayLayer(identity: name, root: directory, os: home.os, tree: read.tree, harness: harness,
          iconLimits: iconLimits, fetched: fetched, skillPins: skillPins, pinned: switch (source) {
            GitOverlay(:final pin) => pin != null,
            DirOverlay(:final pin) => pin != null,
          });
    } on OverlayRefusal catch (refusal) {
      notes.add(LayerNote(id, 'Refused, so left out: $refusal'));
      continue;
    }
    if (expected != null && !sameDigest(expected, layer.aos!.digest)) {
      notes.add(LayerNote(id, 'Its files changed: their digest is ${layer.aos!.digest}, not $expected. Left out'
          '${source is GitOverlay ? ' until synced' : ''}'));
      continue;
    }
    found[normalized] = layer;
    manifests[normalized] = layer.aos!.manifest;
  }
  var resolution = const aos.Resolution(installOrder: [], missing: []);
  while (true) {
    try {
      resolution = aos.resolve(manifests, manifests.keys.toList());
      break;
    } on aos.Cycle catch (cycle) {
      for (final identity in cycle.chain.toSet()) {
        if (manifests.remove(identity) != null) {
          notes.add(LayerNote(LayerId(LayerKind.overlay, names[identity] ?? identity),
              'In a cycle of requires (${cycle.chain.join(' -> ')}): left out'));
        }
      }
    }
  }
  for (final (by, name) in resolution.missing) {
    notes.add(LayerNote(LayerId(LayerKind.overlay, names[by] ?? by),
        'Requires $name, which [overlays] does not name: it applies without it'));
  }
  final ordered = aos.compositionOrder(manifests, resolution.installOrder);
  // A name two overlays offer has one holder: a bundled skill wins over one from elsewhere whichever comes first, and
  // among equals the later overlay does; a skill from elsewhere never takes a name another holds.
  final delivery = aos.Delivery();
  for (final identity in ordered) {
    final layer = found[identity]!;
    for (final name in layer.profile.skills.keys) {
      final held = layer.aos?.fetchedSkills[name];
      delivery.offer(name, identity, held == null,
          held == null ? aos.Held.bundled(identity) : aos.Held.elsewhere(held.source, held.pin));
    }
  }
  for (final identity in ordered) {
    final layer = found[identity]!;
    final lost = {
      for (final name in layer.profile.skills.keys)
        if (layer.aos?.fetchedSkills.containsKey(name) == true && !delivery.holds(name, identity)) name,
    };
    if (lost.isEmpty) continue;
    found[identity] = layer.withoutSkills(lost);
    notes.add(LayerNote(layer.id, 'Not delivered: ${lost.join(', ')}; another overlay holds ${lost.length == 1 ? 'that name' : 'those names'}, '
        'and a skill from elsewhere never takes a name'));
  }
  layers.addAll([for (final identity in ordered) found[identity]!]);
  layers.add(Layer(const LayerId(LayerKind.user), user, dir: home?.configDir));
  ProjectLayer? project;
  if (workingDirectory != null && home != null) {
    const id = LayerId(LayerKind.project);
    final directory = home.canonicalDirectory(workingDirectory);
    if (directory == null) {
      notes.add(const LayerNote(id, 'The working directory is not there'));
    } else {
      final bytes = await home.readBytes(home.join(directory, projectProfileFile));
      await home.trust.load();
      final state = home.trust.state(directory, bytes);
      project = ProjectLayer(directory, state, bytes);
      switch (state) {
        case TrustState.none:
          break;
        case TrustState.untrusted:
          notes.add(const LayerNote(id, 'This directory is not trusted: its profile is ignored'));
        case TrustState.changed:
          notes.add(const LayerNote(id, 'Its profile changed since you trusted it: ignored until you trust it again'));
        case TrustState.trusted:
          final layer = _parse(bytes!, id, directory, notes);
          if (layer != null) layers.add(layer);
      }
    }
  }
  if (agents[agent] case final profile?) {
    layers.add(Layer(LayerId(LayerKind.agent, agent), profile, dir: home?.configDir));
  }
  if (session != null) layers.add(Layer(const LayerId(LayerKind.session), session));
  return ProfileLayers(layers, notes, project: project);
}

Layer? _parse(List<int> bytes, LayerId id, String directory, List<LayerNote> notes) {
  try {
    return Layer(id, Profile.parse(utf8.decode(bytes), id.kind), dir: directory);
  } on ProfileException catch (error) {
    notes.add(LayerNote(id, 'Refused, so left out: $error'));
  } on FormatException {
    notes.add(LayerNote(id, 'Refused, so left out: the profile is not UTF-8'));
  }
  return null;
}
