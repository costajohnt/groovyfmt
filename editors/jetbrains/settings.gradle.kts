// Plugins and their dependencies from Maven Central first: the Plugin Portal's mirror of
// Central artifacts fails intermittently. The portal stays for plugins published only there.
pluginManagement {
    repositories {
        mavenCentral()
        gradlePluginPortal()
    }
}

rootProject.name = "groovyfmt-jetbrains"
