#!/bin/sh
# T497 — clearing application data on Linux when the .deb package is purged.
#
# **The Windows equivalent of this is uninstall.nsh (T356).** There, the checkbox in the
# NSIS uninstaller runs inside the current user's own uninstall process, so
# `SetShellVarContext current` plus `$APPDATA`/`$LOCALAPPDATA` already means the right
# account. dpkg's postrm has no such context: it always runs as root, once, invoked by
# whichever front end did the purge (`apt`, `dpkg`, a graphical package manager) — none
# of which reliably hand this script the identity of the desktop user whose data it is.
# So every real account on the machine is checked instead of trying to guess the one.
#
# **Only on purge, and that is the whole of the "did they ask" question on this
# platform.** Debian's own distinction between `apt remove` (binaries and package
# metadata go, configuration and data stay) and `apt purge` (data goes too) is already
# the explicit two-step choice a person makes — the same choice the Windows uninstaller's
# checkbox represents, just spelled with a flag instead of a checkbox. A plain `remove`
# must leave the library, the server profiles and the place tables alone.
#
# **What this deliberately does not do: touch the secret store.** Same limitation as
# `uninstall.nsh`, for the same reason — see `src-tauri/src/commands/forget.rs`. Erasing a
# server's managed SSH key without a way to read it back first would strand that server;
# only the application itself, before it is removed, is in a position to ask.
set -e

# dpkg passes the action as $1. Anything other than "purge" — "remove", "upgrade",
# "failed-upgrade", "disappear" — must not touch data a person did not ask to lose.
if [ "$1" != "purge" ]; then
    exit 0
fi

# The path fragment `directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")`
# actually produces on Linux (crate `directories` 6.0.0, src/lin.rs): the qualifier and
# organization arguments are not used on this platform at all, only the application
# name, lower-cased and stripped of its spaces — "VRCast Studio" becomes "vrcaststudio".
# Read from the crate source rather than assumed: the shape is *not* "VRCast/VRCast
# Studio", which is what Windows and macOS use.
#
# The default is `$XDG_DATA_HOME` or, absent that, `$HOME/.local/share`. A user who has
# relocated `XDG_DATA_HOME` keeps their data here: that variable lives in a session this
# script has no access to, since dpkg gives postrm nothing but root's own environment.
# Named as a limit rather than silently missed.
#
# Every real home directory under /root and /home is checked — not only one account —
# because postrm has no reliable way to learn which desktop user ran the application,
# and on the single-user desktop machine this application targets that is usually the
# only one there. `getent passwd` is used rather than a directory listing of /home so
# that an entry with no matching login (a leftover directory) is not treated as a user.
getent passwd | while IFS=: read -r _name _pw _uid _gid _gecos home_dir _shell; do
    [ -d "$home_dir" ] || continue
    data_dir="$home_dir/.local/share/vrcaststudio"
    if [ -d "$data_dir" ]; then
        rm -rf "$data_dir"
    fi
done

exit 0
