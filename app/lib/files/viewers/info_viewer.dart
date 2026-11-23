import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../../l10n/app_text.dart';
import 'file_sample.dart';
import 'file_viewer.dart';

/// What a file the app cannot draw says of itself: that it cannot be shown, its size and its type. It matches only a file no other
/// viewer can show: a binary file, an image too large to read, or a type nobody registered a viewer for.
class InfoViewer extends FileViewer {
  const InfoViewer();

  static const String viewerId = 'info';

  @override
  String get id => viewerId;
  @override
  String get labelKey => 'files.viewer.info';
  @override
  bool get fallback => true;

  @override
  bool canShow(FileSample sample) => true;

  @override
  bool matches(FileSample sample, String type) => true;

  @override
  Widget build(BuildContext context, FileViewRequest request) {
    final sample = request.sample;
    final size = sizeText(sample.size);
    final muted = TextStyle(color: context.chatLook.muted, fontSize: 12);
    final image = sample.kind == 'image';
    return SingleChildScrollView(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            context.t(image ? 'files.image.too_large' : 'files.binary', {'size': size}),
            key: ValueKey(image ? 'files too large' : 'files binary'),
          ),
          const SizedBox(height: 8),
          Text(context.t('files.info.type', {'type': request.type}), key: const ValueKey('files info type'), style: muted),
        ],
      ),
    );
  }
}
