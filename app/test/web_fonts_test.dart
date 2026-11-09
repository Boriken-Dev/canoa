// The web build must draw text under the host's strict policy, which allows the site's own origin alone: CanvasKit
// fetches Roboto and its fallback fonts from fonts.gstatic.com unless the app carries Roboto and names its own base.
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
}
