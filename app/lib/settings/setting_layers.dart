import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import 'settings_controller.dart';

/// Which layer of canoa.cfg a value comes from when it is not the person's own file: said on its row, with a way back when the
/// person's file overrides a lower layer.
class SettingLayer {
  const SettingLayer(this.words, {this.onReset});

  /// "From the system file", "Overrides the installed file".
  final String words;

  /// Takes the value out of the person's file so that the lower layer's shows again; null when the person's file holds none.
  final VoidCallback? onReset;
}

/// What [config] says about the value at [path] (`['desktop', 'start_hidden']`; an entry such as `['host', 'agents', 'claude']`
/// counts as one value): "From the system file" when a lower layer gives it and the person's file does not, "Overrides the
/// installed file" with [onReset] when the person's file holds it over a lower layer, else null.
SettingLayer? settingLayer(CanoaConfig config, List<String> path, {required VoidCallback onReset}) {
  final from = config.lowerLayerOf(path);
  if (from != null) return SettingLayer('From $from');
  if (!config.overridesLowerLayer(path)) return null;
  return SettingLayer('Overrides ${config.lowerValueFrom(path)}', onReset: onReset);
}

/// Hands the Settings page's controller to the rows below it, so that any row can say where its value comes from by naming its
/// key in canoa.cfg, with no section repeating the lookup.
class SettingsLayers extends InheritedNotifier<SettingsController> {
  const SettingsLayers({super.key, required SettingsController controller, required super.child}) : super(notifier: controller);

  static SettingsController? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<SettingsLayers>()?.notifier;
}

/// The words a row shows for a value a lower layer gives, and its Reset: for the value at [configKey] in canoa.cfg, as the
/// Settings page holds it, or in [config] when a panel keeps its own copy (its [onReset] then writes the way that panel does).
/// Draws nothing for a value the person's file alone holds.
class SettingNote extends StatelessWidget {
  const SettingNote({super.key, required this.configKey, this.config, this.onReset});

  final List<String> configKey;
  final CanoaConfig? config;
  final VoidCallback? onReset;

  @override
  Widget build(BuildContext context) {
    final controller = SettingsLayers.maybeOf(context);
    final held = config ?? controller?.config;
    if (held == null) return const SizedBox.shrink();
    final reset = onReset ?? (controller == null ? null : () => controller.change((c) => c.resetKey(configKey)));
    final layer = settingLayer(held, configKey, onReset: reset ?? () {});
    if (layer == null) return const SizedBox.shrink();
    return LayerWords(layer: reset == null && layer.onReset != null ? SettingLayer(layer.words) : layer);
  }
}

/// A [SettingLayer] as the small text and the Reset button.
class LayerWords extends StatelessWidget {
  const LayerWords({super.key, required this.layer});

  final SettingLayer layer;

  @override
  Widget build(BuildContext context) {
    final style = Theme.of(context).textTheme.labelSmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant);
    return Wrap(crossAxisAlignment: WrapCrossAlignment.center, children: [
      Text(layer.words, key: const Key('setting-layer'), style: style),
      if (layer.onReset != null)
        TextButton(
          key: const Key('setting-reset'),
          style: TextButton.styleFrom(visualDensity: VisualDensity.compact),
          onPressed: layer.onReset,
          child: const Text('Reset'),
        ),
    ]);
  }
}
