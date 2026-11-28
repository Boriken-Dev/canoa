import 'dart:convert';
import 'dart:io';

import 'package:canoa/brand.dart';
import 'package:canoa/theme/canoa_mark.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

void main() {
  final file = jsonDecode(File('brand.json').readAsStringSync()) as Map<String, dynamic>;

  test('without definitions the brand is the brand file of the repository', () {
    expect(Brand.name, file['name']);
    expect(Brand.shortName, file['short_name']);
    expect(Brand.id, file['id']);
    expect(Brand.envPrefix, file['env_prefix']);
    expect(Brand.accentHex.toUpperCase(), (file['accent'] as String).toUpperCase());
    expect(Brand.markJson, isEmpty);
  });

  String hex(Color c) => '#${(c.toARGB32() & 0xFFFFFF).toRadixString(16).padLeft(6, '0')}'.toUpperCase();

  test('the brand file draws the mark in the default look\'s accent, light and dark', () {
    expect((file['accent'] as String).toUpperCase(), hex(lookFor(LookId.canoa, Brightness.light).accent));
    expect((file['accent_on_dark'] as String).toUpperCase(), hex(lookFor(LookId.canoa, Brightness.dark).accent));
  });

  test('an environment variable carries the brand prefix', () {
    expect(Brand.env('CONFIG'), '${file['env_prefix']}_CONFIG');
  });

  test('a brand mark replaces the paths, and Canoa\'s own are used without one', () {
    expect(MarkPaths.fromBrand(null).smallWeight, CanoaMarkPainter.smallWeight);
    final other = MarkPaths.fromBrand({'hull': 'M 0 0 L 10 0 L 10 10 Z', 'smallWeight': 5, 'spiralWidth': 2});
    expect(other.smallWeight, 5);
    expect(other.spiralWidth, 2);
    expect(other.hull.getBounds().width, 10);
    expect(other.paddle.getBounds().width, MarkPaths.fromBrand(null).paddle.getBounds().width);
  });
}
