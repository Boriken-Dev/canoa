import 'diff_viewer.dart';
import 'file_viewer.dart';
import 'image_viewer.dart';
import 'info_viewer.dart';
import 'markdown_viewer.dart';
import 'structured_viewer.dart';
import 'text_viewer.dart';

/// A registry holding the viewers the app ships: text, Markdown sections, image, a JSON, YAML and TOML tree, a diff, and the notice
/// a file nothing can draw gets. An extension's viewer is added to it with [ViewerRegistry.register], beside these and by the same rules.
ViewerRegistry standardViewerRegistry({ViewerChoices? choices}) => ViewerRegistry(
  choices: choices,
  viewers: const [TextViewer(), MarkdownViewer(), ImageViewer(), StructuredViewer(), DiffViewer(), InfoViewer()],
);
