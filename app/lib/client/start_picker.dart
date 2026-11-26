// What a session started from the chat-only app starts with: the loadout and the profile, chosen beside the New session button. The
// full app chooses them on a draft in its composer; here a session starts at once, so the choice is held until it does.
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ComposerPill;
import 'package:material_ui/material_ui.dart';

import '../brand.dart';
import '../chat/composer_controls.dart' show loadoutChoices, profileChoices;
import '../config/canoa_config.dart';

/// The loadout and the profile the next session started from the chat-only app starts with; each is null until the person chooses.
/// What canoa.cfg names for the kind of client applies in the loadout's place until then.
final class StartChoice extends ChangeNotifier {
  String? _loadout;
  String? _preset;

  /// The loadout chosen, by name.
  String? get loadout => _loadout;

  /// The profile chosen, by name.
  String? get preset => _preset;

  /// Chooses a loadout, which a profile's own does not override; null goes back to the default.
  void chooseLoadout(String? name) {
    if (name == _loadout) return;
    _loadout = name;
    notifyListeners();
  }

  /// Chooses a profile, null for none.
  void chooseProfile(String? name) {
    if (name == _preset) return;
    _preset = name;
    notifyListeners();
  }

  /// Goes back to nothing chosen, as it does once a session has started with what was.
  void clear() {
    if (_loadout == null && _preset == null) return;
    _loadout = null;
    _preset = null;
    notifyListeners();
  }
}

/// The loadout and profile controls of the chat-only app, over [choice] and canoa.cfg's [config]. It lists the same choices as the
/// composer of the full app and shows nothing when canoa.cfg holds no loadout and no profile besides the default one.
class StartPicker extends StatelessWidget {
  const StartPicker({super.key, required this.choice, required this.config, this.client});

  final StartChoice choice;
  final CanoaConfig config;

  /// The kind of client this is (one of [clientKinds]), whose default loadout applies while none is chosen.
  final String? client;

  @override
  Widget build(BuildContext context) {
    final loadouts = config.loadouts.keys.toList();
    // The default profile is always there and sets nothing until it is edited: with no other, there is nothing to choose.
    final presets = config.presets.keys.toList();
    final choosable = presets.any((name) => name != presetDefault);
    if (loadouts.isEmpty && !choosable) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: choice,
      builder: (context, _) {
        final preset = config.presets.containsKey(choice.preset) ? choice.preset : null;
        final started = config.loadoutOfStart(loadout: choice.loadout, preset: preset, client: client);
        final byKind = choice.loadout == null && preset == null && client != null && config.clientLoadouts.containsKey(client);
        return Padding(padding: const EdgeInsets.only(top: 8), child: Wrap(spacing: 8, runSpacing: 6, children: [
          if (loadouts.isNotEmpty)
            MenuAnchor(
              menuChildren: loadoutChoices(
                names: loadouts,
                chosen: started,
                defaultName: config.defaultLoadoutFor(client),
                onChoose: choice.chooseLoadout,
              ),
              builder: (context, controller, _) => InkWell(
                key: const Key('start-loadout'),
                borderRadius: BorderRadius.circular(8),
                onTap: () => controller.isOpen ? controller.close() : controller.open(),
                child: Tooltip(
                  message: byKind ? 'The loadout ${Brand.configFile} gives $client clients; choose another for the next session' : 'The loadout the next session starts with',
                  child: ComposerPill(label: started ?? 'Loadout', icon: Icons.layers_outlined),
                ),
              ),
            ),
          if (choosable)
            MenuAnchor(
              menuChildren: profileChoices(names: presets, chosen: preset, onChoose: choice.chooseProfile),
              builder: (context, controller, _) => InkWell(
                key: const Key('start-profile'),
                borderRadius: BorderRadius.circular(8),
                onTap: () => controller.isOpen ? controller.close() : controller.open(),
                child: Tooltip(
                  message: 'The profile the next session starts with: its model, effort, mode and starting instructions',
                  child: ComposerPill(label: preset ?? 'Profile', icon: Icons.account_tree_outlined),
                ),
              ),
            ),
        ]));
      },
    );
  }
}
