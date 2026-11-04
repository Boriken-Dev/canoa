import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../acp/client.dart';
import '../session_store.dart';
import 'agent_events.dart' show boundedHead, shortText;

/// The most of an agent's question shown, since it can send any amount.
const int maxQuestionText = 4000;

final _hidden = RegExp(r'[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u200e\u200f\u2028-\u202e\u2066-\u2069]');

/// An agent's text as plain data: line breaks kept, nothing that could reorder or hide what surrounds it.
String _plain(String text) => boundedHead(text.replaceAll('\r\n', '\n').replaceAll(_hidden, ' '), maxQuestionText);

/// An agent's question, as a form built from its schema. The person sends an answer, declines, or cancels; everything the agent
/// wrote is shown as text, never as markup or a link.
class QuestionCard extends StatefulWidget {
  const QuestionCard({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  State<QuestionCard> createState() => _QuestionCardState();
}

class _QuestionCardState extends State<QuestionCard> {
  late AgentQuestion _question;

  /// What the person gave each field so far, by name; a field with no value is left out of the answer.
  final Map<String, Object> _values = {};
  final Map<String, TextEditingController> _texts = {};

  @override
  void initState() {
    super.initState();
    _start();
  }

  @override
  void didUpdateWidget(QuestionCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(widget.session.pendingQuestions.first, _question)) {
      _clear();
      _start();
    }
  }

  /// The form of the oldest question, at the values the agent gave as defaults.
  void _start() {
    _question = widget.session.pendingQuestions.first;
    for (final field in _question.request.fields) {
      final start = switch (field) {
        AcpTextField(:final defaultValue) => defaultValue,
        AcpChoiceField(:final defaultValue) => defaultValue,
        AcpChoicesField(:final defaultValue) => defaultValue,
        AcpNumberField(:final defaultValue) => defaultValue,
        AcpBooleanField(:final defaultValue) => defaultValue,
        AcpUnsupportedField() => null,
      };
      if (start != null) _values[field.name] = start;
      if (field is AcpTextField || field is AcpNumberField) {
        _texts[field.name] = TextEditingController(text: start == null ? '' : '$start');
      }
    }
  }

  void _clear() {
    for (final controller in _texts.values) {
      controller.dispose();
    }
    _texts.clear();
    _values.clear();
  }

  @override
  void dispose() {
    _clear();
    super.dispose();
  }

  /// Why the answer cannot be sent yet, or null when it can.
  String? get _problem {
    for (final field in _question.request.fields) {
      final value = _values[field.name];
      final label = shortText(field.title) ?? shortText(field.name) ?? 'A question';
      if (value == null) {
        if (field.required) {
          return field is AcpUnsupportedField ? '$label cannot be answered here, so only Decline or Cancel can answer this.' : '$label is required.';
        }
        continue;
      }
      switch (field) {
        case AcpTextField(:final minLength, :final maxLength):
          final length = (value as String).runes.length;
          if (minLength != null && length < minLength) return '$label needs at least $minLength characters.';
          if (maxLength != null && length > maxLength) return '$label takes at most $maxLength characters.';
        case AcpChoicesField(:final minItems, :final maxItems):
          final count = (value as List).length;
          if (minItems != null && count < minItems) return 'Choose at least $minItems for $label.';
          if (maxItems != null && count > maxItems) return 'Choose at most $maxItems for $label.';
        case AcpNumberField(:final minimum, :final maximum):
          if (minimum != null && (value as num) < minimum) return '$label is at least $minimum.';
          if (maximum != null && (value as num) > maximum) return '$label is at most $maximum.';
        default:
          break;
      }
    }
    return null;
  }

  void _answer(AcpElicitationAnswer answer) => widget.store.answerQuestion(widget.session, answer);

  Widget _field(AcpField field) {
    final title = shortText(field.title) ?? shortText(field.name) ?? 'A question';
    final description = field.description == null ? null : _plain(field.description!);
    final label = '$title${field.required ? ' *' : ''}';
    void set(Object? value) => setState(() => value == null ? _values.remove(field.name) : _values[field.name] = value);
    final key = ValueKey('question-${field.name}');
    switch (field) {
      case AcpTextField(:final format):
        return TextField(
          key: key,
          controller: _texts[field.name],
          decoration: InputDecoration(labelText: label, helperText: [?description, ?shortText(format)].join(' · ').nullIfEmpty),
          onChanged: (text) => set(text.isEmpty ? null : text),
        );
      case AcpNumberField(:final integer):
        return TextField(
          key: key,
          controller: _texts[field.name],
          keyboardType: TextInputType.numberWithOptions(decimal: !integer, signed: true),
          inputFormatters: [FilteringTextInputFormatter.allow(RegExp(integer ? r'[-0-9]' : r'[-0-9.eE+]'))],
          decoration: InputDecoration(labelText: label, helperText: description),
          onChanged: (text) => set(integer ? int.tryParse(text) : num.tryParse(text)),
        );
      case AcpBooleanField():
        return SwitchListTile(
          key: key,
          contentPadding: EdgeInsets.zero,
          title: Text(label),
          subtitle: description == null ? null : Text(description),
          value: _values[field.name] == true,
          onChanged: set,
        );
      case AcpChoiceField(:final options):
        return _Labelled(label: label, description: description, child: Wrap(spacing: 6, runSpacing: 4, children: [
          for (final option in options)
            _Described(
              description: option.description,
              child: ChoiceChip(
                key: ValueKey('question-${field.name}-${option.value}'),
                label: Text(shortText(option.title) ?? shortText(option.value) ?? '?'),
                selected: _values[field.name] == option.value,
                onSelected: (on) => set(on ? option.value : null),
              ),
            ),
        ]));
      case AcpChoicesField(:final options):
        final chosen = [...(_values[field.name] as List<String>?) ?? const <String>[]];
        return _Labelled(label: label, description: description, child: Wrap(spacing: 6, runSpacing: 4, children: [
          for (final option in options)
            _Described(
              description: option.description,
              child: FilterChip(
                key: ValueKey('question-${field.name}-${option.value}'),
                label: Text(shortText(option.title) ?? shortText(option.value) ?? '?'),
                selected: chosen.contains(option.value),
                onSelected: (on) {
                  // What is chosen now, which a tap since this was built may have changed.
                  final chosen = [...(_values[field.name] as List<String>?) ?? const <String>[]];
                  on ? chosen.add(option.value) : chosen.remove(option.value);
                  // The answer keeps the agent's order.
                  final ordered = [for (final each in options) if (chosen.contains(each.value)) each.value];
                  set(ordered.isEmpty ? null : ordered);
                },
              ),
            ),
        ]));
      case AcpUnsupportedField(:final type):
        return _Labelled(
          label: label,
          description: description,
          child: Text('A question of a kind Canoa cannot show (${shortText(type) ?? 'no type'}), so it is left unanswered.',
              style: Theme.of(context).textTheme.labelSmall),
        );
    }
  }

  @override
  Widget build(BuildContext context) {
    final request = _question.request;
    final problem = _problem;
    final more = widget.session.pendingQuestions.length - 1;
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Card(
        key: const Key('question-card'),
        child: ConstrainedBox(
          constraints: BoxConstraints(maxHeight: MediaQuery.sizeOf(context).height * 0.55),
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(12),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              Row(children: [
                const Icon(Icons.help_outline),
                const SizedBox(width: 12),
                Expanded(child: Text('${shortText(widget.session.agent) ?? 'The agent'} asks', style: theme.textTheme.labelMedium)),
              ]),
              if (request.title != null) Text(shortText(request.title) ?? '', style: theme.textTheme.titleSmall),
              SelectableText(_plain(request.message)),
              if (request.description != null) Text(_plain(request.description!), style: theme.textTheme.bodySmall),
              for (final field in request.fields) Padding(padding: const EdgeInsets.only(top: 8), child: _field(field)),
              if (problem != null)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(problem, style: theme.textTheme.labelSmall?.copyWith(fontWeight: FontWeight.w600)),
                ),
              if (more > 0) Text('$more more after this', style: theme.textTheme.labelSmall),
              const SizedBox(height: 8),
              Wrap(alignment: WrapAlignment.end, spacing: 8, children: [
                TextButton(onPressed: () => _answer(const AcpElicitCancel()), child: const Text('Cancel')),
                TextButton(onPressed: () => _answer(const AcpElicitDecline()), child: const Text('Decline')),
                FilledButton(
                  onPressed: problem == null ? () => _answer(AcpElicitAccept(Map.of(_values))) : null,
                  child: const Text('Send answer'),
                ),
              ]),
            ]),
          ),
        ),
      ),
    );
  }
}

/// A choice, with the agent's description of it as a tooltip when it gave one.
class _Described extends StatelessWidget {
  const _Described({required this.description, required this.child});

  final String? description;
  final Widget child;

  @override
  Widget build(BuildContext context) =>
      description == null ? child : Tooltip(message: _plain(description!), child: child);
}

class _Labelled extends StatelessWidget {
  const _Labelled({required this.label, required this.child, this.description});

  final String label;
  final String? description;
  final Widget child;

  @override
  Widget build(BuildContext context) => Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
    Text(label, style: Theme.of(context).textTheme.labelLarge),
    if (description != null) Text(description!, style: Theme.of(context).textTheme.bodySmall),
    const SizedBox(height: 4),
    child,
  ]);
}

extension on String {
  String? get nullIfEmpty => isEmpty ? null : this;
}
