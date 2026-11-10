import org.gradle.api.tasks.testing.Test
import org.gradle.testing.jacoco.plugins.JacocoTaskExtension
import org.gradle.testing.jacoco.tasks.JacocoReport

val nolaAndroidAbis = System.getenv("NOLA_ANDROID_ABIS")
    ?.split(',', ' ', ';')
    ?.map(String::trim)
    ?.filter(String::isNotEmpty)
    ?.toSet()
    ?.takeIf(Set<String>::isNotEmpty)
    ?: setOf("arm64-v8a", "x86_64")

plugins {
    jacoco
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
    alias(libs.plugins.hilt)
    alias(libs.plugins.kotlin.serialization)
}

jacoco {
    toolVersion = "0.8.12"
}

tasks.withType<Test>().configureEach {
    extensions.configure(JacocoTaskExtension::class.java) {
        isIncludeNoLocationClasses = true
        excludes = listOf("jdk.internal.*")
    }
}

tasks.register<JacocoReport>("jacocoDebugUnitTestReport") {
    dependsOn("testDebugUnitTest")

    reports {
        xml.required.set(true)
        html.required.set(true)
        csv.required.set(false)
    }

    val generatedClasses = listOf(
        "**/R.class",
        "**/R$*.class",
        "**/BuildConfig.*",
        "**/Manifest*.*",
        "**/*Test*.*",
        "**/*_Factory.*",
        "**/*_HiltModules*.*",
        "**/Hilt_*.*"
    )
    classDirectories.setFrom(
        files(
            fileTree("$buildDir/tmp/kotlin-classes/debug") { exclude(generatedClasses) },
            fileTree("$buildDir/intermediates/javac/debug/classes") { exclude(generatedClasses) }
        )
    )
    sourceDirectories.setFrom(files("src/main/java"))
    executionData.setFrom(
        fileTree(buildDir) {
            include("jacoco/testDebugUnitTest.exec")
            include("outputs/unit_test_code_coverage/debugUnitTest/testDebugUnitTest.exec")
        }
    )
}

android {
    namespace = "com.nola"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.nola"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "1.0.0"
        val updateDistribution = providers.gradleProperty("nolaDistribution").getOrElse("direct")
        require(updateDistribution in listOf("direct", "store")) { "nolaDistribution must be direct or store" }
        buildConfigField("boolean", "STORE_MANAGED_UPDATES", (updateDistribution == "store").toString())

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            abiFilters += nolaAndroidAbis
        }
        externalNativeBuild {
            cmake {
                cppFlags += "-std=c++17"
                arguments += "-DANDROID_STL=c++_static"
                arguments += "-DBUILD_SHARED_LIBS=OFF"
                arguments += "-DLLAMA_BUILD_APP=OFF"
                arguments += "-DLLAMA_BUILD_COMMON=ON"
                arguments += "-DLLAMA_BUILD_EXAMPLES=OFF"
                arguments += "-DLLAMA_BUILD_SERVER=OFF"
                arguments += "-DLLAMA_BUILD_TESTS=OFF"
                arguments += "-DLLAMA_BUILD_TOOLS=OFF"
                arguments += "-DLLAMA_OPENSSL=OFF"
                arguments += "-DGGML_BACKEND_DL=OFF"
                arguments += "-DGGML_CPU_ALL_VARIANTS=OFF"
                arguments += "-DGGML_LLAMAFILE=OFF"
                arguments += "-DGGML_NATIVE=OFF"
                arguments += "-DGGML_OPENMP=OFF"
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
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

    buildFeatures {
        buildConfig = true
        compose = true
    }

    val updateManifest = if (providers.gradleProperty("nolaDistribution").getOrElse("direct") == "store")
        "src/storeUpdate/AndroidManifest.xml" else "src/directUpdate/AndroidManifest.xml"
    sourceSets.getByName("debug").manifest.srcFile(updateManifest)
    sourceSets.getByName("release").manifest.srcFile(updateManifest)

    sourceSets.getByName("main").assets.srcDir(
        rootProject.projectDir.parentFile.parentFile.resolve("apps/ios/SharedPrompts")
    )

    sourceSets.getByName("test").resources.srcDir(
        rootProject.projectDir.parentFile.parentFile.resolve("tests/fixtures")
    )
    sourceSets.getByName("test").resources.srcDir(
        rootProject.projectDir.parentFile.parentFile.resolve("core/tests/fixtures")
    )
    sourceSets.getByName("test").resources.srcDir(
        rootProject.projectDir.parentFile.parentFile.resolve("core/fixtures/host_protocol")
    )
    sourceSets.getByName("androidTest").assets.srcDir(
        rootProject.projectDir.parentFile.parentFile.resolve("core/tests/fixtures")
    )

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
        }
    }
}

