#!/bin/sh
# Builds DisableTurbo.kext without Xcode: just clang from the Command Line Tools +
# the SDK's Kernel.framework headers. Produces ./DisableTurbo.kext next to the script.
set -eu

cd "$(dirname "$0")"
SRC="DisableTurbo/src/disable_turbo.c"
PLIST="DisableTurbo/Info.plist"
OUT="DisableTurbo.kext"

SDK="$(xcrun --show-sdk-path)"
KHDR="$SDK/System/Library/Frameworks/Kernel.framework/Headers"
if [ ! -d "$KHDR" ]; then
    echo "ERROR: kernel headers not found at:"
    echo "  $KHDR"
    echo "Install the Command Line Tools (xcode-select --install) or point to the SDK with:"
    echo "  sudo xcode-select -s /Library/Developer/CommandLineTools"
    exit 1
fi

echo "SDK: $SDK"
rm -rf "$OUT" build.o

clang -x c -arch x86_64 \
    -target x86_64-apple-macos10.13 \
    -std=c11 -Wall -Wextra \
    -fno-builtin -fno-common -fno-stack-protector -mkernel -nostdinc \
    -I "$KHDR" \
    -DKERNEL -DKERNEL_PRIVATE -DDRIVER_PRIVATE -DAPPLE -DNeXT \
    -c "$SRC" -o build.o

clang -arch x86_64 -nostdlib \
    -Xlinker -kext \
    -Xlinker -object_path_lto -Xlinker /dev/null \
    -lkmod -lcc_kext \
    build.o -o DisableTurbo.bin

mkdir -p "$OUT/Contents/MacOS"
cp "$PLIST" "$OUT/Contents/Info.plist"
mv DisableTurbo.bin "$OUT/Contents/MacOS/DisableTurbo"
rm -f build.o

echo "kext built: $OUT"
if command -v kextlibs >/dev/null 2>&1; then
    echo "Checking symbols (kextlibs):"
    kextlibs -undef-symbols "$OUT" 2>&1 | sed 's/^/  /' || true
fi
echo
echo "To install it: sudo tunerctl install-turbo $OUT"
echo "Manual test:"
echo "  sudo cp -R $OUT /tmp/ && sudo chown -R root:wheel /tmp/$OUT"
echo "  sudo kmutil load -p /tmp/$OUT      # or: sudo kextutil /tmp/$OUT"
echo "  kmutil showloaded | grep DisableTurbo"
