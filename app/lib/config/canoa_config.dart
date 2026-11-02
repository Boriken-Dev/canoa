import 'package:flutter/material.dart' show ThemeMode;
import 'package:toml/toml.dart';

import '../theme/look.dart';

/// Deployment config uses TOML in Canoa.cfg, consistent with AI6's OO.cfg.
/// This file must never contain secrets or private keys.
final class CanoaConfig {
  const CanoaConfig({
    this.startHidden = false,
    this.closeToTray = true,
    this.minimizeToTray = false,
    this.preferWebTransport = true,
    this.connectTimeout = 10,
    this.cdn = 'none',
    this.look = LookId.bubbles,
    this.brightness = 'dark',
  });
  final bool startHidden, closeToTray, minimizeToTray, preferWebTransport;
  final int connectTimeout;
  final String cdn;
  final LookId look;

  /// `light`, `dark`, or `system` to follow the machine.
  final String brightness;

  ThemeMode get themeMode => switch (brightness) {
    'light' => ThemeMode.light,
    'system' => ThemeMode.system,
    _ => ThemeMode.dark,
  };

  CanoaConfig copyWith({bool? startHidden, bool? closeToTray,
      bool? minimizeToTray, bool? preferWebTransport, int? connectTimeout,
      LookId? look, String? brightness}) =>
      CanoaConfig(
        startHidden: startHidden ?? this.startHidden,
        closeToTray: closeToTray ?? this.closeToTray,
        minimizeToTray: minimizeToTray ?? this.minimizeToTray,
        preferWebTransport: preferWebTransport ?? this.preferWebTransport,
        connectTimeout: connectTimeout ?? this.connectTimeout,
        cdn: cdn,
        look: look ?? this.look,
        brightness: brightness ?? this.brightness,
      );

  factory CanoaConfig.parse(String source) {
    final content = source.startsWith('#!toml')
        ? source.substring(source.indexOf('\n') + 1) : source;
    final data = TomlDocument.parse(content).toMap();
    for (final name in data.keys) {
      if (!{'desktop', 'network', 'assets', 'appearance'}.contains(name)) {
        throw FormatException('Canoa.cfg: unknown table $name');
      }
    }
    Map<String, dynamic> table(String name, Set<String> allowed) {
      final raw = data[name];
      if (raw == null) return {};
      if (raw is! Map) throw FormatException('Canoa.cfg: expected a table');
      final entries = Map<String, dynamic>.from(raw);
      for (final key in entries.keys) {
        if (!allowed.contains(key)) throw FormatException('Canoa.cfg: invalid key $key');
      }
      return entries;
    }
    final desktop = table('desktop', {'start_hidden','close_to_tray','minimize_to_tray'});
    final network = table('network', {'prefer_webtransport','connect_timeout'});
    final assets = table('assets', {'cdn'});
    final appearance = table('appearance', {'look', 'brightness'});
    bool flag(Map<String, dynamic> map, String key, bool fallback) {
      final raw = map[key];
      if (raw == null) return fallback;
      if (raw is! bool) throw FormatException('Canoa.cfg: invalid boolean $key');
      return raw;
    }
    final timeout = network['connect_timeout'] ?? 10;
    if (timeout is! int || timeout < 1 || timeout > 120) {
      throw const FormatException('Canoa.cfg: connect_timeout must be 1..120');
    }
    final cdn = assets['cdn'] ?? 'none';
    if (cdn is! String || !{'none', 'local'}.contains(cdn)) {
      throw const FormatException('Canoa.cfg: assets.cdn must be none or local');
    }
    final lookName = appearance['look'] ?? LookId.bubbles.name;
    final look = lookName is String ? LookId.named(lookName) : null;
    if (look == null) {
      throw FormatException('Canoa.cfg: appearance.look must be one of ${LookId.values.map((id) => id.name).join(', ')}');
    }
    final brightness = appearance['brightness'] ?? 'dark';
    if (brightness is! String || !{'light', 'dark', 'system'}.contains(brightness)) {
      throw const FormatException('Canoa.cfg: appearance.brightness must be light, dark or system');
    }
    return CanoaConfig(
      look: look, brightness: brightness,
      startHidden: flag(desktop, 'start_hidden', false),
      closeToTray: flag(desktop, 'close_to_tray', true),
      minimizeToTray: flag(desktop, 'minimize_to_tray', false),
      preferWebTransport: flag(network, 'prefer_webtransport', true),
      connectTimeout: timeout, cdn: cdn,
    );
  }

  String encode() => '''
# Canoa deployment settings — Boriken Dev
[desktop]
start_hidden = $startHidden
close_to_tray = $closeToTray
minimize_to_tray = $minimizeToTray

[network]
prefer_webtransport = $preferWebTransport
connect_timeout = $connectTimeout

[assets]
cdn = "$cdn"

[appearance]
look = "${look.name}"
brightness = "$brightness"
''';
}
