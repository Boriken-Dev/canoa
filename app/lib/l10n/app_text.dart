import 'dart:ui' show PlatformDispatcher;

import 'package:flutter/material.dart';

import '../brand.dart';
import '../theme/look.dart';
import 'base_text.dart';
import 'text_pack.dart';

/// A symbol as it is drawn: an icon, or a glyph of text a theme put in its place.
sealed class AppSymbol {
  const AppSymbol();

  /// The symbol at [size], in [color] (the icon theme's when null).
  Widget build({double? size, Color? color});
}

final class IconSymbol extends AppSymbol {
  const IconSymbol(this.icon);
  final IconData icon;

  @override
  Widget build({double? size, Color? color}) => Icon(icon, size: size, color: color);
}

final class GlyphSymbol extends AppSymbol {
  const GlyphSymbol(this.glyph);
  final String glyph;

  @override
  Widget build({double? size, Color? color}) => Builder(builder: (context) {
    final style = DefaultTextStyle.of(context).style;
    final base = IconTheme.of(context);
    return SizedBox.square(
      dimension: size ?? base.size ?? 24,
      child: FittedBox(
        child: Text(glyph, style: style.copyWith(color: color ?? base.color ?? style.color, fontWeight: FontWeight.w600, height: 1)),
      ),
    );
  });
}

/// Every word and symbol the app shows, resolved for one language and one theme.
///
/// A key is looked up in the theme's text for the person's language, in the theme's own default language, in the base text for the
/// person's language and in the base English; a key none of them has reads as itself. A symbol is the theme's when it sets one, and
/// the base one otherwise. `{brand}` and `{short}` in a text are the build's names; other `{name}`s come from the call's arguments.
@immutable
final class AppText {
  const AppText({this.language = baseLanguage, this.pack});

  /// The language the person reads, as its code (`en`, `es`).
  final String language;

  /// The theme's own text, or null when the theme draws the base.
  final TextPack? pack;

  /// The base text in English, for a widget with no scope above it.
  static const AppText base = AppText();

  /// The text of [context]'s theme and language; the base English where there is none.
  static AppText of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<AppTextScope>()?.text ?? base;

  /// The text [key] names, with `{name}` in it replaced by [args]`[name]`.
  String call(String key, [Map<String, Object?> args = const {}]) => _fill(_find(key) ?? key, args);

  /// The text [key] names, or null when neither the theme nor the base has one: for a name that may come from elsewhere, such as a
  /// section a provider adds, which is called what it says when nothing here does.
  String? maybe(String key, [Map<String, Object?> args = const {}]) {
    final raw = _find(key);
    return raw == null ? null : _fill(raw, args);
  }

  String? _find(String key) {
    final theme = pack?.strings;
    final themeDefault = pack?.defaultLanguage;
    return theme?[language]?[key] ??
        (themeDefault == null ? null : theme?[themeDefault]?[key]) ??
        baseStrings[language]?[key] ??
        baseStrings[baseLanguage]![key];
  }

  static String _fill(String raw, Map<String, Object?> args) {
    var text = raw.replaceAll('{brand}', Brand.name).replaceAll('{short}', Brand.shortName);
    for (final MapEntry(key: name, value: value) in args.entries) {
      text = text.replaceAll('{$name}', '$value');
    }
    return text;
  }

  /// How the symbol [name] is drawn: the theme's, or the base one (a blank square for a name nobody knows).
  AppSymbol symbol(String name) {
    final spec = pack?.symbols[name];
    if (spec?.glyph != null) return GlyphSymbol(spec!.glyph!);
    // A theme may point a symbol at another base symbol, never at one of its own.
    return IconSymbol(baseSymbols[spec?.like] ?? baseSymbols[name] ?? Icons.crop_square);
  }

  /// The languages the person can choose: the base's and the theme's, each by its code.
  List<String> get languages => {...baseStrings.keys, ...?pack?.languages}.toList()..sort();

  /// What a language calls itself, from its own bundle (its code where no bundle names it).
  String languageName(String code) => pack?.strings[code]?['language.name'] ?? baseStrings[code]?['language.name'] ?? code;

  /// The language to read in: [chosen] when text for it exists, else the system's, else English.
  static String resolveLanguage(String? chosen, TextPack? pack) {
    final available = {...baseStrings.keys, ...?pack?.languages};
    for (final candidate in [chosen, PlatformDispatcher.instance.locale.languageCode]) {
      if (candidate != null && available.contains(candidate)) return candidate;
    }
    return baseLanguage;
  }

  @override
  bool operator ==(Object other) => other is AppText && other.language == language && other.pack == pack;

  @override
  int get hashCode => Object.hash(language, pack);
}

/// Makes [AppText.of] answer for the subtree.
class AppTextScope extends InheritedWidget {
  const AppTextScope({super.key, required this.text, required super.child});

  final AppText text;

  @override
  bool updateShouldNotify(AppTextScope oldWidget) => oldWidget.text != text;
}

/// Builds the scope above the app's pages: the language chosen in the config (the system's when null) and the text of the look in
/// force, which follows the platform's language when it changes.
class AppTextHost extends StatefulWidget {
  const AppTextHost({super.key, required this.language, required this.child});

  /// `[appearance] language`: a code, or null to follow the system.
  final String? language;
  final Widget child;

  @override
  State<AppTextHost> createState() => _AppTextHostState();
}

class _AppTextHostState extends State<AppTextHost> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeLocales(List<Locale>? locales) => setState(() {});

  @override
  Widget build(BuildContext context) {
    final pack = context.look.words;
    return AppTextScope(
      text: AppText(language: AppText.resolveLanguage(widget.language, pack), pack: pack),
      child: widget.child,
    );
  }
}

/// A named symbol, as the theme draws it.
class SymbolIcon extends StatelessWidget {
  const SymbolIcon(this.name, {super.key, this.size, this.color});

  final String name;
  final double? size;
  final Color? color;

  @override
  Widget build(BuildContext context) => AppText.of(context).symbol(name).build(size: size, color: color);
}

extension AppTextContext on BuildContext {
  /// The text [key] names in the theme and language in force.
  String t(String key, [Map<String, Object?> args = const {}]) => AppText.of(this)(key, args);

  /// The symbol [name] as the theme draws it.
  AppSymbol symbol(String name) => AppText.of(this).symbol(name);
}
