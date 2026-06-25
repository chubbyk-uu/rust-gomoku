import org.gradle.api.tasks.Exec

plugins {
    id("com.android.application")
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
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += "arm64-v8a"
        }
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

    sourceSets {
        named("main") {
            jniLibs.directories.add("build/generated/rustJniLibs")
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
    dependsOn(buildRustArm64)
}

dependencies {
    implementation("androidx.webkit:webkit:1.16.0")
}
