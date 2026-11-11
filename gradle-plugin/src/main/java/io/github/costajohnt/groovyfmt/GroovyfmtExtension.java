package io.github.costajohnt.groovyfmt;

import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.Property;

/** The {@code groovyfmt { }} block. */
public abstract class GroovyfmtExtension {
    /** Release to download, such as {@code 0.8.0}. Default: the plugin's own version. */
    public abstract Property<String> getVersion();

    /** A local groovyfmt binary to run instead of downloading one. */
    public abstract RegularFileProperty getBinary();

    /** Extra arguments passed to every groovyfmt run. */
    public abstract ListProperty<String> getArgs();

    /** Files and directories to work on, relative to the project directory. Default: the project directory. */
    public abstract ListProperty<String> getPaths();
}
