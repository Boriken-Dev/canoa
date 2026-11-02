import 'dart:convert' show jsonEncode;

import 'package:flutter/material.dart' show ThemeMode;
import 'package:toml/toml.dart';

import '../acp/agent_commands.dart';
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
    this.agents = const [],
  });
  final bool startHidden, closeToTray, minimizeToTray, preferWebTransport;
  final int connectTimeout;
  final String cdn;
  final LookId look;

  /// Agents this person added, offered beside the presets when what they need is installed.
  final List<AgentCommand> agents;

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
        agents: agents,
      );

  factory CanoaConfig.parse(String source) {
    final content = source.startsWith('#!toml')
        ? source.substring(source.indexOf('\n') + 1) : source;
    final data = TomlDocument.parse(content).toMap();
    for (final name in data.keys) {
      if (!{'desktop', 'network', 'assets', 'appearance', 'agents'}.contains(name)) {
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
    final agents = <AgentCommand>[];
    final rawAgents = data['agents'];
    if (rawAgents != null) {
      if (rawAgents is! Map) throw const FormatException('Canoa.cfg: agents must be a table of agents');
      for (final entry in rawAgents.entries) {
        final agent = entry.value;
        if (agent is! Map || agent['program'] is! String || (agent['program'] as String).isEmpty) {
          throw FormatException('Canoa.cfg: agents.${entry.key} needs a program');
        }
        for (final key in agent.keys) {
          if (key != 'program' && key != 'args') throw FormatException('Canoa.cfg: agents.${entry.key}: invalid key $key');
        }
        final args = agent['args'] ?? const <Object>[];
        if (args is! List || args.any((arg) => arg is! String)) {
          throw FormatException('Canoa.cfg: agents.${entry.key}.args must be a list of strings');
        }
        agents.add(AgentCommand(label: '${entry.key}', program: agent['program'] as String, args: args.cast<String>()));
      }
    }
    return CanoaConfig(
      agents: agents,
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
${agents.map((agent) => '''
[agents.${jsonEncode(agent.label)}]
program = ${jsonEncode(agent.program)}
args = ${jsonEncode(agent.args)}
''').join()}''';
}
