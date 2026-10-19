# moly for Android

This directory packages the game as a native Android app: one native library
holds both the game and its server, the server runs on a thread inside the same
process, and assets stream from the public store into the app's private cache
on first run.

## Requirements

- Rust with the `aarch64-linux-android` target:
  `rustup target add aarch64-linux-android`.
- `cargo-ndk`: `cargo install cargo-ndk`.
- Android SDK with platform 34, build-tools 34 and CMake 3.22, found through
  `ANDROID_HOME` (or `ANDROID_SDK_ROOT`) or `sdk.dir` in
  `android/local.properties`.
- NDK r27d under the SDK, as `sdkmanager "ndk;27.3.13750724"` installs it:
  Gradle looks the `ndkVersion` up there, and `cargo ndk` finds it through
  `ANDROID_NDK_HOME`.
- A JDK from 17 to 21 (Gradle 8.6 and AGP 8.4).

## Build

From the repository root, build the native library into the project's
`jniLibs` directory:

```sh
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

The Kotlin component of `rustls-platform-verifier` is resolved from
`Cargo.lock`, so the Rust crate and its Android part always share a version.

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
