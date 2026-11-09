// What an overlay's text can and cannot make of a layer: Canoa's use of the AOS reader, not the reader itself.
import 'dart:convert';

import 'package:canoa/profile/aos_support.dart';
import 'package:canoa/profile/overlay_layer.dart';
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
}
