import 'overlays.dart';
import 'profile_home.dart';

/// A web build installs no overlays and reads no working directory.
ProfileHome? platformProfileHome({String? configPath, OverlayManager overlays = const NoOverlays()}) => null;
