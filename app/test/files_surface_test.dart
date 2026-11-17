// The Files surface: a workspace browsed and previewed on the host its session runs on (two hosts, two disks), a preview that is
// bounded and read-only, hidden characters shown as what they are, the host's own words for a refusal, and `fs/read` over the control
// channel.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart' show HostLink;
import 'package:canoa/worktree/worktree.dart' show SessionWorktree;
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/open_host.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A host's disk, as the host would answer for it: the folder it was made over is the only place it shows.
final class _Disk implements WorkspaceFiles {
  _Disk(this.home, {this.refuse});

  final Directory home;

  /// What `fs/read` answers with a refusal instead, by file name.
  final Map<String, String>? refuse;
  final reads = <String>[];
  final lists = <String>[];

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async {
    lists.add(path ?? '');
    final dir = Directory(path ?? home.path);
    if (!dir.existsSync()) throw const ControlFailed(-32003, 'no such folder');
    final entries = [
      for (final each in dir.listSync()..sort((a, b) => a.path.compareTo(b.path)))
        if (each is Directory || files)
          HostFolderEntry(name: each.uri.pathSegments.where((part) => part.isNotEmpty).last, path: each.path, isFolder: each is Directory),
    ]..sort((a, b) => a.isFolder == b.isFolder ? 0 : (a.isFolder ? -1 : 1));
    final parent = dir.parent.path;
    return HostFolderListing(path: dir.path, parent: parent == dir.path ? null : parent, entries: entries, more: false);
  }

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    reads.add(path);
    final name = path.split(RegExp(r'[\\/]')).last;
    if (refuse?[name] case final String why) throw ControlFailed(-32002, why);
    final all = File(path).readAsBytesSync();
    final limit = maxBytes ?? 256 * 1024;
    final bytes = all.length > limit ? all.sublist(0, limit) : all;
    final image = bytes.length > 4 && bytes[0] == 0x89 && bytes[1] == 0x50;
    final text = !image && !bytes.contains(0);
    final cut = all.length > bytes.length;
    return HostFileRead(
      path: path,
      name: name,
      size: all.length,
      kind: image ? 'image' : (text ? 'text' : 'binary'),
      truncated: cut,
      bytes: (text || (image && !cut)) ? Uint8List.fromList(bytes) : null,
    );
  }
}

Directory _home(String name) {
  final dir = Directory.systemTemp.createTempSync('canoa-files-$name-');
  addTearDown(() => dir.deleteSync(recursive: true));
  return dir;
}

/// A one-pixel PNG.
final _png = base64Decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==');

