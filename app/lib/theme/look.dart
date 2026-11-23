import 'dart:ui' show lerpDouble;

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../l10n/text_pack.dart';
import '../highlight/highlighted.dart' show codeSpansFor;
import 'vendor_looks.dart';

/// The id of a look family. Canoa's own look and the terminal's are built in; every other is a family the host lists (`looks/list`,
/// from the agents it loaded) or one a config names that it does not, which draws as Canoa's look and is kept as written.
@immutable
final class LookId {
  /// The family [name], which a person reads as [label] (the id when none is given).
  const LookId(this.name, [String? label]) : label = label ?? name;

  static const canoa = LookId('canoa', 'Canoa');
  static const terminal = LookId('terminal', 'Terminal');

  /// The families the app carries itself, in the order a person is offered them.
  static const builtIn = [canoa, terminal];

  static final RegExp _word = RegExp(r'^[a-z0-9][a-z0-9_-]{0,63}$');

  /// The family called [name], with [label] for a person to read when it is not built in (the id when none is given); null when
  /// [name] is no id.
  static LookId? named(String name, {String? label}) {
    for (final look in builtIn) {
      if (look.name == name) return look;
    }
    if (!_word.hasMatch(name)) return null;
    final text = label?.trim();
    return LookId(name, text == null || text.isEmpty ? null : text);
  }

  /// What the config and the host's documents call it.
  final String name;

  /// What a person reads for it.
  final String label;

  /// Whether the app carries this family itself, so no host has to list it.
  bool get isBuiltIn => builtIn.contains(this);

  @override
  bool operator ==(Object other) => other is LookId && other.name == name;

  @override
  int get hashCode => name.hashCode;

  @override
  String toString() => 'LookId($name)';
}

/// The tokens every surface draws with, in one look and brightness: the shell (rail, bar, selection), the pages (settings, the
/// store, loadouts, accounts, hosts) and the conversation. [themeFor] turns them into the colour scheme and the component themes,
/// so a field, a button, a tab, a menu, a dialog or a card looks the same wherever it is.
@immutable
final class CanoaLook extends ThemeExtension<CanoaLook> {
  const CanoaLook({
    required this.id,
    required this.page,
    required this.rail,
    required this.bar,
    required this.selected,
    required this.surface,
    required this.raised,
    required this.border,
    required this.text,
    required this.muted,
    required this.accent,
    required this.onAccent,
    required this.success,
    required this.warning,
    required this.danger,
    required this.radius,
    required this.controlRadius,
    required this.railWidth,
    this.words,
  });

  final LookId id;

  /// The page behind everything, and the navigation (rail), the bars, and the row selected.
  final Color page, rail, bar, selected;

  /// A card or a dialog, and a field or a menu, which sit over the page.
  final Color surface, raised;

  /// A hairline, the text on the page and the quieter text beside it.
  final Color border, text, muted;

  /// The accent (buttons, links, focus, the current tab) and the text on it.
  final Color accent, onAccent;

  /// What a state reads as: working or online, needs a look, failed.
  final Color success, warning, danger;

  /// The corner of a card or dialog, and of a field or button.
  final double radius, controlRadius;
  final double railWidth;

  /// The words and symbols this look sets in place of the base ones; null when it draws the base.
  final TextPack? words;

  /// The outline that shows keyboard focus.
  Color get focus => accent;

  static const String monospaceFont = 'monospace';

  @override
  CanoaLook copyWith({LookId? id}) => this;

