// What goes with a message, from the composer: `+`, the clipboard (Ctrl+V), and files dropped on the conversation. What
// the agent receives is checked in the content blocks it is sent, and what it does not take is said before Send.
import 'dart:io';

import 'package:canoa/session_store.dart';
import 'package:desktop_drop/desktop_drop.dart';
import 'package:file_selector/file_selector.dart' show XFile;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A PNG of one pixel.
final Uint8List _png = Uint8List.fromList(const [
  0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01,
  0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41,
  0x54, 0x78, 0x9C, 0x63, 0x60, 0x60, 0xF8, 0x0F, 0x00, 0x01, 0x04, 0x01, 0x00, 0x5F, 0xE5, 0xC3, 0x4B, 0x00, 0x00, 0x00,
  0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
]);

/// Files on disk for the test, by name, with the sizes the app is told.
final class _Disk {
  _Disk() : dir = Directory.systemTemp.createTempSync('canoa-composer');
  final Directory dir;
  final sizes = <String, int>{};

  String file(String name, List<int> bytes) {
    final file = File('${dir.path}${Platform.pathSeparator}$name')..writeAsBytesSync(bytes);
    sizes[file.path] = bytes.length;
    return file.path;
  }
}

Future<void> _paste(WidgetTester tester) async {
  await tester.tap(find.byType(TextField).last);
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
}

Future<(SessionStore, OptionsAgent, FakeClipboard, _Disk)> _started(WidgetTester tester,
    {bool images = true, bool files = true, Size size = const Size(1280, 800)}) async {
  final disk = _Disk();
  addTearDown(() => disk.dir.deleteSync(recursive: true));
  final agent = OptionsAgent(images: images, files: files);
  final clipboard = FakeClipboard();
  final store = SessionStore(includePreview: false);
  final notes = disk.file('notes.txt', 'hello notes'.codeUnits);
  final big = disk.file('big.log', List.filled(2 * 1024 * 1024, 97));
  final shot = disk.file('shot.png', _png);
  await pumpApp(tester, store, startAgent: agent.start, clipboard: clipboard, files: disk.sizes, size: size,
      pickFiles: () async => [XFile(notes), XFile(big), XFile(shot)]);
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await send(tester, 'Say hello');
  return (store, agent, clipboard, disk);
}

List<dynamic> _lastPrompt(OptionsAgent agent) => agent.last('session/prompt')['params']['prompt'] as List<dynamic>;

/// Waits for [name] to be attached: an image is read from disk first, in several steps of real input and output.
Future<void> _attached(WidgetTester tester, String name) =>
    until(tester, () => find.byKey(ValueKey('attachment $name')).evaluate().isNotEmpty, '$name to be attached');

int _prompts(OptionsAgent agent) => agent.methods.where((method) => method == 'session/prompt').length;

