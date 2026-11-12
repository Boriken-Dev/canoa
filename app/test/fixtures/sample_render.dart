import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// The directory named by CANOA_SAMPLES_DIR, or null when sample rendering is off.
String? get samplesDir {
  // A browser test has no environment to read.
  if (kIsWeb) return null;
  final dir = Platform.environment['CANOA_SAMPLES_DIR'];
  return dir == null || dir.isEmpty ? null : dir;
}

/// Writes each golden scene a test draws as a PNG under [dir], named after the golden, instead of comparing it.
/// Pair it with [loadSampleFonts]: the goldens' own comparison stays untouched when [dir] is not set.
final class SampleWriter extends GoldenFileComparator {
  SampleWriter(this.dir);
  final String dir;

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    final out = File('$dir${Platform.pathSeparator}${golden.pathSegments.last}');
    await out.parent.create(recursive: true);
    await out.writeAsBytes(imageBytes);
    return true;
  }

  @override
  Future<void> update(Uri golden, Uint8List imageBytes) => compare(imageBytes, golden);
}

/// Registers the app's bundled Roboto and the Material icon font, which the test engine otherwise draws as boxes.
/// `monospace` is the first of a few common system monospace faces present on this machine, else Roboto.
Future<void> loadSampleFonts() async {
  TestWidgetsFlutterBinding.ensureInitialized();
  Future<ByteData> file(String path) async => ByteData.sublistView(await File(path).readAsBytes());

  final roboto = FontLoader('Roboto');
  for (final weight in ['regular', 'italic', 'medium', 'bold']) {
    roboto.addFont(file('assets/fonts/Roboto-$weight.ttf'));
  }
  await roboto.load();

  final flutterRoot = Platform.environment['FLUTTER_ROOT'];
  if (flutterRoot != null) {
    final icons = '$flutterRoot/bin/cache/artifacts/material_fonts/MaterialIcons-Regular.otf';
    if (File(icons).existsSync()) {
      final loader = FontLoader('MaterialIcons')..addFont(file(icons));
      await loader.load();
    }
  }

  const monospaceFaces = [
    r'C:\Windows\Fonts\consola.ttf',
    '/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf',
    '/System/Library/Fonts/Menlo.ttc',
  ];
  final mono = monospaceFaces.where((path) => File(path).existsSync()).firstOrNull;
  final monospace = FontLoader('monospace')..addFont(file(mono ?? 'assets/fonts/Roboto-regular.ttf'));
  await monospace.load();
}

/// Every test of the run draws its goldens into [samplesDir] with real fonts, when that is set.
Future<void> renderSamplesIfAsked(FutureOr<void> Function() testMain) async {
  final dir = samplesDir;
  if (dir != null) {
    await loadSampleFonts();
    // A test file picks its own tolerant comparator in setUpAll; this runs after it, before each test.
    setUp(() => goldenFileComparator = SampleWriter(dir));
  }
  await testMain();
}