dependencies {
    implementation(libs.lifecycle.process)
    // Generated from the pinned optional Tailcat module; never downloaded by Gradle.
    implementation(fileTree(rootProject.projectDir.parentFile.parentFile.resolve("transport/tailcat/build/android")) { include("*.aar") })
    // Compose BOM
    val composeBom = platform(libs.compose.bom)
    implementation(composeBom)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material.icons)
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    // Hilt
    implementation(libs.hilt.android)
    ksp(libs.hilt.compiler)
    implementation(libs.hilt.navigation.compose)

    // Room
    implementation(libs.room.runtime)
    implementation(libs.room.ktx)
    ksp(libs.room.compiler)

    // Ktor
    implementation(libs.ktor.client.core)
    implementation(libs.ktor.client.cio)
    implementation(libs.ktor.client.content.negotiation)
    implementation(libs.ktor.serialization.json)
    implementation(libs.ktor.client.logging)

    // DataStore
    implementation(libs.datastore.preferences)

    // Lifecycle
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)

    // Navigation
    implementation(libs.navigation.compose)

    // Activity
    implementation(libs.activity.compose)

    // Core
    implementation(libs.core.ktx)

    // Markdown rendering
    implementation(libs.markwon.core)
    implementation(libs.markwon.ext.tables)
    implementation(libs.markwon.ext.latex)
    implementation(libs.markwon.ext.strikethrough)
    implementation(libs.markwon.inline.parser)

    // Local PDF text extraction for chat attachments
    implementation("com.tom-roush:pdfbox-android:2.0.27.0")

    // Health Connect
    implementation(libs.health.connect.client)

    // Coroutines
    implementation(libs.coroutines.core)
    implementation(libs.coroutines.android)

    // WorkManager
    implementation(libs.work.runtime)
    implementation(libs.work.hilt)
    ksp(libs.work.hilt.compiler)

    // Third-party auth dependencies are disabled; login uses email/password only.
    // implementation(libs.credentials)
    // implementation(libs.credentials.play.services.auth)
    // implementation(libs.googleid)
    // implementation(libs.browser)

    // Testing
    testImplementation(libs.junit)
    testImplementation(kotlin("test-junit"))
    testImplementation(libs.coroutines.test)
    testImplementation("io.ktor:ktor-client-mock:2.3.12")
    androidTestImplementation(composeBom)
    androidTestImplementation(libs.compose.ui.test)
    androidTestImplementation(libs.coroutines.test)
    androidTestImplementation(libs.room.testing)
}

val buildRustCore by tasks.registering(org.gradle.api.tasks.Exec::class) {
    val coreDir = rootProject.projectDir.parentFile.parentFile.resolve("core")
    val outputDir = layout.projectDirectory.dir("src/main/cpp/rust")
    workingDir(coreDir)
    commandLine("bash", "scripts/build_android.sh", outputDir.asFile.absolutePath)
    // Cargo tracks include_str!/include_bytes!, build scripts, toolchains and
    // environment inputs. A second Gradle source whitelist can silently skip
    // changed embedded resources. Always ask Cargo; it keeps compilation fresh.
    outputs.upToDateWhen { false }
    outputs.dir(outputDir)
}

tasks.configureEach {
    if (name.startsWith("configureCMake")) {
        dependsOn(buildRustCore)
    }
}

// Store distribution must not acquire sideload privileges from a dependency.
tasks.configureEach {
    if (providers.gradleProperty("nolaDistribution").getOrElse("direct") == "store" &&
        name.startsWith("process") && name.endsWith("MainManifest")) {
        doLast {
            val manifests = outputs.files.asFileTree.matching { include("**/AndroidManifest.xml") }.files
            check(manifests.isNotEmpty()) { "Store update permission audit found no merged manifest" }
            check(manifests.none { it.readText().contains("android.permission.REQUEST_INSTALL_PACKAGES") }) {
                "Store distribution cannot request APK installation permission"
            }
        }
    }
}
