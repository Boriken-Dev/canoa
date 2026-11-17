enum InputDeviceType {
  builtIn,
  wiredHeadset,
  lineIn,
  bluetoothSco,
  bluetoothA2dp,
  bluetoothLe,
  usb,
  hdmi,
  airPlay,
  thunderbolt,
  displayPort,
  unknown;

  static InputDeviceType fromString(String? value) {
    return InputDeviceType.values.firstWhere(
      (e) => e.name == value,
      orElse: () => InputDeviceType.unknown,
    );
  }
}
