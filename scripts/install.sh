#!/bin/sh
# Beginner installer for a prebuilt Intel Mac Tuner release.
# Run this from inside the unzipped release folder: ./install.sh
set -eu

cd "$(dirname "$0")"

if [ ! -f tunerctl ] || [ ! -f intelmactunerd ]; then
    echo "Run this from inside the unzipped release folder (tunerctl/intelmactunerd must be next to it)."
    exit 1
fi

echo "Installing Intel Mac Tuner (you may be asked for your password)..."

sudo mkdir -p /usr/local/bin
sudo cp tunerctl intelmactunerd /usr/local/bin/
sudo chmod +x /usr/local/bin/tunerctl /usr/local/bin/intelmactunerd

# Install and start the daemon (runs at boot).
sudo /usr/local/bin/tunerctl install

# Menu bar app.
if [ -d "Intel Mac Tuner.app" ]; then
    rm -rf "/Applications/Intel Mac Tuner.app" 2>/dev/null || sudo rm -rf "/Applications/Intel Mac Tuner.app"
    cp -R "Intel Mac Tuner.app" /Applications/ 2>/dev/null || sudo cp -R "Intel Mac Tuner.app" /Applications/
    open "/Applications/Intel Mac Tuner.app" || true
fi

# Turbo Boost kext (optional; needs macOS to allow third-party kexts).
if [ -d DisableTurbo.kext ]; then
    sudo /usr/local/bin/tunerctl install-turbo DisableTurbo.kext \
        || echo "Turbo kext not installed (optional). You can set it up later."
fi

echo
echo "Done. The menu bar icon should appear at the top-right."
echo "If the app doesn't open, right-click it in /Applications and choose Open,"
echo "or allow it in System Settings > Privacy & Security."
