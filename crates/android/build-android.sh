#!/usr/bin/env bash
# Build the Android app for arm64 phones and the x86_64 emulator:
#   target/android/grooph.aab  App Bundle to upload to Google Play
#   target/android/grooph.apk  the same app as one APK, to install directly
#
# Needs: the Android SDK (ANDROID_HOME) with platforms;android-36 and a
# build-tools, the NDK (ANDROID_NDK_HOME), cargo-ndk, Java and bundletool.
#
# Environment:
#   BUNDLETOOL         path to bundletool-all.jar
#   KEYSTORE           keystore with the upload key
#   KEYSTORE_PASSWORD  its password
#   KEY_ALIAS          the key's alias in it
#   KEY_PASSWORD       the key's password (default: KEYSTORE_PASSWORD)
#   VERSION_CODE       Play's version number, higher on every upload (default: 1)
set -euo pipefail

# API levels. 29: midir needs AMidi (libamidi), which Android 10 added.
# 36: what Google Play requires new apps and updates to target (Aug 2026).
MIN_SDK=29
TARGET_SDK=36
ABIS=(arm64-v8a x86_64)

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/target/android"
sdk="${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ndk="${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the Android NDK}"
build_tools="$sdk/build-tools/$(ls "$sdk/build-tools" | sort -V | tail -1)"
android_jar="$sdk/platforms/android-$TARGET_SDK/android.jar"
key_password="${KEY_PASSWORD:-$KEYSTORE_PASSWORD}"
version_code="${VERSION_CODE:-1}"
version_name="$(cargo metadata --manifest-path "$root/Cargo.toml" --no-deps --format-version 1 |
  jq -r '.packages[] | select(.name == "grooph-android") | .version')"
readelf="$(ls "$ndk"/toolchains/llvm/prebuilt/*/bin/llvm-readelf | head -1)"

[[ -f "$android_jar" ]] || { echo "Missing $android_jar (sdkmanager \"platforms;android-$TARGET_SDK\")" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out/module/manifest"

echo "Building the native library ($version_name, version code $version_code)"
abi_args=()
for abi in "${ABIS[@]}"; do abi_args+=(-t "$abi"); done
(cd "$root" && cargo ndk "${abi_args[@]}" --platform "$MIN_SDK" -o "$out/module/lib" \
  build -p grooph-android --release)

for abi in "${ABIS[@]}"; do
  lib="$out/module/lib/$abi/libgrooph_android.so"
  # Google Play requires native code built for 16 KB memory pages.
  if "$readelf" -lW "$lib" | awk '$1 == "LOAD" && $NF != "0x4000" && $NF != "0x10000" { bad = 1 } END { exit !bad }'; then
    echo "$lib is not aligned to 16 KB pages" >&2
    exit 1
  fi
  # Ship the C++ runtime only if something links it.
  if "$readelf" -d "$lib" | grep -q 'libc++_shared.so'; then
    triple=$([[ $abi == arm64-v8a ]] && echo aarch64-linux-android || echo x86_64-linux-android)
    cp "$(ls "$ndk"/toolchains/llvm/prebuilt/*/sysroot/usr/lib/"$triple"/libc++_shared.so)" "$out/module/lib/$abi/"
  fi
done

echo "Linking the manifest and resources"
"$build_tools/aapt2" compile --dir "$here/res" -o "$out/res.zip"
"$build_tools/aapt2" link --proto-format -o "$out/linked.zip" -I "$android_jar" \
  --manifest "$here/AndroidManifest.xml" --min-sdk-version "$MIN_SDK" \
  --target-sdk-version "$TARGET_SDK" --version-code "$version_code" \
  --version-name "$version_name" --auto-add-overlay "$out/res.zip"
(cd "$out/module" && unzip -q "$out/linked.zip" && mv AndroidManifest.xml manifest/)
(cd "$out/module" && zip -qr "$out/base.zip" .)

echo "Building the App Bundle"
java -jar "$BUNDLETOOL" build-bundle --modules="$out/base.zip" --output="$out/grooph.aab"
jarsigner -keystore "$KEYSTORE" -storepass "$KEYSTORE_PASSWORD" -keypass "$key_password" \
  "$out/grooph.aab" "$KEY_ALIAS" > /dev/null

echo "Building the APK"
java -jar "$BUNDLETOOL" build-apks --bundle="$out/grooph.aab" --output="$out/grooph.apks" \
  --mode=universal --ks="$KEYSTORE" --ks-pass="pass:$KEYSTORE_PASSWORD" \
  --ks-key-alias="$KEY_ALIAS" --key-pass="pass:$key_password"
unzip -qo "$out/grooph.apks" universal.apk -d "$out"
mv "$out/universal.apk" "$out/grooph.apk"

echo "Done: $out/grooph.aab and $out/grooph.apk"
