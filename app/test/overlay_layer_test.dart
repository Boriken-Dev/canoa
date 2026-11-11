// What an overlay's text can and cannot make of a layer: Canoa's use of the AOS reader, not the reader itself.
import 'dart:convert';

import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/overlay_layer.dart';
import 'package:canoa/profile/profile.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a harness hint that is not one plain line is not applied', () {
    final layer = overlayLayer(
      identity: 'o',
      root: '/ov/o',
      os: 'linux',
      harness: 'claude-code',
      tree: treeOfFiles({
        'overlay.cfg': utf8.encode('[harness.claude-code]\nmodel = "claude\\nfake: row"\neffort = "high"\n'),
      }).tree,
    );
    expect(layer.profile.model, isNull);
    expect(layer.profile.effort, 'high');
    expect(layer.aos!.notApplied, contains(startsWith('harness.claude-code.model')));
  });

  test('a valid [about] icon is kept as the bytes the digest covered, and an invalid one is not', () {
    final png = base64Decode(
        'iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR42mNQc6v4TwlmGDVg1IBRA4aLAQD27eMQzhBbwQAAAABJRU5ErkJggg==');
    Map<String, List<int>> iconOf(String path, List<int> bytes) => {
      for (final entry in overlayLayer(
        identity: 'o',
        root: '/ov/o',
        os: 'linux',
        tree: treeOfFiles({'overlay.cfg': utf8.encode('[about]\nicon = "$path"\n'), path: bytes}).tree,
      ).aos!.icon.entries)
        entry.key: entry.value,
    };
    expect(iconOf('icon.png', png), {'icon.png': png});
    expect(iconOf('icon.png', utf8.encode('<svg/>')), isEmpty, reason: 'not a PNG');
  });

  group('OVERLAY.md', () {
    Layer layerOf(Map<String, String> files) => overlayLayer(
      identity: 'team',
      root: '/ov/team',
      os: 'linux',
      tree: treeOfFiles({for (final MapEntry(:key, :value) in files.entries) key: utf8.encode(value)}).tree,
    );
    const skill = {'skills/review/SKILL.md': '---\nname: review\n---\n'};
    List<(String?, String?)> contextsOf(Layer layer) => [for (final c in layer.profile.contexts) (c.text, c.file)];

    test('applies as the overlay.cfg and CONTEXT.md form does', () {
      const body = '# Team\n\nBe brief.\n';
      final md = layerOf({
        'OVERLAY.md': '---\nname: team\nenv:\n  TEAM_MODE: strict\n  TEAM_NOTE: two words\n---\n$body',
        ...skill,
      });
      final cfg = layerOf({
        'overlay.cfg': 'name = "team"\n[env]\nTEAM_MODE = "strict"\nTEAM_NOTE = "two words"\n',
        'CONTEXT.md': body,
        ...skill,
      });
      expect(contextsOf(md), [(body, null)]);
      expect(contextsOf(cfg), [(null, 'CONTEXT.md')]);
      Map<String, Object?> rest(Layer layer) => {
        for (final MapEntry(:key, :value) in mergeProfiles([layer]).view().entries)
          if (key != 'contexts' && key != 'overlays') key: value,
      };
      expect(jsonEncode(rest(md)), jsonEncode(rest(cfg)), reason: "the same profile but for the context's form");
      expect(md.aos!.manifest, cfg.aos!.manifest);
      expect(md.aos!.notApplied, cfg.aos!.notApplied);
    });

    test('a body that is context ignores CONTEXT.md and says so', () {
      final layer = layerOf({'OVERLAY.md': '---\nname: team\n---\nFrom the body.\n', 'CONTEXT.md': 'From the file.\n'});
      expect(contextsOf(layer), [('From the body.\n', null)]);
      expect(layer.aos!.notApplied, ['context: CONTEXT.md, ignored while the body of OVERLAY.md is the context']);
    });

    test('naming OVERLAY.md as context puts its body there, and a blank body is no context', () {
      final named = layerOf({'OVERLAY.md': '---\ndefaults:\n  context: false\ncontext: [OVERLAY.md]\n---\nThe body.\n'});
      expect(contextsOf(named), [('The body.\n', null)]);
      expect(layerOf({'OVERLAY.md': '---\ncontext: [OVERLAY.md]\n---\n \n'}).profile.contexts, isEmpty);
    });

    test('both manifests, a body that is not text, and YAML outside the subset refuse the overlay', () {
      void refused(Map<String, List<int>> files, String subject) {
        expect(
          () => overlayLayer(identity: 'team', root: '/ov/team', os: 'linux', tree: treeOfFiles(files).tree),
          throwsA(isA<OverlayRefusal>()
              .having((e) => e.code, 'code', 'manifest')
              .having((e) => e.subject, 'subject', startsWith(subject))),
        );
      }

      refused({'OVERLAY.md': utf8.encode('---\n---\n'), 'overlay.cfg': utf8.encode('')}, 'two-manifests');
      refused({'OVERLAY.md': [...utf8.encode('---\nname: team\n---\n'), 0xff, 0xfe]}, 'invalid-utf8');
      refused({'OVERLAY.md': utf8.encode('---\nenv:\n  A: &x one\n---\n')}, 'yaml-anchor');
      refused({'OVERLAY.md': utf8.encode('---\nenv:\n  A: !!str one\n---\n')}, 'yaml-tag');
      refused({'OVERLAY.md': utf8.encode('---\nenv:\n  <<: {A: one}\n---\n')}, 'yaml-merge-key');
      refused({'OVERLAY.md': utf8.encode('---\nenv:\n  A: one\n')}, 'unterminated-front-matter');
    });
  });
}
