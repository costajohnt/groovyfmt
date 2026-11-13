import org.jetbrains.intellij.platform.gradle.IntelliJPlatformType
import org.jetbrains.intellij.platform.gradle.TestFrameworkType
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.dsl.KotlinVersion

plugins {
    id("org.jetbrains.kotlin.jvm") version "2.3.21"
    id("org.jetbrains.intellij.platform") version "2.20.1"
}

group = "io.github.costajohnt"
// The plugin carries groovyfmt's version, so it downloads the release it was built with.
version = file("../../Cargo.toml").readLines().first { it.startsWith("version = ") }.split('"')[1]

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

// The oldest IDE the plugin runs in: LSP4IJ's own minimum, IntelliJ Platform 2024.2.
val minBuild = "242"
val lsp4ij = "com.redhat.devtools.lsp4ij:0.21.0"

dependencies {
    intellijPlatform {
        intellijIdeaCommunity("2024.2.6")
        plugin(lsp4ij)
        // only so the tests can check that the IDE's own Groovy formatter keeps Reformat Code
        bundledPlugin("org.intellij.groovy")
        testFramework(TestFrameworkType.Platform)
    }
    testImplementation("junit:junit:4.13.2")
    // the 2024.2 test framework needs it at runtime
    testRuntimeOnly("org.opentest4j:opentest4j:1.3.0")
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        jvmTarget = JvmTarget.JVM_21
        // IntelliJ 2024.2 bundles the Kotlin 1.9 standard library, which the plugin uses at run time
        apiVersion = KotlinVersion.KOTLIN_1_9
        languageVersion = KotlinVersion.KOTLIN_1_9
    }
}

intellijPlatform {
    pluginConfiguration {
        version = project.version.toString()
        ideaVersion {
            sinceBuild = minBuild
            untilBuild = provider { null }
        }
        // the first section of CHANGELOG.md, one list item per entry
        changeNotes = provider {
            file("CHANGELOG.md").readLines().dropWhile { !it.startsWith("## ") }.drop(1).takeWhile { !it.startsWith("## ") }
                .filter { it.startsWith("- ") }
                .joinToString("", "<ul>", "</ul>") { "<li>${it.removePrefix("- ").replace("&", "&amp;").replace("<", "&lt;")}</li>" }
        }
    }
    pluginVerification {
        ides {
            create(IntelliJPlatformType.IntellijIdeaCommunity, "2024.2.6")
            create(IntelliJPlatformType.IntellijIdeaCommunity, "2025.2.6.4")
            create(IntelliJPlatformType.IntellijIdea, "2026.3")
            create(IntelliJPlatformType.AndroidStudio, "2026.2.2.8")
        }
    }
    // Marketplace signing and publishing, for release.yml only. A secret that isn't set comes
    // through as an empty variable, which counts as unset.
    fun secret(name: String) = providers.environmentVariable(name).filter { it.isNotBlank() }
    signing {
        certificateChain = secret("JETBRAINS_CERTIFICATE_CHAIN")
        privateKey = secret("JETBRAINS_PRIVATE_KEY")
        password = secret("JETBRAINS_PRIVATE_KEY_PASSWORD")
    }
    publishing {
        token = secret("JETBRAINS_MARKETPLACE_TOKEN")
    }
    buildSearchableOptions = false
}

// A release build embeds the SHA-256 of each release archive, hashed from the archives the
// release workflow built (-PgroovyfmtArchives=<dir>). Any other build embeds none and so never
// downloads groovyfmt.
val archives = providers.gradleProperty("groovyfmtArchives")
val checksums = tasks.register<JavaExec>("groovyfmtChecksums") {
    onlyIf { archives.isPresent }
    val out = layout.buildDirectory.dir("generated/checksums")
    outputs.dir(out)
    inputs.property("version", project.version)
    archives.orNull?.let { inputs.dir(it) }
    // the compiled Kotlin only: the full main output includes the resources this task feeds
    classpath = files(tasks.compileKotlin.flatMap { it.destinationDirectory }) + sourceSets.main.get().compileClasspath
    mainClass = "io.github.costajohnt.groovyfmt.intellij.BinaryKt"
    argumentProviders.add(CommandLineArgumentProvider {
        listOf(project.version.toString(), archives.get(), out.get().file("groovyfmt-checksums.properties").asFile.path)
    })
    doFirst { out.get().asFile.mkdirs() }
}
val pluginVersion = project.version.toString()
tasks.named<ProcessResources>("processResources") {
    inputs.property("version", pluginVersion)
    filesMatching("groovyfmt-version.txt") { expand("version" to pluginVersion) }
    if (archives.isPresent) from(checksums)
}

tasks.test {
    // The integration test runs the groovyfmt binary named here (an absolute path).
    providers.environmentVariable("GROOVYFMT_BIN").orNull?.let { systemProperty("groovyfmt.bin", it) }
    // No groovyfmt on PATH in the tests: only the one a test sets up is found.
    environment("PATH", "/usr/bin:/bin")
}
