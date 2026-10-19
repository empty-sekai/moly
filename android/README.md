# moly for Android

This directory packages the game as a native Android app: one native library
holds both the game and its server, the server runs on a thread inside the same
process, and assets stream from the public store into the app's private cache
on first run.

## Requirements

- Rust with the `aarch64-linux-android` target:
  `rustup target add aarch64-linux-android`.
- `cargo-ndk` 4: `cargo install cargo-ndk`.
- Android SDK with platform 37, build-tools 37.0.0 and CMake 4.1.2, found
  through `ANDROID_HOME` (or `ANDROID_SDK_ROOT`) or `sdk.dir` in
  `android/local.properties`.
- NDK r30 under the SDK (`ndk/30.0.16248370`): Gradle looks the `ndkVersion`
  up there, and `cargo ndk` must build the Rust library with the same NDK,
  found through `ANDROID_NDK_HOME`, so both sides share one C++ runtime.
- A JDK 17 or later (Gradle 9.8 and AGP 9.4).

## Build

From the repository root, build the native library into the project's
`jniLibs` directory:

```sh
ANDROID_NDK_HOME=$ANDROID_HOME/ndk/30.0.16248370 \
  cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p moly-android --release
```

Then package the APK:

```sh
cd android
./gradlew :app:assembleRelease
```

The APK lands in `android/app/build/outputs/apk/release/`. For a quick device
test, `./gradlew :app:assembleDebug` builds a debug-signed APK that
`adb install` accepts.

The Rust library is not built by Gradle: run the `cargo ndk` command again
after changing Rust code, before assembling.

## Logs

A start that refuses (no store, a missing directory, a port that cannot be
bound) writes its message to logcat under the `moly` tag:

```sh
adb logcat -s moly
```

The game's own log lines go to logcat under their module names; `adb logcat`
shows them together with the tag above.

## Notice

The Gradle wrapper and the C++ shim that makes Gradle package the C++ runtime
come from Bevy's GameActivity example, distributed under the MIT or Apache-2.0
licence; their files carry their own notices.
