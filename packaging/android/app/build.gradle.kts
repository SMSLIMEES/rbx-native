// The Android build of rbxview: one module, no dependencies. The viewer is
// the same wasm/WebGPU build the browser version runs (see
// `scripts/build-web.sh`); the `syncWebAssets` task copies it into the APK's
// assets, and `MainActivity` serves it to a WebView from an intercepted
// origin, standing in for `rbxview --serve` (see `AssetProxy.kt`).
plugins {
    id("com.android.application")
    kotlin("android")
}

/** `crates/rbx_viewer/web`: the page, and the `pkg` `build-web.sh` made. */
val webRoot = rootProject.file("../../crates/rbx_viewer/web")

/**
 * Copies the built page into `src/main/assets/web`, which is what `LocalServer`
 * serves out of the APK. Fails loudly when the wasm bundle is not there yet,
 * rather than shipping a page that cannot start.
 */
val syncWebAssets by tasks.registering(Copy::class) {
    from(webRoot) {
        include("index.html")
        include("pkg/**")
        into("web")
    }
    into(layout.projectDirectory.dir("src/main/assets"))
    doFirst {
        check(webRoot.resolve("pkg/rbx_viewer.js").isFile) {
            "crates/rbx_viewer/web/pkg is missing: run scripts/build-web.sh first " +
                "(it needs the wasm32 target and wasm-bindgen-cli of the Cargo.lock version)."
        }
    }
}

android {
    namespace = "dev.rbxnative.viewer"
    compileSdk = 35
    // Pinned to what `android.yml` installs, so a local and a CI build agree.
    buildToolsVersion = "35.0.0"

    defaultConfig {
        applicationId = "dev.rbxnative.viewer"
        // WebGPU in Android's system WebView is Chrome 128+; Android 10 still
        // gets WebView updates on every device worth installing this on.
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            // Sideloaded, not a store listing: unsigned is expected.
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    androidResources {
        // The wasm module is already dense; compressed in the APK it costs a
        // decompress on every start, and `application/wasm` does not mind.
        noCompress += "wasm"
    }
}

// `src/main/assets` is an ordinary source directory, so the only thing left
// to arrange is that the copy lands before the assets are packaged.
tasks.named("preBuild").configure { dependsOn(syncWebAssets) }

// Deliberately no dependencies: everything the app needs (`WebView`,
// `Activity`, the notification-free toasts) is in the platform, so the build
// resolves nothing but the Android Gradle Plugin itself and a CI run cannot
// break on a library update.

