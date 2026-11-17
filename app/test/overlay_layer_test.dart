// What an overlay's text can and cannot make of a layer: Canoa's use of the AOS reader, not the reader itself.
import 'dart:convert';

import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/overlay_layer.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/session.dart';
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

  group('path additions', () {
    Layer layerOf(Map<String, String> files, {String os = 'linux', String root = '/ov/team'}) => overlayLayer(
      identity: 'team',
      root: root,
      os: os,
      tree: treeOfFiles({for (final MapEntry(:key, :value) in files.entries) key: utf8.encode(value)}).tree,
    );
    Map<String, String> environmentOf(Layer layer, Map<String, String> host) {
      final built = buildOverlayEnvironment([layer.aos!], const {}, (name) => host[name]);
      return {
        for (final name in const ['PATH', 'PYTHONPATH', 'NODE_PATH'])
          name: ?built.owned(name),
      };
    }

    const host = {'PATH': 'host-bin', 'PYTHONPATH': 'host-py', 'NODE_PATH': 'host-node'};
    const files = {'bin/tool': 'x', 'lib/python/t.py': 'x', 'lib/node/t.js': 'x'};

    test("an overlay's bin and lib go first on the host's own path variables", () {
      expect(environmentOf(layerOf(files), host), {
        'PATH': '/ov/team/bin:host-bin',
        'PYTHONPATH': '/ov/team/lib/python:host-py',
        'NODE_PATH': '/ov/team/lib/node:host-node',
      });
      expect(environmentOf(layerOf(files, os: 'windows', root: r'C:\ov\team'), host), {
        'PATH': r'C:\ov\team\bin;host-bin',
        'PYTHONPATH': r'C:\ov\team\lib\python;host-py',
        'NODE_PATH': r'C:\ov\team\lib\node;host-node',
      });
      expect(layerOf(files).aos!.notApplied, isEmpty);
    });

    test('a path that leaves the overlay is refused, and any variable may take a path addition', () {
      for (final path in ['../up', 'bin/../../up', '/etc', 'C:/Windows']) {
        expect(
          () => layerOf({'overlay.cfg': '[[paths]]\nvariable = "MANPATH"\npath = "$path"\n'}),
          throwsA(isA<OverlayRefusal>().having((e) => e.code, 'code', anyOf('invalid', 'path'))),
          reason: path,
        );
      }
      for (final variable in ['LD_LIBRARY_PATH', 'PERL5LIB', 'HOME', 'NODE_OPTIONS']) {
        layerOf({'overlay.cfg': '[[paths]]\nvariable = "$variable"\npath = "lib"\n', 'lib/x': 'x'});
      }
    });
  });

  group('variables', () {
    Layer layerOf(Map<String, String> files) => overlayLayer(
      identity: 'team',
      root: '/ov/team',
      os: 'linux',
      tree: treeOfFiles({for (final MapEntry(:key, :value) in files.entries) key: utf8.encode(value)}).tree,
    );

    test('every variable applies, and the sensitive ones are listed in the report', () {
      final layer = layerOf({
        'overlay.cfg': '[env]\nNODE_OPTIONS = "--require x"\nLD_PRELOAD = "x.so"\nHTTPS_PROXY = "http://proxy.example"\n'
            'ANTHROPIC_BASE_URL = "https://gateway.example"\nCODEX_HOME = "x"\n',
      });
      for (final name in ['NODE_OPTIONS', 'LD_PRELOAD', 'HTTPS_PROXY', 'ANTHROPIC_BASE_URL', 'CODEX_HOME']) {
        expect(layer.profile.env, contains(name));
      }
      final view = mergeProfiles([layer]).view();
      final listed = [
        for (final setting in ((view['overlays']! as List).first as Map)['non_defaults'] as List)
          if ((setting as List).first == 'env-sensitive') setting[1],
      ];
      expect(listed, containsAll(['NODE_OPTIONS', 'LD_PRELOAD', 'HTTPS_PROXY', 'ANTHROPIC_BASE_URL', 'CODEX_HOME']));
    });

    test('only the names the specification reserves for the host are refused', () {
      for (final name in ['AOS_MODE', 'FOO_OVERLAY_ROOT', 'foo_overlay_state']) {
        expect(
          () => layerOf({'env': '$name=x\n'}),
          throwsA(isA<OverlayRefusal>().having((e) => e.code, 'code', 'reserved').having((e) => e.subject, 'subject', name)),
          reason: name,
        );
        expect(
          () => layerOf({'overlay.cfg': '[env]\n$name = "x"\n'}),
          throwsA(isA<OverlayRefusal>().having((e) => e.code, 'code', anyOf('reserved', 'invalid'))),
          reason: name,
        );
      }
    });

    Future<Map<String, String>> environmentOf(Layer layer, SessionVariables session) async {
      final effective = mergeProfiles([layer], session: session);
      final env = await effective.environment(
        lookup: (_) => null,
        readFile: (path, {within}) async => null,
        session: session,
      );
      return {for (final MapEntry(:key, :value) in env.entries) key: value.expose()};
    }

    test('an overlay reads the identity variables and the host\'s own, and the agent is given the identity', () async {
      for (final (harness, vendor) in [
        ('claude-code', 'anthropic'),
        ('codex', 'openai'),
        ('antigravity', 'google'),
        ('grok', 'xai'),
      ]) {
        final env = await environmentOf(
          layerOf({'overlay.cfg': '[env]\nWHO = "\${AGENTS_HARNESS}/\${AGENTS_VENDOR}/\${AGENT}"\n'}),
          SessionVariables(SessionWho(harness: harness, profile: 'main')),
        );
        expect(env['WHO'], '$harness/$vendor/$harness');
        expect(env['AGENTS_HARNESS'], harness);
        expect(env['AGENTS_VENDOR'], vendor);
        expect(env['AGENTS_HOST'], 'canoa');
        expect(env['AGENTS_PROFILE'], 'main');
      }
      final env = await environmentOf(
        layerOf({
          'overlay.cfg': '[env]\nAGENTS_MODEL = "mine"\nSTATE = "\${CODEX_HOME}/thing"\nCODEX_HOME = "\${CODEX_HOME}/own"\n',
        }),
        SessionVariables(const SessionWho(harness: 'codex', model: 'gpt'), host: const {'CODEX_HOME': '/state'}),
      );
      expect(env['AGENTS_MODEL'], 'mine');
      expect(env['STATE'], '/state/thing');
      expect(env['CODEX_HOME'], '/state/own');
    });

    test('a variable with no value is not set, and none is ever empty', () async {
      final env = await environmentOf(
        layerOf({'overlay.cfg': '[env]\nWHO = "\${AGENTS_HARNESS}"\n'}),
        SessionVariables(const SessionWho(harness: 'pi', model: '', account: '')),
      );
      for (final name in ['AGENTS_VENDOR', 'AGENTS_MODEL', 'AGENTS_ACCOUNT', 'AGENTS_ROLE', 'AGENTS_SESSION_ID']) {
        expect(env, isNot(contains(name)), reason: name);
      }
      expect(env.values, everyElement(isNotEmpty));
      expect(accountWords('claude-login', const [], null), ('subscription', 'anthropic'));
      expect(accountWords('codex-api-key', const [], null), ('api-key', 'openai'));
      expect(accountWords('foundry-local', const [], null), ('local', 'microsoft-foundry'));
      expect(accountWords('harness-login', const [], 'codex'), ('subscription', 'openai'));
    });
  });
}
