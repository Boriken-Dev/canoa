import 'package:coqui_acp/coqui_acp.dart';

import 'agent_events.dart';
import 'session_options.dart';

/// The mode ids the Rust translators (`coqui_acp_harness::mode`) offer, in the order Shift+Tab cycles them.
const List<String> modeOrder = ['manual', 'accept_edits', 'auto', 'plan', 'bypass'];

/// The names the translators' modes are shown under.
const Map<String, String> modeLabels = {
  'manual': 'Manual',
  'accept_edits': 'Accept edits',
  'auto': 'Auto',
  'plan': 'Plan',
  'bypass': 'Bypass',
};

/// The ids an agent may give a mode that asks for nothing: the translators' own, and the names Claude Code and others use.
const Set<String> _bypassIds = {'bypass', 'bypasspermissions', 'full_access', 'full-access', 'fullaccess', 'yolo'};

/// Whether a mode of [value], or labelled [label], asks for nothing.
bool isBypassMode(String value, [String? label]) =>
    _bypassIds.contains(value.toLowerCase()) || label?.toLowerCase() == 'bypass';

/// The id Canoa gives the bypass mode it provides itself, for an agent that has none.
const String ownBypass = 'bypass';

/// One entry of the mode menu: an agent's own mode, or the Bypass Canoa provides where the agent has no mode that asks for nothing.
final class ModeEntry {
  const ModeEntry(this.value, this.label, {this.own = false});
  final String value;
  final String label;

  /// Whether Canoa provides it, by answering the agent's requests, rather than the agent.
  final bool own;

  bool get bypass => isBypassMode(value, label);
}

/// The modes [option] offers, and Bypass beside them when none of them asks for nothing.
List<ModeEntry> modeEntries(ConfigOption? option) {
  final entries = [for (final choice in option?.choices ?? const <Choice>[]) ModeEntry(choice.value, choice.label)];
  if (!entries.any((entry) => entry.bypass)) entries.add(const ModeEntry(ownBypass, 'Bypass', own: true));
  return entries;
}

/// The entry after [current] in [entries], in the order of [modeOrder] where the ids are known and the agent's own order
/// otherwise; the first entry when [current] is not among them.
ModeEntry nextMode(List<ModeEntry> entries, String? current) {
  final ordered = [...entries];
  int rank(ModeEntry entry) {
    final at = modeOrder.indexOf(entry.bypass ? ownBypass : entry.value);
    return at < 0 ? modeOrder.length : at;
  }
  if (ordered.every((entry) => rank(entry) < modeOrder.length)) ordered.sort((a, b) => rank(a).compareTo(rank(b)));
  final at = ordered.indexWhere((entry) => entry.value == current);
  return ordered[(at + 1) % ordered.length];
}

/// The option a request in bypass is answered with: the first that allows once, else the first that allows always. Null when
/// it offers neither: that one is the person's.
AcpPermissionOption? bypassAnswer(List<AcpPermissionOption> options) =>
    options.where((option) => option.kind == 'allow_once').firstOrNull ??
    options.where((option) => option.kind == 'allow_always').firstOrNull;

/// The mode option among [options].
ConfigOption? modeOptionIn(List<ConfigOption> options) => optionIn(options, OptionRole.mode);
