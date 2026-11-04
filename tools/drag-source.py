#!/usr/bin/env python3
"""Another app with a file to drag (for tools/probe-labwc.sh): a window,
app id io.wados.Probe, that is all one label; dragging it carries the file
given, as a file list (text/uri-list), the way a file manager's would.

    python3 -I tools/drag-source.py <file>
"""
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GObject, Gtk  # noqa: E402

path = sys.argv[1]


def activate(app):
    win = Gtk.ApplicationWindow(application=app, title="Drag source")
    win.set_default_size(400, 300)
    label = Gtk.Label(label=path.rsplit("/", 1)[-1], hexpand=True, vexpand=True)
    drag = Gtk.DragSource(actions=Gdk.DragAction.COPY)

    def prepare(_source, _x, _y):
        files = Gdk.FileList.new_from_array([Gio.File.new_for_path(path)])
        value = GObject.Value(Gdk.FileList, files)
        return Gdk.ContentProvider.new_for_value(value)

    drag.connect("prepare", prepare)
    label.add_controller(drag)
    win.set_child(label)
    win.present()


app = Gtk.Application(application_id="io.wados.Probe", flags=Gio.ApplicationFlags.NON_UNIQUE)
app.connect("activate", activate)
app.run([])
