#!/bin/sh
# Installs Openhub from a release, and offers to install its GNOME Shell
# extension too. Put it in the folder with the release's
# openhub-<version>-x86_64.flatpak and
# openhub@cjts15.github.io.shell-extension.zip, then run: sh install.sh
#
# Flatpak bundles can't run anything when installed, so this script does
# what would otherwise be done by hand. Everything goes into your user
# installation; nothing needs sudo.
set -eu

APP_ID=io.github.cjts15.Openhub
UUID=openhub@cjts15.github.io
DIR=$(cd "$(dirname "$0")" && pwd)

if ! command -v flatpak >/dev/null 2>&1; then
    echo "Flatpak isn't installed. Install it first: https://flathub.org/setup" >&2
    exit 1
fi

bundle=$(ls -1 "$DIR"/openhub-*-x86_64.flatpak 2>/dev/null | sort -V | tail -n 1)
if [ -z "$bundle" ]; then
    echo "No openhub-<version>-x86_64.flatpak next to this script." >&2
    exit 1
fi

echo "Installing the GNOME 51 runtime from Flathub…"
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive flathub org.gnome.Platform//51

echo "Installing $(basename "$bundle")…"
flatpak install --user --noninteractive --bundle "$bundle"

zip="$DIR/$UUID.shell-extension.zip"
if [ -f "$zip" ] && command -v gnome-extensions >/dev/null 2>&1; then
    printf 'Also install the top-bar extension (batteries, Bluetooth and sound in the top bar)? [Y/n] '
    # Read from the terminal, so the prompt works when the script is piped
    # too; without one, the answer is yes.
    answer=y
    if (exec </dev/tty) 2>/dev/null; then
        read -r answer </dev/tty || answer=y
    else
        echo
    fi
    case "$answer" in
    [Nn]*)
        echo "Skipped the extension. You can run this script again to add it."
        ;;
    *)
        gnome-extensions install --force "$zip"
        # GNOME Shell only sees a new extension after logging back in, so
        # turn it on in the settings it reads at startup.
        current=$(gsettings get org.gnome.shell enabled-extensions)
        case "$current" in
        *"'$UUID'"*) ;;
        "@as []" | "[]") gsettings set org.gnome.shell enabled-extensions "['$UUID']" ;;
        *) gsettings set org.gnome.shell enabled-extensions "${current%]}, '$UUID']" ;;
        esac
        echo "Extension installed. Log out and back in once to see it in the top bar."
        ;;
    esac
fi

echo "Done. Open Openhub from your apps, or run: flatpak run $APP_ID"