/// The signature and header of a PNG that claims [width] by [height] pixels and holds none: what a few dozen bytes can declare.
Uint8List _pngClaiming(int width, int height) {
  int crc(List<int> bytes) {
    var value = 0xffffffff;
    for (final byte in bytes) {
      value ^= byte;
      for (var bit = 0; bit < 8; bit++) {
        value = (value & 1) != 0 ? (value >> 1) ^ 0xedb88320 : value >> 1;
      }
    }
    return value ^ 0xffffffff;
  }

  List<int> four(int value) => [(value >> 24) & 255, (value >> 16) & 255, (value >> 8) & 255, value & 255];
  final header = [...'IHDR'.codeUnits, ...four(width), ...four(height), 8, 2, 0, 0, 0];
  final idat = [...'IDAT'.codeUnits, 0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
  return Uint8List.fromList([
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
    ...four(13),
    ...header,
    ...four(crc(header)),
    ...four(idat.length - 4),
    ...idat,
    ...four(crc(idat)),
    ...four(0),
    ...'IEND'.codeUnits,
    ...four(crc('IEND'.codeUnits)),
  ]);
}

Finder _key(String name) => find.byKey(ValueKey(name));

Future<void> _show(WidgetTester tester, Widget child) async {
  await tester.binding.setSurfaceSize(const Size(420, 700));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(
    MaterialApp(
      theme: themeFor(LookId.canoa, Brightness.dark),
      home: Scaffold(body: child),
    ),
  );
  await tester.pumpAndSettle();
}

ChatSession _session(SessionStore store, String title, String? cwd) =>
    store.addProviderSession(title: title, agent: 'Claude Code', peerId: 'local:claude', remoteSessionId: 'r-$title', cwd: cwd);

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/files_surface_test.dart')));

  testWidgets('a workspace is listed folders first and a folder is entered and left, and left again above the workspace', (tester) async {
    final home = _home('browse')..createSync();
    Directory('${home.path}/src').createSync();
    Directory('${home.path}/src/deep').createSync();
    File('${home.path}/README.md').writeAsStringSync('# readme');
    File('${home.path}/src/main.dart').writeAsStringSync('void main() {}');
    final disk = _Disk(home);
    await _show(tester, FilesPanel(files: disk, root: home.path));

    expect(find.text('Workspace'), findsOneWidget);
    final names = [for (final tile in tester.widgetList<ListTile>(find.byType(ListTile))) ((tile.title! as Text).data)];
    expect(names, ['src', 'README.md'], reason: 'folders before files');
    expect(tester.widget<IconButton>(_key('files up')).onPressed, isNotNull, reason: 'the host lets this key browse above the workspace');

    await tester.tap(_key('files entry src'));
    await tester.pumpAndSettle();
    expect(find.text('src'), findsOneWidget, reason: 'the place is shown relative to the workspace');
    expect(_key('files entry main.dart'), findsOneWidget);
    await tester.tap(_key('files entry deep'));
    await tester.pumpAndSettle();
    expect(_key('files entry main.dart'), findsNothing);
    expect(find.text('Nothing to show in this folder.'), findsOneWidget);
    await tester.tap(_key('files up'));
    await tester.pumpAndSettle();
    await tester.tap(_key('files up'));
    await tester.pumpAndSettle();
    expect(find.text('Workspace'), findsOneWidget);
    await tester.tap(_key('files up'));
    await tester.pumpAndSettle();
    expect(find.text(home.parent.path), findsOneWidget, reason: 'above the workspace the whole path is shown');
    expect(disk.reads, isEmpty, reason: 'browsing reads no file');
  });

  testWidgets('the surface as it is drawn: a folder, and a file previewed', (tester) async {
    final home = _home('scene')..createSync();
    Directory('${home.path}/src').createSync();
    Directory('${home.path}/docs').createSync();
    File('${home.path}/README.md').writeAsStringSync('# Canoa\n');
    File('${home.path}/src/main.dart').writeAsStringSync("import 'dart:io';\n\nvoid main() {\n  stdout.writeln('hello');\n}\n");
    File('${home.path}/src/logo.png').writeAsBytesSync(_png);
    await _show(
      tester,
      RepaintBoundary(
        child: FilesPanel(files: _Disk(home), root: home.path),
      ),
    );
    await expectLater(find.byType(FilesPanel), matchesGoldenFile('goldens/files_surface_list.png'));
    await tester.tap(_key('files entry src'));
    await tester.pumpAndSettle();
    await tester.tap(_key('files entry main.dart'));
    await tester.pumpAndSettle();
    await expectLater(find.byType(FilesPanel), matchesGoldenFile('goldens/files_surface_preview.png'));
  });

  testWidgets('a text file is previewed as read-only text with the hidden characters it holds shown as what they are', (tester) async {
    final home = _home('text')..createSync();
    File('${home.path}/a.txt').writeAsStringSync('hello\nreal\u202Etxt.exe\u200B end\n');
    final disk = _Disk(home);
    await _show(tester, FilesPanel(files: disk, root: home.path));
    await tester.tap(_key('files entry a.txt'));
    await tester.pumpAndSettle();
    final shown = [for (var at = 0; at < 3; at++) tester.widget<SelectableText>(_key('files line $at')).data!].join('\n');
    expect(shown, 'hello\nreal<U+202E>txt.exe<U+200B> end\n');
    expect(find.byType(TextField), findsNothing, reason: 'previewing never implies editing');
    expect(find.byType(EditableText), findsNWidgets(3), reason: 'only the selectable lines themselves');
    expect(disk.reads, ['${home.path}${Platform.pathSeparator}a.txt']);
    await tester.tap(_key('files back'));
    await tester.pumpAndSettle();
    expect(_key('files entry a.txt'), findsOneWidget);
  });

  testWidgets('a long file says how much of it is shown, an image is drawn, and a binary file is not previewed', (tester) async {
    final home = _home('kinds')..createSync();
    File('${home.path}/big.txt').writeAsStringSync('x' * (300 * 1024));
    File('${home.path}/dot.png').writeAsBytesSync(_png);
    File('${home.path}/blob.bin').writeAsBytesSync([1, 2, 0, 3]);
    await _show(tester, FilesPanel(files: _Disk(home), root: home.path));

    await tester.tap(_key('files entry big.txt'));
    await tester.pumpAndSettle();
    expect(find.text('Showing the first 256 KiB of 300 KiB.'), findsOneWidget);
    await tester.tap(_key('files back'));
    await tester.pumpAndSettle();

    await tester.tap(_key('files entry dot.png'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 300)));
    await tester.pumpAndSettle();
    expect(_key('files image'), findsOneWidget);
    await tester.tap(_key('files back'));
    await tester.pumpAndSettle();

    await tester.tap(_key('files entry blob.bin'));
    await tester.pumpAndSettle();
    expect(find.text('This file cannot be previewed (4 bytes).'), findsOneWidget);
  });

  testWidgets('a file asked for is shown in its folder and previewed, again for each new request, and a missing one says so', (tester) async {
    final home = _home('reveal-file')..createSync();
    Directory('${home.path}/src').createSync();
    File('${home.path}/src/main.dart').writeAsStringSync('void main() {}');
    File('${home.path}/README.md').writeAsStringSync('# readme');
    final disk = _Disk(home);
    await _show(tester, FilesPanel(files: disk, root: home.path, reveal: FileReveal('${home.path}/src/main.dart')));
    expect(_key('files name'), findsOneWidget);
    expect(find.text('main.dart'), findsOneWidget);
    expect(find.text('void main() {}'), findsOneWidget);

    await _show(tester, FilesPanel(files: disk, root: home.path, reveal: FileReveal('${home.path}/README.md')));
    expect(find.text('README.md'), findsOneWidget);
    expect(find.text('# readme'), findsOneWidget);

    await _show(tester, FilesPanel(files: disk, root: home.path, reveal: FileReveal('${home.path}/src/gone.dart')));
    expect(_key('files problem'), findsOneWidget);
    expect(find.textContaining('not there any more'), findsOneWidget);
  });

  testWidgets('a refusal is the host\'s own words, on the file and on the folder', (tester) async {
    final home = _home('refused')..createSync();
    File('${home.path}/.env').writeAsStringSync('TOKEN=1');
    final disk = _Disk(home, refuse: {'.env': 'that path is one the person protects'});
    await _show(tester, FilesPanel(files: disk, root: home.path));
    await tester.tap(_key('files entry .env'));
    await tester.pumpAndSettle();
    expect(find.text('that path is one the person protects'), findsOneWidget);
    await tester.tap(_key('files back'));
    await tester.pumpAndSettle();
    expect(_key('files entry .env'), findsOneWidget, reason: 'the folder is still there to go on from');

    await _show(tester, FilesPanel(files: disk, root: '${home.path}/not-there'));
    expect(find.text('no such folder'), findsOneWidget);
  });

  testWidgets('Reveal is offered for a workspace on this computer, runs through the file manager, and is not offered elsewhere', (
    tester,
  ) async {
    final home = _home('reveal')..createSync();
    File('${home.path}/a.txt').writeAsStringSync('a');
    final host = OpenHost();
    await _show(tester, FilesPanel(files: _Disk(home), root: home.path, opener: await host.opener()));
    expect(_key('files action reveal'), findsOneWidget, reason: 'the folder can be revealed');
    await tester.tap(_key('files entry a.txt'));
    await tester.pumpAndSettle();
    await tester.tap(_key('files action reveal'));
    await tester.pumpAndSettle();
    expect(host.started.single, ('${home.path}${Platform.pathSeparator}a.txt', 'reveal'));

    // On another host a path means nothing here: no action.
    await _show(tester, FilesPanel(files: _Disk(home), root: home.path));
    expect(_key('files action reveal'), findsNothing);
    expect(_key('files action vscode'), findsNothing);
  });

  testWidgets('each session shows the disk of the host it runs on, and a session with no folder or no reachable host has no Files', (
    tester,
  ) async {
    final here = _home('here')..createSync();
    final there = _home('there')..createSync();
    File('${here.path}/mine.txt').writeAsStringSync('on this computer');
    File('${there.path}/theirs.txt').writeAsStringSync('on another host');
    final hereDisk = _Disk(here), thereDisk = _Disk(there);
    final store = SessionStore();
    addTearDown(store.dispose);
    final onHere = _session(store, 'One', here.path);
    final onThere = _session(store, 'Two', there.path);
    final unreachable = _session(store, 'Three', here.path);
    final noFolder = _session(store, 'Four', null);
    final surface = FilesSurface(
      (session) => switch (session.id) {
        final id when id == onHere.id => hereDisk,
        final id when id == onThere.id => thereDisk,
        _ => null,
      },
    );

    expect(surface.available(onHere), isTrue, reason: 'it opens before any agent has started');
    expect(surface.available(onThere), isTrue);
    expect(surface.available(unreachable), isFalse);
    expect(surface.available(noFolder), isFalse);
    expect(surface.placements, [SurfaceSlot.right]);

    final controller = SurfaceController('files');
    for (final (session, present, absent) in [(onHere, 'mine.txt', 'theirs.txt'), (onThere, 'theirs.txt', 'mine.txt')]) {
      await tester.pumpWidget(const SizedBox());
      await _show(tester, Builder(builder: (context) => surface.build(context, session, controller)));
      expect(_key('files entry $present'), findsOneWidget);
      expect(_key('files entry $absent'), findsNothing);
    }
    expect(hereDisk.lists, [here.path]);
    expect(thereDisk.lists, [there.path]);
  });

  test('a worktree is the workspace when the session has one', () {
    final store = SessionStore();
    addTearDown(store.dispose);
    final session = store.addProviderSession(
      title: 'W',
      agent: 'Claude Code',
      peerId: 'local:claude',
      remoteSessionId: 'r-w',
      cwd: '/repo',
      worktree: const SessionWorktree(path: '/repo-wt/feature', branch: 'feature', repo: '/repo'),
    );
    expect(FilesSurface.workspaceOf(session), '/repo-wt/feature');
  });

  test('what hides in text is shown by its code point, and a line break or tab is left', () {
    expect(visibleText('a\tb\nc'), 'a\tb\nc');
    expect(visibleText('a\u0007b\u00ADc\u202Ed\u2066e\u200Bf\uFEFFg'), 'a<U+0007>b<U+00AD>c<U+202E>d<U+2066>e<U+200B>f<U+FEFF>g');
    expect(visibleText('x\u{E0041}y'), 'x<U+E0041>y');
    expect(visibleText('plain \u00E9\u4E2D\u{1F600}'), 'plain \u00E9\u4E2D\u{1F600}');
  });

  test('the blank-drawing characters are shown too, and a long stack of combining marks is cut', () {
    String chars(List<int> codes) => String.fromCharCodes(codes);
    for (final code in [0x115f, 0x1160, 0x3164, 0xffa0, 0x034f, 0x17b4, 0x17b5, 0x1d173, 0x1d17a, 0xfff9, 0xfffb]) {
      final hex = code.toRadixString(16).toUpperCase().padLeft(4, '0');
      expect(visibleText('a${chars([code])}b'), 'a<U+$hex>b', reason: hex);
    }
    // A few marks are a letter with accents; more is a stack.
    final marks = chars([0x301, 0x302, 0x303, 0x304, 0x305, 0x306]);
    expect(visibleText('e$marks'), 'e${chars([0x301, 0x302, 0x303])}<U+0304><U+0305><U+0306>');
    expect(
      visibleText('e${chars([0x301])}x${chars([0x301])}'),
      'e${chars([0x301])}x${chars([0x301])}',
      reason: 'a run starts again after a letter',
    );
    expect(visibleText('a\r\nb'), 'a\r\nb', reason: 'a carriage return is a line end, not hidden');
  });

  test('a file is shown a line at a time, a long line in pieces, and a carriage return dropped', () {
    expect(previewLines('a\r\nb\n'), ['a', 'b', '']);
    final long = 'x' * (lineLength * 2 + 5);
    final pieces = previewLines(long);
    expect(pieces.map((piece) => piece.length), [lineLength, lineLength, 5]);
    // A pair of surrogates is not cut in two.
    final emoji = '\u{1F600}' * lineLength;
    expect(previewLines(emoji).every((piece) => !piece.startsWith(String.fromCharCode(0xde00))), isTrue);
    expect(previewLines(emoji).join(), emoji);
  });

  testWidgets('a long file is built a line at a time, not as one block of text', (tester) async {
    final home = _home('lines')..createSync();
    File('${home.path}/long.txt').writeAsStringSync([for (var at = 0; at < 20000; at++) 'line $at'].join('\n'));
    await _show(tester, FilesPanel(files: _Disk(home), root: home.path));
    await tester.tap(_key('files entry long.txt'));
    await tester.pumpAndSettle();
    final built = tester.widgetList(find.byType(SelectableText)).length;
    expect(built, lessThan(60), reason: 'only the lines on the screen exist');
    expect(_key('files line 0'), findsOneWidget);
    expect(_key('files line 19999'), findsNothing);
  });

  test('the declared size is read from the header of each kind of image, and from nothing else', () {
    List<int> le(int value, int bytes) => [for (var i = 0; i < bytes; i++) (value >> (8 * i)) & 255];
    final png = declaredImageSize(_pngClaiming(1234, 567));
    expect((png?.width, png?.height), (1234, 567));
    expect(declaredImageSize(_png)?.width, 1);
    final gif = Uint8List.fromList([...'GIF89a'.codeUnits, ...le(320, 2), ...le(200, 2), 0, 0, 0]);
    expect((declaredImageSize(gif)?.width, declaredImageSize(gif)?.height), (320, 200));
    // A JPEG with a comment segment before the frame header.
    final jpeg = Uint8List.fromList([
      0xff, 0xd8, 0xff, 0xfe, 0x00, 0x04, 0x61, 0x62, //
      0xff, 0xc0, 0x00, 0x11, 8, 0x03, 0xe8, 0x07, 0xd0, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1,
    ]);
    expect((declaredImageSize(jpeg)?.width, declaredImageSize(jpeg)?.height), (2000, 1000));
    List<int> riff(String kind, List<int> body) => [
      ...'RIFF'.codeUnits,
      ...le(4 + 8 + body.length, 4),
      ...'WEBP'.codeUnits,
      ...kind.codeUnits,
      ...le(body.length, 4),
      ...body,
    ];
    final lossy = Uint8List.fromList(riff('VP8 ', [0, 0, 0, 0x9d, 0x01, 0x2a, ...le(640, 2), ...le(480, 2)]));
    expect((declaredImageSize(lossy)?.width, declaredImageSize(lossy)?.height), (640, 480));
    final lossless = Uint8List.fromList(riff('VP8L', [0x2f, ...le((99) | (49 << 14), 4)]));
    expect((declaredImageSize(lossless)?.width, declaredImageSize(lossless)?.height), (100, 50));
    final extended = Uint8List.fromList(riff('VP8X', [0, 0, 0, 0, ...le(16383, 3), ...le(8191, 3)]));
    expect((declaredImageSize(extended)?.width, declaredImageSize(extended)?.height), (16384, 8192));
    // Cut short, or not an image at all.
    expect(declaredImageSize(Uint8List.fromList(_png.sublist(0, 20))), isNull);
    expect(declaredImageSize(Uint8List.fromList(utf8.encode('hello, not an image'))), isNull);
    expect(declaredImageSize(Uint8List(0)), isNull);
  });
  testWidgets('an image that declares more pixels than the preview decodes is not decoded, and a small one is bounded', (tester) async {
    final home = _home('huge')..createSync();
    File('${home.path}/huge.png').writeAsBytesSync(_pngClaiming(40000, 40000));
    File('${home.path}/dot.png').writeAsBytesSync(_png);
    await _show(tester, FilesPanel(files: _Disk(home), root: home.path));
    await tester.tap(_key('files entry huge.png'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 300)));
    await tester.pumpAndSettle();
    expect(find.byType(Image), findsNothing, reason: 'nothing was handed to the decoder');
    expect(find.textContaining('too large to draw (40000 by 40000 pixels)'), findsOneWidget);
    await tester.tap(_key('files back'));
    await tester.pumpAndSettle();
    await tester.tap(_key('files entry dot.png'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 300)));
    await tester.pumpAndSettle();
    expect(find.byType(Image), findsOneWidget);
  });

  group('fs/read over the control channel', () {
    test('asks the host for the file and the limit, and reads what it answers', () async {
      final asked = <Map<String, Object?>>[];
      final link = _Link((method, params) {
        asked.add({'method': method, ...params});
        return {
          'path': '/w/a.txt',
          'name': 'a.txt',
          'size': 9,
          'kind': 'text',
          'mime': null,
          'truncated': true,
          'data': base64Encode(utf8.encode('hello')),
        };
      });
      final files = HostFiles(HostControl.dialing(() async => link));
      final read = await files.readFile('/w/a.txt', maxBytes: 5);
      expect(asked.single, {'method': 'fs/read', 'path': '/w/a.txt', 'max_bytes': 5});
      expect([read.kind, read.size, read.truncated, utf8.decode(read.bytes!)], ['text', 9, true, 'hello']);
      await files.readFile('/w/a.txt');
      expect(asked.last.containsKey('max_bytes'), isFalse);
    });

    test('a file the host sends no bytes for has none, and its refusal is the host\'s', () async {
      final link = _Link(
        (method, params) => params['path'] == '/w/no'
            ? throw StateError('that path is one the person protects')
            : {'path': '/w/b', 'name': 'b', 'size': 4, 'kind': 'binary', 'truncated': false, 'data': null},
      );
      final files = HostFiles(HostControl.dialing(() async => link));
      expect((await files.readFile('/w/b')).bytes, isNull);
      await expectLater(files.readFile('/w/no'), throwsA(isA<ControlFailed>().having((e) => e.message, 'message', contains('protects'))));
    });
  });
}

/// A control channel whose host answers each request with [answer].
final class _Link implements HostLink {
  _Link(this.answer);

  final Object? Function(String method, Map<String, Object?> params) answer;
  final _out = StreamController<String>();

  @override
  Stream<String> get messages => _out.stream;

  @override
  Future<void> send(String message) async {
    final request = jsonDecode(message) as Map<String, Object?>;
    try {
      final result = answer(request['method']! as String, request['params']! as Map<String, Object?>);
      _out.add(jsonEncode({'jsonrpc': '2.0', 'id': request['id'], 'result': result}));
    } on Object catch (error) {
      _out.add(
        jsonEncode({
          'jsonrpc': '2.0',
          'id': request['id'],
          'error': {'code': -32002, 'message': '$error'},
        }),
      );
    }
  }

  @override
  Future<void> close() => _out.close();
}
