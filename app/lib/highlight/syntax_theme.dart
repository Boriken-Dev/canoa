import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import 'syntax.dart';

/// How code is coloured. With nothing set every colour comes from the look's own tokens (so light, dark and a vendor's look each
/// get theirs); a theme replaces any of them by registering one of these as a [ThemeExtension].
@immutable
final class SyntaxTheme extends ThemeExtension<SyntaxTheme> {
  const SyntaxTheme({this.colors = const {}, this.maxChars = defaultMaxChars});

  /// Replacements by kind of text.
  final Map<Tok, Color> colors;

  /// A text longer than this is drawn plain: the lexer is not run over a whole file that large.
  final int maxChars;

  /// Two megabytes of text, past what the Files preview reads.
  static const int defaultMaxChars = 2 * 1024 * 1024;

  /// The theme in force at [context].
  static SyntaxTheme of(BuildContext context) => Theme.of(context).extension<SyntaxTheme>() ?? const SyntaxTheme();

  /// What each kind of text is drawn as, over [base], for the look [look].
  Map<Tok, TextStyle> styles(ChatLook look, TextStyle base) {
    Color pick(Tok tok, Color fallback) => colors[tok] ?? fallback;
    return {
      Tok.plain: base,
      Tok.keyword: base.copyWith(color: pick(Tok.keyword, look.accent)),
      Tok.type: base.copyWith(color: pick(Tok.type, look.text), fontWeight: FontWeight.w600),
      Tok.literal: base.copyWith(color: pick(Tok.literal, look.warning)),
      Tok.string: base.copyWith(color: pick(Tok.string, look.success)),
      Tok.number: base.copyWith(color: pick(Tok.number, look.warning)),
      Tok.comment: base.copyWith(color: pick(Tok.comment, look.muted), fontStyle: FontStyle.italic),
      Tok.meta: base.copyWith(color: pick(Tok.meta, Color.lerp(look.muted, look.accent, 0.5)!)),
      Tok.inserted: base.copyWith(color: pick(Tok.inserted, look.success)),
      Tok.deleted: base.copyWith(color: pick(Tok.deleted, look.danger)),
    };
  }

  @override
  SyntaxTheme copyWith({Map<Tok, Color>? colors, int? maxChars}) => SyntaxTheme(colors: colors ?? this.colors, maxChars: maxChars ?? this.maxChars);

  @override
  SyntaxTheme lerp(SyntaxTheme? other, double t) => t < 0.5 || other == null ? this : other;
}
