// The web build must draw text under the host's strict policy, which allows the site's own origin alone: CanvasKit
// fetches Roboto and its fallback fonts from fonts.gstatic.com unless the app carries Roboto and names its own base,
// and the build then puts the fallback fonts there (tools/fallback_fonts.py). It registers no service worker.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the app declares the Roboto family, so CanvasKit has nothing to download for it', () {
    final pubspec = File('pubspec.yaml').readAsStringSync();
    expect(pubspec, contains('family: Roboto'));
    final assets = RegExp(r'asset: (assets/fonts/Roboto-[a-z]+\.ttf)').allMatches(pubspec).map((m) => m.group(1)!);
    expect(assets, isNotEmpty);
    for (final asset in assets) {
      expect(File(asset).existsSync(), isTrue, reason: asset);
    }
  });

  test('the web bootstrap sends the engine\'s fallback font requests to the site, not to Google', () {
    final bootstrap = File('web/flutter_bootstrap.js').readAsStringSync();
    expect(bootstrap, contains('fontFallbackBaseUrl'));
    expect(bootstrap, isNot(contains('gstatic')));
    expect(RegExp(r'fontFallbackBaseUrl:\s*"(?!https?:)').hasMatch(bootstrap), isTrue);
  });

  test('the web bootstrap registers no service worker, and the loading script drops one an older build left', () {
    final bootstrap = File('web/flutter_bootstrap.js').readAsStringSync();
    expect(bootstrap, isNot(contains('serviceWorkerSettings')));
    expect(bootstrap, isNot(contains('flutter_service_worker_version')));
    final splash = File('web/splash.js').readAsStringSync();
    expect(splash, contains('getRegistrations'));
    expect(splash, contains('unregister'));
  });

  test('the fallback fonts are pinned by digest, one line each, at the paths the engine requests', () {
    final lines = File('../tools/fallback_fonts.sha256').readAsLinesSync().where((l) => !l.startsWith('#') && l.isNotEmpty);
    expect(lines, isNotEmpty);
    final line = RegExp(r'^[0-9a-f]{64}  [a-z0-9]+/v[0-9]+/[A-Za-z0-9._-]+\.woff2$');
    for (final entry in lines) {
      expect(line.hasMatch(entry), isTrue, reason: entry);
    }
    expect(lines.any((l) => l.contains('notosanssymbols/')), isTrue);
    expect(lines.any((l) => l.contains('notocoloremoji/')), isTrue);
  });
}
