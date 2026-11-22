import 'package:flutter/widgets.dart';

import '../../surfaces/markdown_section_viewer.dart';
import 'file_sample.dart';
import 'file_viewer.dart';

/// A Markdown file as its frontmatter, an outline and a card per section. It is the first viewer of a file that has frontmatter or
/// three headings, or that was opened at a section; any other Markdown file reads as plain text until the person chooses this view.
class MarkdownViewer extends FileViewer {
  const MarkdownViewer();

  static const String viewerId = 'markdown';

  @override
  String get id => viewerId;
  @override
  String get labelKey => 'files.viewer.markdown';
  @override
  Set<String> get mimeTypes => const {'text/markdown'};
  @override
  int get priority => 10;
  @override
  bool get opensSections => true;

  @override
  bool canShow(FileSample sample) => sample.isText;

  @override
  bool isDefaultFor(FileViewRequest request) {
    final text = request.sample.text;
    if (text == null) return false;
    return request.sample.memo('markdown structured', () => parseMarkdownSections(text).isStructured);
  }

  @override
  Widget build(BuildContext context, FileViewRequest request) =>
      MarkdownSectionViewer(key: ValueKey('markdown ${request.sample.path}'), text: request.sample.text ?? '', initialSection: request.section);
}
