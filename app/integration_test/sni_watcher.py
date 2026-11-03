#!/usr/bin/env python3
"""A minimal StatusNotifier watcher and host, for running the desktop test on a Linux session with no tray of its own.

It owns org.kde.StatusNotifierWatcher on the session bus and records each item that registers, as a desktop's panel does,
and draws nothing: the test picks the item's menu entries over D-Bus, as a panel would on a person's click. Run it
before the test, and end it after: python3 sni_watcher.py. Needs python3-dbus and python3-gobject.
"""
import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

INTERFACE = "org.kde.StatusNotifierWatcher"


class Watcher(dbus.service.Object):
    def __init__(self, bus):
        super().__init__(bus, "/StatusNotifierWatcher")
        self.items = []

    @dbus.service.method(INTERFACE, in_signature="s", out_signature="", sender_keyword="sender")
    def RegisterStatusNotifierItem(self, service, sender=None):
        # An item names itself by bus name or by object path; a path is on the caller's own connection.
        item = sender + service if service.startswith("/") else service + "/StatusNotifierItem"
        if item not in self.items:
            self.items.append(item)
            self.StatusNotifierItemRegistered(item)

    @dbus.service.method(INTERFACE, in_signature="s", out_signature="")
    def RegisterStatusNotifierHost(self, service):
        pass

    @dbus.service.signal(INTERFACE, signature="s")
    def StatusNotifierItemRegistered(self, item):
        pass

    @dbus.service.signal(INTERFACE, signature="")
    def StatusNotifierHostRegistered(self):
        pass

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, interface, name):
        return self.GetAll(interface)[name]

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return {
            "RegisteredStatusNotifierItems": dbus.Array(self.items, signature="s"),
            "IsStatusNotifierHostRegistered": dbus.Boolean(True),
            "ProtocolVersion": dbus.Int32(0),
        }


def main():
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    name = dbus.service.BusName(INTERFACE, bus, do_not_queue=True)
    watcher = Watcher(bus)
    watcher.StatusNotifierHostRegistered()
    print("watching", name.get_name(), flush=True)
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
