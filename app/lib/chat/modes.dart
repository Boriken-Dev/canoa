import 'package:coqui_acp_chat/coqui_acp_chat.dart' as chat;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ModeEntry;

import 'agent_events.dart';
import 'session_options.dart';

// The mode ids, what Bypass is and how Shift+Tab goes through the modes are `coqui_acp_chat`'s; the names are Canoa's.
export 'package:coqui_acp_chat/coqui_acp_chat.dart' show ModeEntry, bypassAnswer, isBypassMode, modeOptionIn, modeOrder, nextMode, ownBypass;

/// The names the translators' modes are shown under.
const Map<String, String> modeLabels = {
  'manual': 'Manual',
  'accept_edits': 'Accept edits',
  'auto': 'Auto',
  'plan': 'Plan',
  'bypass': 'Bypass',
};

/// The names of the modes other agents give by their own ids, such as Claude Code's `permissionMode`.
const Map<String, String> _otherModeLabels = {
  'default': 'Manual',
  'acceptedits': 'Accept edits',
  'bypasspermissions': 'Bypass',
  'full_access': 'Bypass',
  'full-access': 'Bypass',
  'fullaccess': 'Bypass',
  'yolo': 'Bypass',
  'dontask': "Don't ask",
};

/// The names the efforts are shown under.
const Map<String, String> effortLabels = {
  'none': 'None',
  'minimal': 'Minimal',
  'low': 'Low',
  'medium': 'Medium',
  'high': 'High',
  'xhigh': 'Extra high',
  'max': 'Max',
};

/// [label] as a person reads it: a raw identifier (the id itself, or capitals throughout) in sentence case, the translators'
/// and Claude Code's mode and effort ids by their names, and anything an agent worded itself as it is. A model keeps what the
/// agent calls it, since its names are not words.
String choiceLabel(OptionRole role, Choice choice) {
  final value = choice.value, label = choice.label;
  if (role == OptionRole.mode) {
    final known = modeLabels[value] ?? _otherModeLabels[value.toLowerCase()];
    if (known != null && _isRaw(label, value)) return known;
  } else if (role == OptionRole.effort) {
    final known = effortLabels[value.toLowerCase()];
    if (known != null && _isRaw(label, value)) return known;
  }
  if (role == OptionRole.model || !_isRaw(label, value)) return label;
  final words = label.replaceAll(RegExp(r'[_\-]+'), ' ').trim().toLowerCase();
  return words.isEmpty ? label : '${words[0].toUpperCase()}${words.substring(1)}';
}

/// Whether [label] is [value] spelled as it is sent, or capitals throughout.
bool _isRaw(String label, String value) =>
    label == value || label.toLowerCase() == value.toLowerCase() || (label.toUpperCase() == label && label.contains(RegExp('[A-Z]')));

/// One line on what [entry]'s mode does, where its name is one Canoa knows; null for a mode an agent worded itself.
extension ModeEntryText on ModeEntry {
  String? get description => modeDescriptions[bypass ? 'Bypass' : label];
}

/// What each mode does, by the name it is shown under.
const Map<String, String> modeDescriptions = {
  'Manual': 'Asks before each change or command',
  'Accept edits': 'Changes files without asking; commands still ask',
  'Auto': 'Lets the agent decide what needs asking',
  'Plan': 'Reads and plans; makes no changes',
  'Bypass': 'Asks for nothing',
  "Don't ask": 'Refuses what is not already allowed, rather than ask',
};

/// The modes [option] offers, worded for a person, and Bypass beside them when none of them asks for nothing.
List<ModeEntry> modeEntries(ConfigOption? option) => [
  for (final entry in chat.modeEntries(option))
    ModeEntry(
      entry.value,
      entry.own ? entry.label : choiceLabel(OptionRole.mode, option!.choices.firstWhere((choice) => choice.value == entry.value)),
      own: entry.own,
      selectable: entry.selectable,
    ),
];
