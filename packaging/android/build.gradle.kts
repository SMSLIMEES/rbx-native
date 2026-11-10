// The Android app for rbxview. One module, and the plugin versions are the
// ones `android.yml` pins, so a release build and CI disagree about nothing.
plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
