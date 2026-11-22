// The tray's icon is the brand's mark drawn in the look's accent: rendered here for two accents, packed as the .ico Windows loads,
// and handed to the tray (with its words) again when the look changes.
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:canoa/brand.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell_io.dart';
import 'package:canoa/desktop/tray_icon.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/l10n/text_pack.dart';
import 'package:flutter/painting.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _teal = Color(0xFF3DAFA1), _red = Color(0xFFC0392B);

Future<List<int>> _solidPixels(Uint8List png) async {
  final codec = await ui.instantiateImageCodec(png);
  final image = (await codec.getNextFrame()).image;
  final data = (await image.toByteData())!.buffer.asUint8List();
  final colours = <int>{};
  for (var at = 0; at < data.length; at += 4) {
    if (data[at + 3] == 255) colours.add((data[at] << 16) | (data[at + 1] << 8) | data[at + 2]);
  }
  image.dispose();
  return colours.toList();
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('the mark is drawn in the accent it is given, and two accents give two icons', (tester) async {
    await tester.runAsync(() async {
      final one = await renderMarkPng(_teal, 32);
      final other = await renderMarkPng(_red, 32);
      expect(await _solidPixels(one), [0x3DAFA1]);
      expect(await _solidPixels(other), [0xC0392B]);
      expect(one, isNot(other));
    });
  });

  testWidgets('the Windows icon holds a PNG for each size, as an .ico', (tester) async {
    await tester.runAsync(() async {
      final ico = await renderTrayIcon(_red, ico: true);
      final header = ByteData.sublistView(ico);
      expect([header.getUint16(0, Endian.little), header.getUint16(2, Endian.little)], [0, 1]);
      expect(header.getUint16(4, Endian.little), trayIconSizes.length);
      for (final (index, size) in trayIconSizes.indexed) {
        final entry = 6 + 16 * index;
        expect(header.getUint8(entry), size);
        final length = header.getUint32(entry + 8, Endian.little), offset = header.getUint32(entry + 12, Endian.little);
        final png = Uint8List.sublistView(ico, offset, offset + length);
        expect(png.sublist(0, 4), [0x89, 0x50, 0x4E, 0x47]);
        expect(await _solidPixels(png), [0xC0392B], reason: 'the $size px entry');
      }
      // Linux and macOS take the one PNG.
      expect((await renderTrayIcon(_red, ico: false)).sublist(1, 4), [0x50, 0x4E, 0x47]);
    });
  });

  testWidgets('an icon file is kept for its colour and the other colours\' files go', (tester) async {
    await tester.runAsync(() async {
      final folder = await Directory.systemTemp.createTemp('tray_icon_test');
      addTearDown(() => folder.delete(recursive: true));
      final first = await writeTrayIcon(_teal, ico: true, folder: folder);
      final second = await writeTrayIcon(_red, ico: true, folder: folder);
      expect(first, isNot(second));
      expect(second, endsWith('tray-c0392b.ico'));
      expect(File(first).existsSync(), isFalse);
      expect(File(second).existsSync(), isTrue);
      expect(await writeTrayIcon(_red, ico: true, folder: folder), second);
      expect(Brand.id, isNotEmpty);
    });
  });

  group('the tray shell', () {
    final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    const window = MethodChannel('window_manager');
    const tray = MethodChannel('tray_manager');
    final icons = <String>[];
    final tooltips = <String>[];

    setUp(() {
      icons.clear();
      tooltips.clear();
      messenger.setMockMethodCallHandler(window, (call) async => call.method.startsWith('is') ? call.method == 'isVisible' : null);
      messenger.setMockMethodCallHandler(tray, (call) async {
        final arguments = call.arguments as Map?;
        if (call.method == 'setIcon') icons.add(arguments!['iconPath'] as String);
        if (call.method == 'setToolTip') tooltips.add(arguments!['toolTip'] as String);
        return null;
      });
    });

    tearDown(() {
      messenger.setMockMethodCallHandler(window, null);
      messenger.setMockMethodCallHandler(tray, null);
    });

    Future<TrayShell> started() async {
      final shell = TrayShell(trayAvailable: () async => true, desktop: true, iconFile: (accent) async => 'icon-${accent.toARGB32().toRadixString(16)}');
      await shell.initialize(const CanoaConfig(), onSettings: () {});
      addTearDown(shell.dispose);
      return shell;
    }

    List<String?> labels(TrayShell shell) => [for (final item in shell.menu!.items!) item.label];

    test('starts with the brand\'s accent, and a look\'s accent changes the icon once', () async {
      final shell = await started();
      expect(icons, hasLength(1));
      expect(icons.single, endsWith(Brand.accent.toARGB32().toRadixString(16)));
      await shell.applyAppearance(text: AppText.base, accent: _red);
      expect(icons, hasLength(2));
      expect(icons.last, endsWith('ffc0392b'));
      await shell.applyAppearance(text: AppText.base, accent: _red);
      expect(icons, hasLength(2));
    });

    test('the menu and tooltip take the look\'s words, in its language', () async {
      final shell = await started();
      expect(labels(shell).first, 'Open ${Brand.name}');
      const pack = TextPack(strings: {'es': {'tray.hosting': 'Anfitrión…'}, 'en': {'tray.settings': 'Preferences…'}}, defaultLanguage: 'en');
      await shell.applyAppearance(text: const AppText(language: 'es', pack: pack), accent: Brand.accent);
      expect(labels(shell), ['Abrir ${Brand.name}', 'Anfitrión…', 'Preferences…', 'Ocultar en la bandeja', null, 'Salir de ${Brand.name}']);
      await shell.showWaiting(2);
      expect(labels(shell).first, 'Abrir ${Brand.name} (2 esperan por ti)');
      expect(icons, hasLength(1), reason: 'words alone do not redraw the icon');
    });
  });
}