  @override
  CanoaLook lerp(CanoaLook? other, double t) {
    if (other == null) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t)!;
    double number(double a, double b) => lerpDouble(a, b, t)!;
    return CanoaLook(
      id: (t < 0.5 ? this : other).id,
      page: mix(page, other.page),
      rail: mix(rail, other.rail),
      bar: mix(bar, other.bar),
      selected: mix(selected, other.selected),
      surface: mix(surface, other.surface),
      raised: mix(raised, other.raised),
      border: mix(border, other.border),
      text: mix(text, other.text),
      muted: mix(muted, other.muted),
      accent: mix(accent, other.accent),
      onAccent: mix(onAccent, other.onAccent),
      success: mix(success, other.success),
      warning: mix(warning, other.warning),
      danger: mix(danger, other.danger),
      radius: number(radius, other.radius),
      controlRadius: number(controlRadius, other.controlRadius),
      railWidth: number(railWidth, other.railWidth),
      words: (t < 0.5 ? this : other).words,
    );
  }
}

Color _hairline(Color muted, Color page) => Color.alphaBlend(muted.withValues(alpha: 0.3), page);

final _canoaDark = CanoaLook(
  id: LookId.canoa, page: const Color(0xFF262523), rail: const Color(0xFF1C1B19), bar: const Color(0xFF232220),
  selected: const Color(0xFF3A3733), surface: const Color(0xFF2E2C29), raised: const Color(0xFF302E2B),
  border: _hairline(const Color(0xFFB3AEA5), const Color(0xFF262523)), text: const Color(0xFFEDE8DE), muted: const Color(0xFFB3AEA5),
  accent: const Color(0xFFD9A066), onAccent: const Color(0xFF241707),
  success: const Color(0xFF7FC48A), warning: const Color(0xFFE2B455), danger: const Color(0xFFEA8F85),
  radius: 12, controlRadius: 8, railWidth: 260,
);
final _canoaLight = CanoaLook(
  id: LookId.canoa, page: const Color(0xFFFFFDF8), rail: const Color(0xFFF1EDE4), bar: const Color(0xFFFAF7F1),
  selected: const Color(0xFFE4DDCE), surface: const Color(0xFFF6F2E9), raised: const Color(0xFFFFFFFF),
  border: _hairline(const Color(0xFF6A6358), const Color(0xFFFFFDF8)), text: const Color(0xFF2B2822), muted: const Color(0xFF6A6358),
  accent: const Color(0xFF9A5B1E), onAccent: const Color(0xFFFFFFFF),
  success: const Color(0xFF276B37), warning: const Color(0xFF8A5A00), danger: const Color(0xFFB3261E),
  radius: 12, controlRadius: 8, railWidth: 260,
);
final _terminalDark = CanoaLook(
  id: LookId.terminal, page: const Color(0xFF0A0B0A), rail: const Color(0xFF0E0F0E), bar: const Color(0xFF121412),
  selected: const Color(0xFF243024), surface: const Color(0xFF111311), raised: const Color(0xFF131613),
  border: _hairline(const Color(0xFF8DA08D), const Color(0xFF0A0B0A)), text: const Color(0xFFD3E2D3), muted: const Color(0xFF8DA08D),
  accent: const Color(0xFF7FD67F), onAccent: const Color(0xFF04140A),
  success: const Color(0xFF7FD67F), warning: const Color(0xFFE0C060), danger: const Color(0xFFF08A82),
  radius: 2, controlRadius: 2, railWidth: 220,
);
final _terminalLight = CanoaLook(
  id: LookId.terminal, page: const Color(0xFFF8F9F6), rail: const Color(0xFFE9EBE6), bar: const Color(0xFFF0F2ED),
  selected: const Color(0xFFD3DBCD), surface: const Color(0xFFEDEFEA), raised: const Color(0xFFFFFFFF),
  border: _hairline(const Color(0xFF58665A), const Color(0xFFF8F9F6)), text: const Color(0xFF1B241C), muted: const Color(0xFF58665A),
  accent: const Color(0xFF1F6B2A), onAccent: const Color(0xFFFFFFFF),
  success: const Color(0xFF1F6B2A), warning: const Color(0xFF7A5A00), danger: const Color(0xFFB3261E),
  radius: 2, controlRadius: 2, railWidth: 220,
);

