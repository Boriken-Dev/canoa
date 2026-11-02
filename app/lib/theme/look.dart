import 'dart:ui' show lerpDouble;

import 'package:flutter/material.dart';

/// The looks a person can choose, named for how they look.
enum LookId {
  bubbles('Bubbles'),
  document('Document'),
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

/// A look is data: everything the conversation draws differently is here, so a new one needs no new widget.
@immutable
final class CanoaLook extends ThemeExtension<CanoaLook> {
  const CanoaLook({
    required this.id,
    required this.rail,
    required this.bar,
    required this.selected,
    required this.user,
    required this.agent,
    required this.notice,
    required this.composer,
    required this.muted,
    required this.accent,
    required this.added,
    required this.removed,
    required this.code,
    required this.messageWidth,
    required this.gap,
    required this.padding,
    required this.radius,
    required this.bodySize,
    required this.railWidth,
    this.agentInBubble = true,
    this.userToTheRight = true,
    this.monospace = false,
    this.userPrefix,
  });

  final LookId id;
  final Color rail, bar, selected, user, agent, notice, composer, muted, accent, added, removed, code;
  final double messageWidth, gap, padding, radius, bodySize, railWidth;

  /// With this off, an agent's words sit on the page and only the person's are set apart.
  final bool agentInBubble;
  final bool userToTheRight;
  final bool monospace;

  /// Put before what the person wrote, in place of setting it apart with a colour.
  final String? userPrefix;

  static const String monospaceFont = 'monospace';

  @override
  CanoaLook copyWith({LookId? id}) => this;

  @override
  CanoaLook lerp(CanoaLook? other, double t) {
    if (other == null) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t)!;
    double size(double a, double b) => lerpDouble(a, b, t)!;
    final near = t < 0.5 ? this : other;
    return CanoaLook(
      id: near.id,
      rail: mix(rail, other.rail),
      bar: mix(bar, other.bar),
      selected: mix(selected, other.selected),
      user: mix(user, other.user),
      agent: mix(agent, other.agent),
      notice: mix(notice, other.notice),
      composer: mix(composer, other.composer),
      muted: mix(muted, other.muted),
      accent: mix(accent, other.accent),
      added: mix(added, other.added),
      removed: mix(removed, other.removed),
      code: mix(code, other.code),
      messageWidth: size(messageWidth, other.messageWidth),
      gap: size(gap, other.gap),
      padding: size(padding, other.padding),
      radius: size(radius, other.radius),
      bodySize: size(bodySize, other.bodySize),
      railWidth: size(railWidth, other.railWidth),
      agentInBubble: near.agentInBubble,
      userToTheRight: near.userToTheRight,
      monospace: near.monospace,
      userPrefix: near.userPrefix,
    );
  }
}

const _bubblesDark = CanoaLook(
  id: LookId.bubbles,
  rail: Color(0xFF17242D),
  bar: Color(0xFF15222C),
  selected: Color(0xFF294952),
  user: Color(0xFF235853),
  agent: Color(0xFF1D2B34),
  notice: Color(0xFF26333D),
  composer: Color(0xFF1B3039),
  muted: Color(0xFFABBEC8),
  accent: Color(0xFF6DE2D2),
  added: Color(0xFF1E4D3A),
  removed: Color(0xFF5A2A2E),
  code: Color(0xFF0F1A21),
  messageWidth: 720,
  gap: 14,
  padding: 14,
  radius: 16,
  bodySize: 15,
  railWidth: 272,
);

const _bubblesLight = CanoaLook(
  id: LookId.bubbles,
  rail: Color(0xFFE4EEEE),
  bar: Color(0xFFEFF5F5),
  selected: Color(0xFFC6E4E0),
  user: Color(0xFFBFE6E0),
  agent: Color(0xFFFFFFFF),
  notice: Color(0xFFDDE7E9),
  composer: Color(0xFFFFFFFF),
  muted: Color(0xFF4F6570),
  accent: Color(0xFF0E7C70),
  added: Color(0xFFCDEFD9),
  removed: Color(0xFFF8D4D4),
  code: Color(0xFFEDF2F3),
  messageWidth: 720,
  gap: 14,
  padding: 14,
  radius: 16,
  bodySize: 15,
  railWidth: 272,
);

