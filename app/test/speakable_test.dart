// What of a reply is read aloud: its prose, with no code block, image or markup, and the sentences it is read in as it streams.
import 'package:canoa/voice/speakable.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('spokenText', () {
    test('keeps the prose and takes the markup off', () {
      expect(spokenText('Use **bold** and _soft_ words, see [the docs](https://example.org/x) for `more`.'), 'Use bold and soft words, see the docs for more.');
    });

    test('a code block is not read, and its neighbours stay apart', () {
      expect(spokenText('Run this:\n\n```sh\nrm -rf build\ncargo test\n```\n\nThen it is done.'), 'Run this:\nThen it is done.');
    });

    test('an indented code block and an image are not read either', () {
      expect(spokenText('Look:\n\n    int x = 1;\n\n![a chart](chart.png)\n\nThat is all.'), 'Look:\nThat is all.');
    });

    test('headings and list items end as sentences, and a table is read row by row', () {
      expect(spokenText('# Plan\n\n- first step\n- second step\n\n| a | b |\n|---|---|\n| 1 | 2 |\n'), 'Plan.\nfirst step.\nsecond step.\na, b.\n1, 2.');
    });

    test('a long list with inline code reads the code as its text', () {
      expect(spokenText('1. Open `main.dart`\n2. Save it'), 'Open main.dart.\nSave it.');
    });

    test('a reply that is only code says nothing', () {
      expect(spokenText('```\nonly code\n```'), '');
    });
  });

  group('SentenceCutter', () {
    test('cuts a whole reply into its sentences', () {
      expect(sentencesOf('First one. Second one! Third one? Last'), ['First one.', 'Second one!', 'Third one?', 'Last.']);
    });

    test('a reply that grows gives each sentence once, when its end is known', () {
      final cutter = SentenceCutter();
      expect(cutter.feed('Hello the'), isEmpty);
      expect(cutter.feed('Hello there. How are'), ['Hello there.']);
      expect(cutter.feed('Hello there. How are you? Fi'), ['How are you?']);
      // The end of a text is not a boundary until something follows, or the reply is over.
      expect(cutter.feed('Hello there. How are you? Fine.'), isEmpty);
      expect(cutter.feed('Hello there. How are you? Fine.', last: true), ['Fine.']);
      expect(cutter.finished, isTrue);
      expect(cutter.feed('Hello there. How are you? Fine. More', last: true), isEmpty);
    });

    test('a code block that is still being written is not read, and what follows it is', () {
      final cutter = SentenceCutter();
      expect(cutter.feed('Here:\n\n```\nfinal x'), ['Here:']);
      expect(cutter.feed('Here:\n\n```\nfinal x = 1;\n```\n\nThat sets x.'), isEmpty);
      expect(cutter.feed('Here:\n\n```\nfinal x = 1;\n```\n\nThat sets x. Done', last: true), ['That sets x.', 'Done.']);
    });

    test('a number with a point is not a sentence end, and a very long run is cut at a space', () {
      expect(sentencesOf('It costs 3.5 dollars.'), ['It costs 3.5 dollars.']);
      final long = List.filled(80, 'word').join(' ');
      final parts = sentencesOf(long);
      expect(parts.length, greaterThan(1));
      expect(parts.every((part) => part.length <= 320), isTrue);
      expect(parts.join(' '), '$long.');
    });
  });
}
