import 'package:flutter/foundation.dart';

/// How a theme draws one named symbol instead of the base one: a glyph of text, or another base symbol.
@immutable
final class SymbolSpec {
  const SymbolSpec.glyph(String this.glyph) : like = null;
  const SymbolSpec.like(String this.like) : glyph = null;

  /// Text drawn where the symbol goes, such as `>` or `$`.
  final String? glyph;

  /// The name of a base symbol to draw instead.
  final String? like;

  static SymbolSpec? parse(Object? value) {
    if (value is String && value.isNotEmpty) return SymbolSpec.glyph(value);
    if (value is! Map) return null;
    final glyph = value['glyph'], like = value['like'];
    if (glyph is String && glyph.isNotEmpty) return SymbolSpec.glyph(glyph);
    if (like is String && like.isNotEmpty) return SymbolSpec.like(like);
    return null;
  }

  @override
  bool operator ==(Object other) => other is SymbolSpec && other.glyph == glyph && other.like == like;

  @override
  int get hashCode => Object.hash(glyph, like);
}

/// The words and symbols a theme sets in place of the base ones: any subset, in any languages.
///
/// A theme that sets none draws the base text; one that sets some takes the rest from the base, in the language the person reads.
@immutable
final class TextPack {
  const TextPack({this.strings = const {}, this.symbols = const {}, this.defaultLanguage});

  /// `language -> key -> text`.
  final Map<String, Map<String, String>> strings;

  /// `symbol name -> how the theme draws it`.
  final Map<String, SymbolSpec> symbols;

  /// The language the theme's own text is written in, tried after the person's language and before the base text.
  final String? defaultLanguage;

  /// The languages the pack has text in.
  Iterable<String> get languages => strings.keys;

  /// A pack from the JSON a theme's document carries as `text`: `{"default_language": "en", "strings": {"en": {key: text}},
  /// "symbols": {name: glyph | {"glyph": text} | {"like": base symbol}}}`. Anything else in it is ignored; null when nothing is left.
  static TextPack? parse(Object? json) {
    if (json is! Map) return null;
    final strings = <String, Map<String, String>>{};
    final rawStrings = json['strings'];
    if (rawStrings is Map) {
      for (final MapEntry(key: language, value: table) in rawStrings.entries) {
        if (language is! String || table is! Map) continue;
        final texts = <String, String>{
          for (final MapEntry(key: key, value: text) in table.entries)
            if (key is String && text is String) key: text,
        };
        if (texts.isNotEmpty) strings[language] = texts;
      }
    }
    final symbols = <String, SymbolSpec>{};
    final rawSymbols = json['symbols'];
    if (rawSymbols is Map) {
      for (final MapEntry(key: name, value: value) in rawSymbols.entries) {
        final spec = SymbolSpec.parse(value);
        if (name is String && spec != null) symbols[name] = spec;
      }
    }
    if (strings.isEmpty && symbols.isEmpty) return null;
    final defaultLanguage = json['default_language'];
    return TextPack(strings: strings, symbols: symbols, defaultLanguage: defaultLanguage is String ? defaultLanguage : null);
  }
}
