// RBX Explorer on Android: one module, no dependencies. The app is the
// wasm build of `crates/rbx_explorer` (see `scripts/build-explorer.sh`) in a
// WebView; `syncWebAssets` copies the page into the APK's assets, and
// `MainActivity` serves it from an intercepted https origin.
plugins {
    id("com.android.application")
    kotlin("android")
}

/** `crates/rbx_explorer/web`: the page, its stylesheet, and the built `pkg`. */
val webRoot = rootProject.file("../../crates/rbx_explorer/web")

val syncWebAssets by tasks.registering(Copy::class) {
    from(webRoot) {
        include("index.html")
        include("style.css")
        include("pkg/**")
        into("web")
    }
    into(layout.projectDirectory.dir("src/main/assets"))
    doFirst {
        check(webRoot.resolve("pkg/rbx_explorer.js").isFile) {
            "crates/rbx_explorer/web/pkg is missing: run scripts/build-explorer.sh first " +
                "(it needs the wasm32 target and wasm-bindgen-cli of the Cargo.lock version)."
        }
    }
}

android {
    namespace = "dev.rbxnative.explorer"
    compileSdk = 35
    buildToolsVersion = "35.0.0"

    defaultConfig {
        applicationId = "dev.rbxnative.explorer"
        // Any WebView from the last years runs wasm and ES modules; 29 is
        // what the viewer asks for too.
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
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
    // Unlike the viewer, the wasm is left compressed: most of it is the
    // embedded Roblox API dump, JSON that zips to a fraction of its size.
}

tasks.named("preBuild").configure { dependsOn(syncWebAssets) }
