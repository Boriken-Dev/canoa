// How each kind of attachment reaches an agent, by what it takes and where it runs.
import 'dart:io';

import 'package:canoa/acp/local_agent.dart' show readLocalFile;
import 'package:canoa/chat/attachments.dart';
import 'package:flutter_test/flutter_test.dart';

const _all = PromptTakes(images: true, files: true);
const _none = PromptTakes(images: false, files: false);
const _elsewhere = PromptTakes(images: true, files: true, links: false);

AttachmentRoute _route(Attachment attachment, PromptTakes takes) => routeOf(attachment, takes, embedLimit: 100, agent: 'It').$1;

void main() {
  final image = Attachment.image('a.png', 'image/png', [1], path: '/w/a.png');
  final pasted = Attachment.image('Pasted image 1.png', 'image/png', [1]);
  final small = Attachment.file('notes.txt', '/w/notes.txt', 10);
  final large = Attachment.file('big.log', '/w/big.log', 1000);
  final text = Attachment.text('Pasted text, 1 line', 'x');
  final folder = Attachment.folder('w', '/w');

  test('an agent that takes everything is given each as it is, and a large file or a folder by a link', () {
    expect([for (final each in [image, small, large, text, folder]) _route(each, _all)],
        [AttachmentRoute.image, AttachmentRoute.embedded, AttachmentRoute.link, AttachmentRoute.embedded, AttachmentRoute.link]);
  });

  test('an agent that takes nothing is given links where there is a file, and nothing else', () {
    expect([for (final each in [image, pasted, small, text, folder]) _route(each, _none)],
        [AttachmentRoute.link, AttachmentRoute.refused, AttachmentRoute.link, AttachmentRoute.refused, AttachmentRoute.link]);
    expect(routeOf(pasted, _none, embedLimit: 100, agent: 'It').$2, 'It does not take images');
  });

  test('an agent elsewhere is never given a link to this computer\'s files', () {
    expect([for (final each in [image, small, large, text, folder]) _route(each, _elsewhere)],
        [AttachmentRoute.image, AttachmentRoute.embedded, AttachmentRoute.refused, AttachmentRoute.embedded, AttachmentRoute.refused]);
    expect(routeOf(large, _elsewhere, embedLimit: 100, agent: 'It').$2, 'It runs elsewhere, and cannot read this computer\'s files by a link');
  });

  test('a file that is not text goes as a link, and to an agent elsewhere not at all', () async {
    Future<List<int>> read(String path, {int? maxBytes}) async => [0xff, 0xfe];
    final linked = await toAcp(small, _all, embedLimit: 100, read: read);
    expect(linked.isLink, isTrue);
    await expectLater(toAcp(small, _elsewhere, embedLimit: 100, read: read), throwsStateError);
  });

  test('a file that grew past the limit since it was added goes as a link', () async {
    Future<List<int>> read(String path, {int? maxBytes}) async => List.filled(200, 97);
    final grown = await toAcp(small, _all, embedLimit: 100, read: read);
    expect(grown.isLink, isTrue);
    await expectLater(toAcp(small, _elsewhere, embedLimit: 100, read: read), throwsStateError);
  });

  test('a file is read at most one byte past the limit, and a small file that grew in the draft goes as a link', () async {
    final dir = await Directory.systemTemp.createTemp('canoa_attach');
    try {
      final file = File('${dir.path}/notes.txt');
      await file.writeAsString('small');
      final attached = Attachment.file('notes.txt', file.path, 5);
      await file.writeAsBytes(List.filled(5000, 97));
      var asked = -1;
      var got = -1;
      Future<List<int>> read(String path, {int? maxBytes}) async {
        asked = maxBytes ?? -1;
        final bytes = await readLocalFile(path, maxBytes: maxBytes);
        got = bytes.length;
        return bytes;
      }

      final grown = await toAcp(attached, _all, embedLimit: 100, read: read);
      expect(asked, 101);
      expect(got, 101);
      expect(grown.isLink, isTrue);
      await expectLater(toAcp(attached, _elsewhere, embedLimit: 100, read: read), throwsStateError);
      expect((await readLocalFile(file.path)).length, 5000);
    } finally {
      await dir.delete(recursive: true);
    }
  });

  test('sizes read as a person reads them', () {
    expect([sizeLabel(512), sizeLabel(1536), sizeLabel(20 * 1024), sizeLabel(3 * 1024 * 1024)], ['512 B', '1.5 KB', '20 KB', '3.0 MB']);
  });
}