/// [id]'s tokens in [brightness]. Canoa's and the terminal's are built in; every other family comes from the host ([vendorLooks])
/// and, until it lists it, draws as Canoa's look does.
CanoaLook lookFor(LookId id, Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final canoa = dark ? _canoaDark : _canoaLight;
  if (id == LookId.canoa) return canoa;
  if (id == LookId.terminal) return dark ? _terminalDark : _terminalLight;
  return vendorLooks.shell(id, brightness, canoa) ?? canoa;
}

/// The conversation's colours and sizes in each look: bubbles on a warm page, or a terminal's monospace lines.
ChatLook chatLookFor(LookId id, Brightness brightness, ColorScheme scheme, {ChatDensity density = ChatDensity.normal}) {
  final dark = brightness == Brightness.dark;
  final shell = lookFor(id, brightness);
  final ChatTokens canoa = dark
      ? (user: const Color(0xFF34322E), notice: const Color(0xFF2E2C29), added: const Color(0xFF2F4A34), removed: const Color(0xFF5C3030), code: const Color(0xFF1B1A18))
      : (user: const Color(0xFFEDE7DA), notice: const Color(0xFFEDE8DE), added: const Color(0xFFD5EDD7), removed: const Color(0xFFF6D6D3), code: const Color(0xFFF1ECE2));
  final ChatTokens terminalTokens = dark
      ? (user: const Color(0xFF0A0B0A), notice: const Color(0xFF161A16), added: const Color(0xFF173A1C), removed: const Color(0xFF45201F), code: const Color(0xFF111311))
      : (user: const Color(0xFFF8F9F6), notice: const Color(0xFFE6E9E2), added: const Color(0xFFD4EED4), removed: const Color(0xFFF4D5D3), code: const Color(0xFFEDEFEA));
  final (:user, :notice, :added, :removed, :code) = id == LookId.canoa
      ? canoa
      : id == LookId.terminal
      ? terminalTokens
      : vendorLooks.chat(id, brightness, canoa);
  final terminal = id == LookId.terminal;
  final look = ChatLook(
    bg: shell.page,
    surface: shell.page,
    raised: shell.raised,
    text: scheme.onSurface,
    muted: shell.muted,
    accent: shell.accent,
    border: shell.border,
    success: Colors.green.shade400,
    warning: scheme.tertiary,
    danger: scheme.error,
    user: user,
    agent: shell.page,
    notice: notice,
    composer: shell.raised,
    code: code,
    added: added,
    removed: removed,
    messageWidth: terminal ? 1100 : 780,
    gap: terminal ? 6 : 22,
    padding: terminal ? 4 : 12,
    radius: terminal ? 2 : 12,
    bodySize: terminal ? 13 : 16,
    density: density,
    codeFont: CanoaLook.monospaceFont,
    agentInBubble: false,
    userToTheRight: !terminal,
    monospace: terminal,
    userPrefix: terminal ? '> ' : null,
  );
  // The colours come from the look itself, so each look and brightness draws code in its own.
  return look.copyWith(codeSpans: codeSpansFor(look));
}

/// The colour scheme the tokens make, so a widget reading `colorScheme` draws in the look and not in a seeded palette.
ColorScheme _schemeFor(CanoaLook look, Brightness brightness) => ColorScheme.fromSeed(seedColor: look.accent, brightness: brightness).copyWith(
  primary: look.accent,
  onPrimary: look.onAccent,
  primaryContainer: look.selected,
  onPrimaryContainer: look.text,
  secondary: look.accent,
  onSecondary: look.onAccent,
  secondaryContainer: look.selected,
  onSecondaryContainer: look.text,
  tertiary: look.warning,
  error: look.danger,
  onError: brightness == Brightness.dark ? const Color(0xFF1A0705) : const Color(0xFFFFFFFF),
  surface: look.page,
  onSurface: look.text,
  onSurfaceVariant: look.muted,
  surfaceContainerLowest: look.page,
  surfaceContainerLow: look.rail,
  surfaceContainer: look.surface,
  surfaceContainerHigh: look.raised,
  surfaceContainerHighest: look.selected,
  outline: look.muted,
  outlineVariant: look.border,
  surfaceTint: Colors.transparent,
);