const _documentDark = CanoaLook(
  id: LookId.document,
  rail: Color(0xFF1C1B19),
  bar: Color(0xFF232220),
  selected: Color(0xFF3A3733),
  user: Color(0xFF34322E),
  agent: Color(0xFF262523),
  notice: Color(0xFF2E2C29),
  composer: Color(0xFF302E2B),
  muted: Color(0xFFB3AEA5),
  accent: Color(0xFFD9A066),
  added: Color(0xFF2F4A34),
  removed: Color(0xFF5C3030),
  code: Color(0xFF1B1A18),
  messageWidth: 780,
  gap: 22,
  padding: 12,
  radius: 12,
  bodySize: 16,
  railWidth: 260,
  agentInBubble: false,
);

const _documentLight = CanoaLook(
  id: LookId.document,
  rail: Color(0xFFF1EDE4),
  bar: Color(0xFFFAF7F1),
  selected: Color(0xFFE4DDCE),
  user: Color(0xFFEDE7DA),
  agent: Color(0xFFFFFDF8),
  notice: Color(0xFFEDE8DE),
  composer: Color(0xFFFFFFFF),
  muted: Color(0xFF6A6358),
  accent: Color(0xFF9A5B1E),
  added: Color(0xFFD5EDD7),
  removed: Color(0xFFF6D6D3),
  code: Color(0xFFF1ECE2),
  messageWidth: 780,
  gap: 22,
  padding: 12,
  radius: 12,
  bodySize: 16,
  railWidth: 260,
  agentInBubble: false,
);

const _terminalDark = CanoaLook(
  id: LookId.terminal,
  rail: Color(0xFF0E0F0E),
  bar: Color(0xFF121412),
  selected: Color(0xFF243024),
  user: Color(0xFF0A0B0A),
  agent: Color(0xFF0A0B0A),
  notice: Color(0xFF161A16),
  composer: Color(0xFF131613),
  muted: Color(0xFF8DA08D),
  accent: Color(0xFF7FD67F),
  added: Color(0xFF173A1C),
  removed: Color(0xFF45201F),
  code: Color(0xFF111311),
  messageWidth: 1100,
  gap: 6,
  padding: 4,
  radius: 2,
  bodySize: 13,
  railWidth: 220,
  agentInBubble: false,
  userToTheRight: false,
  monospace: true,
  userPrefix: '> ',
);

const _terminalLight = CanoaLook(
  id: LookId.terminal,
  rail: Color(0xFFE9EBE6),
  bar: Color(0xFFF0F2ED),
  selected: Color(0xFFD3DBCD),
  user: Color(0xFFF8F9F6),
  agent: Color(0xFFF8F9F6),
  notice: Color(0xFFE6E9E2),
  composer: Color(0xFFFFFFFF),
  muted: Color(0xFF58665A),
  accent: Color(0xFF1F6B2A),
  added: Color(0xFFD4EED4),
  removed: Color(0xFFF4D5D3),
  code: Color(0xFFEDEFEA),
  messageWidth: 1100,
  gap: 6,
  padding: 4,
  radius: 2,
  bodySize: 13,
  railWidth: 220,
  agentInBubble: false,
  userToTheRight: false,
  monospace: true,
  userPrefix: '> ',
);

CanoaLook lookFor(LookId id, Brightness brightness) {
  final dark = brightness == Brightness.dark;
  return switch (id) {
    LookId.bubbles => dark ? _bubblesDark : _bubblesLight,
    LookId.document => dark ? _documentDark : _documentLight,
    LookId.terminal => dark ? _terminalDark : _terminalLight,
  };
}

ThemeData themeFor(LookId id, Brightness brightness) {
  final look = lookFor(id, brightness);
  final dark = brightness == Brightness.dark;
  return ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: ColorScheme.fromSeed(seedColor: look.accent, brightness: brightness),
    scaffoldBackgroundColor: switch ((id, dark)) {
      (LookId.bubbles, true) => const Color(0xFF101820),
      (LookId.bubbles, false) => const Color(0xFFF6FAFA),
      (LookId.document, true) => const Color(0xFF262523),
      (LookId.document, false) => const Color(0xFFFFFDF8),
      (LookId.terminal, true) => const Color(0xFF0A0B0A),
      (LookId.terminal, false) => const Color(0xFFF8F9F6),
    },
    fontFamily: look.monospace ? CanoaLook.monospaceFont : null,
    visualDensity: look.monospace ? VisualDensity.compact : VisualDensity.standard,
    extensions: [look],
  );
}

extension CanoaLookOf on BuildContext {
  CanoaLook get look => Theme.of(this).extension<CanoaLook>() ?? _bubblesDark;
}
