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

  test('a file named by an @ goes embedded when the agent takes files and it is small, else as a link; a folder is always a link', () {
    final mentioned = Attachment.mention('notes.txt', '/w/notes.txt', 10);
    final mentionedLarge = Attachment.mention('big.log', '/w/big.log', 1000);
    final mentionedFolder = Attachment.mention('src/', '/w/src', 0, folder: true);
    expect([for (final each in [mentioned, mentionedLarge, mentionedFolder]) _route(each, _all)], [AttachmentRoute.embedded, AttachmentRoute.link, AttachmentRoute.link]);
    expect([for (final each in [mentioned, mentionedLarge, mentionedFolder]) _route(each, const PromptTakes(images: true, files: false))], everyElement(AttachmentRoute.link));
    expect(_route(mentioned, _elsewhere), AttachmentRoute.embedded, reason: 'its contents need no link to this computer');
    expect(_route(mentionedLarge, _elsewhere), AttachmentRoute.refused);
  });
  test('a mention of another conversation or of a path on the agent\'s host survives being saved, and a conversation goes embedded or not at all', () {
    final chat = Attachment.conversation('Fix login', '# Fix login\n\nUser: hi');
    final far = Attachment.mention('src/a.dart', '/srv/app/src/a.dart', 5, onAgentHost: true);
    final back = [for (final each in [chat, far]) Attachment.fromJson(each.toJson())!];
    expect([for (final each in back) each.mention], [true, true]);
    expect(back[0].text, '# Fix login\n\nUser: hi');
    expect(back[1].onAgentHost, isTrue);
    expect(_route(back[0], _all), AttachmentRoute.embedded);
    expect(_route(back[0], _none), AttachmentRoute.refused, reason: 'it has no file to link to');
    expect(_route(back[1], _elsewhere), AttachmentRoute.link, reason: 'the agent runs where the file is');
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