/// Component themes drawn from the tokens: fields, buttons, segmented buttons, tabs, menus, dialogs, cards and list rows.
ThemeData _components(ThemeData base, CanoaLook look, ColorScheme scheme) {
  final control = RoundedRectangleBorder(borderRadius: BorderRadius.circular(look.controlRadius));
  final card = RoundedRectangleBorder(borderRadius: BorderRadius.circular(look.radius), side: BorderSide(color: look.border));
  final menu = RoundedRectangleBorder(borderRadius: BorderRadius.circular(look.controlRadius), side: BorderSide(color: look.border));
  OutlineInputBorder field(Color color, [double width = 1]) =>
      OutlineInputBorder(borderRadius: BorderRadius.circular(look.controlRadius), borderSide: BorderSide(color: color, width: width));
  final menuStyle = MenuStyle(
    backgroundColor: WidgetStatePropertyAll(look.raised),
    surfaceTintColor: const WidgetStatePropertyAll(Colors.transparent),
    shape: WidgetStatePropertyAll(menu),
    elevation: const WidgetStatePropertyAll(3),
  );
  return base.copyWith(
    dividerColor: look.border,
    dividerTheme: DividerThemeData(color: look.border, space: 1, thickness: 1),
    focusColor: look.focus.withValues(alpha: 0.18),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: look.raised,
      isDense: true,
      border: field(look.border),
      enabledBorder: field(look.border),
      focusedBorder: field(look.focus, 2),
      errorBorder: field(look.danger),
      focusedErrorBorder: field(look.danger, 2),
      disabledBorder: field(look.border.withValues(alpha: 0.5)),
      labelStyle: TextStyle(color: look.muted),
      hintStyle: TextStyle(color: look.muted),
      helperStyle: TextStyle(color: look.muted),
      errorStyle: TextStyle(color: look.danger),
    ),
    filledButtonTheme: FilledButtonThemeData(style: FilledButton.styleFrom(
      backgroundColor: look.accent, foregroundColor: look.onAccent, disabledBackgroundColor: look.selected,
      disabledForegroundColor: look.muted, shape: control, elevation: 0)),
    elevatedButtonTheme: ElevatedButtonThemeData(style: ElevatedButton.styleFrom(
      backgroundColor: look.raised, foregroundColor: look.text, surfaceTintColor: Colors.transparent, shape: control, elevation: 0,
      side: BorderSide(color: look.border))),
    outlinedButtonTheme: OutlinedButtonThemeData(style: OutlinedButton.styleFrom(
      foregroundColor: look.text, shape: control, side: BorderSide(color: look.border))),
    textButtonTheme: TextButtonThemeData(style: TextButton.styleFrom(foregroundColor: look.accent, shape: control)),
    iconButtonTheme: IconButtonThemeData(style: IconButton.styleFrom(foregroundColor: look.muted, shape: control)),
    segmentedButtonTheme: SegmentedButtonThemeData(style: ButtonStyle(
      shape: WidgetStatePropertyAll(control),
      side: WidgetStatePropertyAll(BorderSide(color: look.border)),
      backgroundColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.selected : Colors.transparent),
      foregroundColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.text : look.muted),
      iconColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.accent : look.muted),
    )),
    tabBarTheme: TabBarThemeData(
      labelColor: look.text,
      unselectedLabelColor: look.muted,
      indicatorColor: look.accent,
      dividerColor: look.border,
      indicatorSize: TabBarIndicatorSize.label,
    ),
    menuTheme: MenuThemeData(style: menuStyle),
    popupMenuTheme: PopupMenuThemeData(color: look.raised, surfaceTintColor: Colors.transparent, shape: menu, elevation: 3),
    dropdownMenuTheme: DropdownMenuThemeData(menuStyle: menuStyle),
    menuButtonTheme: MenuButtonThemeData(style: ButtonStyle(
      foregroundColor: WidgetStatePropertyAll(look.text),
      overlayColor: WidgetStatePropertyAll(look.selected.withValues(alpha: 0.6)),
    )),
    dialogTheme: DialogThemeData(backgroundColor: look.surface, surfaceTintColor: Colors.transparent, shape: card, elevation: 6),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: look.surface, surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(look.radius)))),
    cardTheme: CardThemeData(color: look.surface, surfaceTintColor: Colors.transparent, shape: card, elevation: 0, margin: EdgeInsets.zero),
    listTileTheme: ListTileThemeData(
      shape: control,
      selectedColor: look.text,
      iconColor: look.muted,
      textColor: look.text,
    ),
    expansionTileTheme: ExpansionTileThemeData(iconColor: look.muted, collapsedIconColor: look.muted, textColor: look.text, collapsedTextColor: look.text),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.onAccent : look.muted),
      trackColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.accent : look.selected),
      trackOutlineColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.accent : look.border),
    ),
    checkboxTheme: CheckboxThemeData(
      fillColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.accent : Colors.transparent),
      checkColor: WidgetStatePropertyAll(look.onAccent),
      side: BorderSide(color: look.muted, width: 1.5),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(look.controlRadius / 2)),
    ),
    radioTheme: RadioThemeData(fillColor: WidgetStateProperty.resolveWith((states) => states.contains(WidgetState.selected) ? look.accent : look.muted)),
    chipTheme: ChipThemeData(
      backgroundColor: Colors.transparent,
      selectedColor: look.selected,
      side: BorderSide(color: look.border),
      shape: control,
      labelStyle: base.textTheme.labelLarge?.copyWith(color: look.text),
      checkmarkColor: look.accent,
    ),
    tooltipTheme: TooltipThemeData(
      decoration: BoxDecoration(color: look.raised, borderRadius: BorderRadius.circular(look.controlRadius), border: Border.all(color: look.border)),
      textStyle: base.textTheme.bodySmall?.copyWith(color: look.text),
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: look.raised, contentTextStyle: base.textTheme.bodyMedium?.copyWith(color: look.text), actionTextColor: look.accent,
      shape: menu),
    progressIndicatorTheme: ProgressIndicatorThemeData(color: look.accent, linearTrackColor: look.selected),
    textSelectionTheme: TextSelectionThemeData(cursorColor: look.accent, selectionColor: look.accent.withValues(alpha: 0.3)),
    scrollbarTheme: ScrollbarThemeData(thumbColor: WidgetStatePropertyAll(look.muted.withValues(alpha: 0.5))),
    appBarTheme: AppBarTheme(
      backgroundColor: look.bar, foregroundColor: look.text, surfaceTintColor: Colors.transparent, elevation: 0, scrolledUnderElevation: 0),
  );
}

ThemeData themeFor(LookId id, Brightness brightness, {ChatDensity density = ChatDensity.normal}) {
  final shell = lookFor(id, brightness);
  final scheme = _schemeFor(shell, brightness);
  final chat = chatLookFor(id, brightness, scheme, density: density);
  final base = ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    scaffoldBackgroundColor: chat.bg,
    fontFamily: chat.monospace ? CanoaLook.monospaceFont : null,
    visualDensity: chat.monospace ? VisualDensity.compact : VisualDensity.standard,
    extensions: [shell, chat],
  );
  return _components(base, shell, scheme);
}

extension CanoaLookOf on BuildContext {
  /// The tokens every surface draws with: the page, the rail and the bar, a card, the borders, the accent and what a state reads as.
  CanoaLook get look => Theme.of(this).extension<CanoaLook>() ?? _canoaDark;
}
