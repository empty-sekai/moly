# `rustls-platform-verifier` is reached from Rust through JNI, which ProGuard
# cannot see; keep its classes when minification is enabled.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
