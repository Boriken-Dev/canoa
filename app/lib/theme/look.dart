import 'dart:ui' show lerpDouble;

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

/// The looks a person can choose, named for how they look.
enum LookId {
  canoa('Canoa'),
  terminal('Terminal');

  const LookId(this.label);
  final String label;

  static LookId? named(String name) {
    for (final look in values) {
      if (look.name == name) return look;
    }
    return null;
  }
}

/// What the app's own shell draws with: the session rail, the bar over a conversation. The conversation's own colours and sizes
/// are the [ChatLook] beside it in the theme, of which each [LookId] is a preset.
@immutable
final class CanoaLook extends ThemeExtension<CanoaLook> {
  const CanoaLook({required this.id, required this.rail, required this.bar, required this.selected, required this.railWidth});

  final LookId id;
  final Color rail, bar, selected;
  final double railWidth;

  static const String monospaceFont = 'monospace';

  @override
  CanoaLook copyWith({LookId? id}) => this;

  @override
  CanoaLook lerp(CanoaLook? other, double t) {
    if (other == null) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t)!;
    return CanoaLook(
      id: (t < 0.5 ? this : other).id,
      rail: mix(rail, other.rail),
      bar: mix(bar, other.bar),
      selected: mix(selected, other.selected),
      railWidth: lerpDouble(railWidth, other.railWidth, t)!,
    );
  }
}

const _canoaDark = CanoaLook(id: LookId.canoa, rail: Color(0xFF1C1B19), bar: Color(0xFF232220), selected: Color(0xFF3A3733), railWidth: 260);
const _canoaLight = CanoaLook(id: LookId.canoa, rail: Color(0xFFF1EDE4), bar: Color(0xFFFAF7F1), selected: Color(0xFFE4DDCE), railWidth: 260);
const _terminalDark = CanoaLook(id: LookId.terminal, rail: Color(0xFF0E0F0E), bar: Color(0xFF121412), selected: Color(0xFF243024), railWidth: 220);
const _terminalLight = CanoaLook(id: LookId.terminal, rail: Color(0xFFE9EBE6), bar: Color(0xFFF0F2ED), selected: Color(0xFFD3DBCD), railWidth: 220);

CanoaLook lookFor(LookId id, Brightness brightness) {
  final dark = brightness == Brightness.dark;
  return switch (id) {
    LookId.canoa => dark ? _canoaDark : _canoaLight,
    LookId.terminal => dark ? _terminalDark : _terminalLight,
  };
}

Color _accentOf(LookId id, Brightness brightness) => switch ((id, brightness == Brightness.dark)) {
  (LookId.canoa, true) => const Color(0xFFD9A066),
  (LookId.canoa, false) => const Color(0xFF9A5B1E),
  (LookId.terminal, true) => const Color(0xFF7FD67F),
  (LookId.terminal, false) => const Color(0xFF1F6B2A),
};

/// The conversation's colours and sizes in each look: bubbles on a warm page, or a terminal's monospace lines.
ChatLook chatLookFor(LookId id, Brightness brightness, ColorScheme scheme, {ChatDensity density = ChatDensity.normal}) {
  final dark = brightness == Brightness.dark;
  final (page, user, agent, notice, composer, muted, accent, added, removed, code) = switch ((id, dark)) {
    (LookId.canoa, true) => (
      const Color(0xFF262523), const Color(0xFF34322E), const Color(0xFF262523), const Color(0xFF2E2C29), const Color(0xFF302E2B),
      const Color(0xFFB3AEA5), const Color(0xFFD9A066), const Color(0xFF2F4A34), const Color(0xFF5C3030), const Color(0xFF1B1A18),
    ),
    (LookId.canoa, false) => (
      const Color(0xFFFFFDF8), const Color(0xFFEDE7DA), const Color(0xFFFFFDF8), const Color(0xFFEDE8DE), const Color(0xFFFFFFFF),
      const Color(0xFF6A6358), const Color(0xFF9A5B1E), const Color(0xFFD5EDD7), const Color(0xFFF6D6D3), const Color(0xFFF1ECE2),
    ),
    (LookId.terminal, true) => (
      const Color(0xFF0A0B0A), const Color(0xFF0A0B0A), const Color(0xFF0A0B0A), const Color(0xFF161A16), const Color(0xFF131613),
      const Color(0xFF8DA08D), const Color(0xFF7FD67F), const Color(0xFF173A1C), const Color(0xFF45201F), const Color(0xFF111311),
    ),
    (LookId.terminal, false) => (
      const Color(0xFFF8F9F6), const Color(0xFFF8F9F6), const Color(0xFFF8F9F6), const Color(0xFFE6E9E2), const Color(0xFFFFFFFF),
      const Color(0xFF58665A), const Color(0xFF1F6B2A), const Color(0xFFD4EED4), const Color(0xFFF4D5D3), const Color(0xFFEDEFEA),
    ),
  };
  final terminal = id == LookId.terminal;
  return ChatLook(
    bg: page,
    surface: agent,
    raised: composer,
    text: scheme.onSurface,
    muted: muted,
    accent: accent,
    border: Color.alphaBlend(muted.withValues(alpha: 0.3), page),
    success: Colors.green.shade400,
    warning: scheme.tertiary,
    danger: scheme.error,
    user: user,
    agent: agent,
    notice: notice,
    composer: composer,
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
}

ThemeData themeFor(LookId id, Brightness brightness, {ChatDensity density = ChatDensity.normal}) {
  final shell = lookFor(id, brightness);
  final scheme = ColorScheme.fromSeed(seedColor: _accentOf(id, brightness), brightness: brightness);
  final chat = chatLookFor(id, brightness, scheme, density: density);
  return ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    scaffoldBackgroundColor: chat.bg,
    fontFamily: chat.monospace ? CanoaLook.monospaceFont : null,
    visualDensity: chat.monospace ? VisualDensity.compact : VisualDensity.standard,
    extensions: [shell, chat],
  );
}

extension CanoaLookOf on BuildContext {
  /// The shell's own tokens: the rail and the bar.
  CanoaLook get look => Theme.of(this).extension<CanoaLook>() ?? _canoaDark;
}
