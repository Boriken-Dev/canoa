import 'layout_state.dart';

/// Where nothing is kept on disk, the layout lasts while the app runs.
LayoutBackend platformLayoutBackend({String? configPath, String name = 'layout.json'}) => MemoryLayoutBackend();
