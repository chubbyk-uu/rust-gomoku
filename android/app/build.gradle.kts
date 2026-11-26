import org.gradle.api.tasks.Exec
import java.util.Properties

plugins {
    id("com.android.application")
}

val releaseSigningPropertiesFile = file(
    System.getenv("RUST_GOMOKU_ANDROID_SIGNING_PROPERTIES")
        ?: "${System.getProperty("user.home")}/.android/rust-gomoku-release.properties"
)
val releaseSigningProperties = Properties()
val hasReleaseSigning = releaseSigningPropertiesFile.isFile
val releaseSigningPropertiesPath = releaseSigningPropertiesFile.absolutePath

if (hasReleaseSigning) {
    releaseSigningPropertiesFile.inputStream().use { releaseSigningProperties.load(it) }
}

fun releaseSigningProperty(name: String): String {
    return releaseSigningProperties.getProperty(name)
        ?: throw GradleException("Missing `$name` in ${releaseSigningPropertiesFile.absolutePath}")
}

android {
    namespace = "io.github.chubbykuu.rustgomoku"
    compileSdk = 36
    ndkVersion = "29.0.14206865"
    buildToolsVersion = "36.0.0"

    defaultConfig {
        applicationId = "io.github.chubbykuu.rustgomoku"
        minSdk = 26
        targetSdk = 36
        versionCode = 19
        versionName = "0.1.19"

        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(releaseSigningProperty("storeFile"))
                storePassword = releaseSigningProperty("storePassword")
                keyAlias = releaseSigningProperty("keyAlias")
                keyPassword = releaseSigningProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    androidResources {
        // Mix9svq weights are already LZ4-compressed; store uncompressed in APK
        // for faster asset extraction and AssetFileDescriptor length checks.
        noCompress += "lz4"
    }

    sourceSets {
        named("main") {
            jniLibs.directories.add("build/generated/rustJniLibs")
            assets.directories.add("build/generated/nnueAssets")
        }
    }
}

val stageNnueAssets by tasks.registering(Sync::class) {
    val repositoryRoot = rootProject.projectDir.parentFile
    val nnueSourceDir = file(
        System.getenv("RUST_GOMOKU_NNUE_DIR")
            ?: repositoryRoot.resolve("opponent/rapfi-networks/mix9svq").absolutePath
    )
    val outputDirectory = layout.buildDirectory.dir("generated/nnueAssets/nnue")

    from(nnueSourceDir) {
        include(
            "mix9svqfreestyle_bsmix.bin.lz4",
            "mix9svqrenju_bs15_black.bin.lz4",
            "mix9svqrenju_bs15_white.bin.lz4",
        )
    }
    into(outputDirectory)
    doLast {
        val stagedDir = outputDirectory.get().asFile
        val expected = listOf(
            "mix9svqfreestyle_bsmix.bin.lz4",
            "mix9svqrenju_bs15_black.bin.lz4",
            "mix9svqrenju_bs15_white.bin.lz4",
        )
        val missing = expected.filter { !stagedDir.resolve(it).isFile }
        if (missing.isNotEmpty()) {
            logger.warn(
                "NNUE weight files missing from ${nnueSourceDir.absolutePath}: $missing. " +
                    "APK will fall back to classical evaluation unless weights are provided."
            )
        }
    }
}

val buildRustArm64 by tasks.registering(Exec::class) {
    val repositoryRoot = rootProject.projectDir.parentFile
    val outputDirectory = layout.buildDirectory.dir("generated/rustJniLibs")

    workingDir(repositoryRoot)
    inputs.files(
        repositoryRoot.resolve("Cargo.toml"),
        repositoryRoot.resolve("Cargo.lock"),
        repositoryRoot.resolve("android/rust_bridge/Cargo.toml"),
        repositoryRoot.resolve("android/rust_bridge/Cargo.lock"),
    )
    inputs.dir(repositoryRoot.resolve("src"))
    inputs.dir(repositoryRoot.resolve("android/rust_bridge/src"))
    outputs.dir(outputDirectory)

    commandLine(
        "cargo",
        "ndk",
        "-t",
        "arm64-v8a",
        "-o",
        outputDirectory.get().asFile.absolutePath,
        "build",
        "--release",
        "--locked",
        "--manifest-path",
        "android/rust_bridge/Cargo.toml",
    )
}

tasks.named("preBuild").configure {
    dependsOn(buildRustArm64, stageNnueAssets)
}

val verifyReleaseSigning by tasks.registering {
    inputs.property("hasReleaseSigning", hasReleaseSigning)
    inputs.property("releaseSigningPropertiesPath", releaseSigningPropertiesPath)

    doLast {
        if (inputs.properties["hasReleaseSigning"] != true) {
            val path = inputs.properties["releaseSigningPropertiesPath"]
            throw GradleException(
                "Release signing is not configured. Create $path " +
                    "or set RUST_GOMOKU_ANDROID_SIGNING_PROPERTIES to a signing properties file."
            )
        }
    }
}

tasks.matching { it.name == "assembleRelease" || it.name == "bundleRelease" }.configureEach {
    dependsOn(verifyReleaseSigning)
}

val testMobileUi by tasks.registering(Exec::class) {
    val repositoryRoot = rootProject.projectDir.parentFile
    val uiLogic = repositoryRoot.resolve("android/app/src/main/assets/ui_logic.js")
    val appScript = repositoryRoot.resolve("android/app/src/main/assets/app.js")
    val appStyles = repositoryRoot.resolve("android/app/src/main/assets/app.css")
    val appPage = repositoryRoot.resolve("android/app/src/main/assets/index.html")
    val uiTests = repositoryRoot.resolve("android/app/src/test/js/ui_logic.test.cjs")

    workingDir(repositoryRoot)
    inputs.files(uiLogic, appScript, appStyles, appPage, uiTests)
    commandLine("node", uiTests.absolutePath)
}

tasks.configureEach {
    if (name == "test" || name == "testDebugUnitTest") {
        dependsOn(testMobileUi)
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.16.0")
    implementation("androidx.activity:activity:1.9.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel:2.8.7")
}
