/// What an installed skill's page shows under its card: the Store's entry for it when it came from a source the Store knows, and its
/// `SKILL.md` rendered as Markdown.
library;

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../highlight/highlighted.dart';
import '../l10n/app_text.dart';
import '../profile/skill_copies.dart';
import '../surfaces/markdown_section_viewer.dart' show parseMarkdownSections;
import 'entry_details.dart';

/// The Store's entry of the skill [copy], as its source's listing shows an entry: what its `SKILL.md` says it is for, where it
/// comes from and what it is pinned to. [source] is the label of the source or catalog that holds it, [shown] where that is.
class SkillStoreEntry extends StatelessWidget {
  const SkillStoreEntry({super.key, required this.copy, required this.source, required this.shown, this.document});

  final SkillCopy copy;
  final String source, shown;

  /// The skill's `SKILL.md`, whose front matter gives the description.
  final String? document;

  @override
  Widget build(BuildContext context) {
    final description = document == null
        ? null
        : parseMarkdownSections(document!).frontmatter.where((entry) => entry.key == 'description').map((entry) => entry.value).firstOrNull;
    return DetailSection(
      key: const Key('skill-entry'),
      title: context.t('store.skill_entry'),
      children: [
        if (description != null && description.trim().isNotEmpty) DetailField(label: context.t('store.skill_field_description'), value: description.trim()),
        DetailField(label: context.t('store.skill_field_source'), value: source),
        DetailField(label: context.t('store.skill_field_address'), value: shown),
        DetailField(label: context.t('store.skill_field_version'), value: copy.ref ?? copy.digest),
        DetailField(
          label: context.t('store.skill_field_pin'),
          value: copy.pin == null
              ? context.t('store.skill_entry_unpinned', {'digest': copy.digest})
              : context.t('store.skill_entry_pin', {'pin': copy.pin}),
        ),
      ],
    );
  }
}

/// A skill's `SKILL.md` as Markdown, or a line saying there is none; [loading] is true while it is being read.
class SkillDocument extends StatelessWidget {
  const SkillDocument({super.key, required this.text, this.loading = false});

  final String? text;
  final bool loading;

  @override
  Widget build(BuildContext context) => DetailSection(
    title: context.t('store.skill_document'),
    children: [
      if (loading)
        const Padding(padding: EdgeInsets.only(top: 8), child: LinearProgressIndicator())
      else if (text == null)
        Padding(padding: const EdgeInsets.only(top: 4), child: Text(context.t('store.skill_document_none'), key: const Key('skill-document-none')))
      else
        Padding(
          padding: const EdgeInsets.only(top: 4),
          child: AgentMarkdown(key: const Key('skill-document'), text: text!, builders: highlightedMarkdown),
        ),
    ],
  );
}

/// [SkillDocument] for text that is read when the page opens, such as a skill an overlay provides.
class ReadSkillDocument extends StatefulWidget {
  const ReadSkillDocument({super.key, required this.read});

  final Future<String?> Function() read;

  @override
  State<ReadSkillDocument> createState() => _ReadSkillDocumentState();
}

class _ReadSkillDocumentState extends State<ReadSkillDocument> {
  late final Future<String?> _text = widget.read();

  @override
  Widget build(BuildContext context) => FutureBuilder<String?>(
    future: _text,
    builder: (context, snapshot) => SkillDocument(
      text: snapshot.data,
      loading: snapshot.connectionState != ConnectionState.done,
    ),
  );
}