/// Sends [text], and waits for the agent to have it: the files that go with it are read first.
Future<void> _sendWaiting(WidgetTester tester, OptionsAgent agent, String text) async {
  final before = _prompts(agent);
  await send(tester, text);
  await until(tester, () => _prompts(agent) > before, 'the agent to have the message');
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/composer_input_test.dart')));

  testWidgets('a pasted image becomes an image attachment and goes as ACP image content', (tester) async {
    final (store, agent, clipboard, _) = await _started(tester);
    clipboard.copiedImage = _png;
    await _paste(tester);
    expect(find.byKey(const ValueKey('attachment Pasted image 1.png')), findsOneWidget);
    expect(find.widgetWithText(TextField, ''), findsWidgets, reason: 'no text was pasted for it');
    await _sendWaiting(tester, agent, 'What is this?');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'image']);
    expect(prompt[1]['mimeType'], 'image/png');
    expect(store.selected!.attachments, isEmpty);
    await finish(tester);
  });

  testWidgets('copied files become attachments: small text embedded, a large file and a folder linked', (tester) async {
    final (_, agent, clipboard, disk) = await _started(tester);
    clipboard.copiedFiles = [for (final path in disk.sizes.keys) path];
    await _paste(tester);
    await _attached(tester, 'shot.png');
    expect(find.byKey(const ValueKey('attachment notes.txt')), findsOneWidget);
    expect(find.byKey(const ValueKey('attachment big.log')), findsOneWidget);
    expect(find.text('Larger than 1.0 MB: it goes as a link'), findsOneWidget, reason: 'said before Send');
    await _sendWaiting(tester, agent, 'Read these');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'resource', 'resource_link', 'image']);
    expect(prompt[1]['resource']['text'], 'hello notes');
    expect(prompt[2]['uri'], Uri.file(disk.sizes.keys.elementAt(1)).toString());
    expect(prompt[2]['size'], 2 * 1024 * 1024);
    await finish(tester);
  });

  testWidgets('plain text stays text, and a very long paste becomes an attachment of its own', (tester) async {
    final (_, agent, clipboard, _) = await _started(tester);
    clipboard.copiedText = 'short words';
    await _paste(tester);
    expect(find.widgetWithText(TextField, 'short words'), findsOneWidget);
    clipboard.copiedText = 'x' * 9000;
    await _paste(tester);
    expect(find.byKey(const ValueKey('attachment Pasted text, 1 line')), findsOneWidget);
    expect(find.widgetWithText(TextField, 'short words'), findsOneWidget, reason: 'the long one did not flood the box');
    final before = _prompts(agent);
    await tester.tap(find.byTooltip('Send'));
    await until(tester, () => _prompts(agent) > before, 'the agent to have the message');
    final prompt = _lastPrompt(agent);
    expect(prompt.map((block) => block['type']), ['text', 'resource']);
    expect((prompt[1]['resource']['text'] as String).length, 9000);
    await finish(tester);
  });

  testWidgets('a file dropped on the conversation is attached', (tester) async {
    final (_, agent, _, disk) = await _started(tester);
    final drop = tester.widget<DropTarget>(find.byType(DropTarget));
    drop.onDragDone!(DropDoneDetails(files: [DropItemFile(disk.sizes.keys.first)], localPosition: Offset.zero,
        globalPosition: Offset.zero));
    await settle(tester);
    expect(find.byKey(const ValueKey('attachment notes.txt')), findsOneWidget);
    await _sendWaiting(tester, agent, 'Dropped');
    expect(_lastPrompt(agent).map((block) => block['type']), ['text', 'resource']);
    await finish(tester);
  });

  testWidgets('+ adds files and a folder reference', (tester) async {
    final (_, agent, _, _) = await _started(tester);
    await tester.tap(find.byKey(const Key('composer-plus')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add files or images…'));
    await _attached(tester, 'shot.png');
    await tester.tap(find.byKey(const Key('composer-plus')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a folder reference…'));
    await settle(tester);
    expect(find.byKey(const ValueKey('attachment repo')), findsOneWidget);
    await _sendWaiting(tester, agent, 'All of it');
    expect(_lastPrompt(agent).map((block) => block['type']), ['text', 'resource', 'resource_link', 'image', 'resource_link']);
    expect(_lastPrompt(agent).last['uri'], Uri.directory('/work/repo').toString());
    await finish(tester);
  });

  testWidgets('an image for an agent that takes none is refused before Send, or linked where it has a file', (tester) async {
    final (store, agent, clipboard, disk) = await _started(tester, images: false);
    clipboard.copiedImage = _png;
    await _paste(tester);
    expect(find.text('Scripted does not take images.'), findsOneWidget);
    expect(store.selected!.attachments, isEmpty, reason: 'nothing kept that cannot go');
    clipboard
      ..copiedImage = null
      ..copiedFiles = [disk.sizes.keys.last];
    await _paste(tester);
    await _attached(tester, 'shot.png');
    expect(find.text('Scripted does not take images: it goes as a link to the file'), findsOneWidget);
    await _sendWaiting(tester, agent, 'Look');
    expect(_lastPrompt(agent).map((block) => block['type']), ['text', 'resource_link']);
    await finish(tester);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the attachments row draws at $label width', (tester) async {
      final (_, _, clipboard, disk) = await _started(tester, images: false, size: size);
      clipboard.copiedFiles = [for (final path in disk.sizes.keys) path];
      await _paste(tester);
      await _attached(tester, 'shot.png');
      clipboard
        ..copiedFiles = const []
        ..copiedText = 'y' * 9000;
      await _paste(tester);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/attachments_$label.png'));
      await finish(tester);
    });
  }
}
