package io.github.costajohnt.groovyfmt;

import java.io.File;
import java.util.ArrayList;
import java.util.List;
import javax.inject.Inject;
import org.gradle.api.DefaultTask;
import org.gradle.api.GradleException;
import org.gradle.api.file.DirectoryProperty;
import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.Property;
import org.gradle.api.tasks.Input;
import org.gradle.api.tasks.Internal;
import org.gradle.api.tasks.Optional;
import org.gradle.api.tasks.TaskAction;
import org.gradle.api.tasks.UntrackedTask;
import org.gradle.process.ExecOperations;
import org.gradle.process.ExecResult;

/** Runs groovyfmt in one mode: {@code check}, {@code format} or {@code lint}. */
// ponytail: never up to date; groovyfmt itself takes well under a second, so caching is not worth wrong results.
@UntrackedTask(because = "groovyfmt finds its own files and configs; declaring them as inputs could make the task wrongly up to date")
public abstract class GroovyfmtTask extends DefaultTask {
    @Input
    public abstract Property<String> getMode();

    @Input
    public abstract Property<String> getVersion();

    @Internal
    public abstract RegularFileProperty getBinary();

    @Input
    public abstract ListProperty<String> getArgs();

    /** Absolute paths. */
    @Input
    public abstract ListProperty<String> getPaths();

    @Internal
    public abstract DirectoryProperty getWorkingDir();

    @Internal
    public abstract DirectoryProperty getCacheDir();

    @Input
    public abstract Property<Boolean> getOffline();

    @Input
    @Optional
    public abstract Property<String> getDownloadBaseUrl();

    @Internal
    public abstract Property<String> getTokenProperty();

    @Inject
    protected abstract ExecOperations getExecOperations();

    @TaskAction
    public void run() throws Exception {
        File exe;
        if (getBinary().isPresent()) {
            exe = getBinary().get().getAsFile();
            if (!exe.isFile()) {
                throw new GradleException("groovyfmt.binary " + exe + " does not exist");
            }
        } else {
            exe = Provisioner.provision(getVersion().get(), getCacheDir().get().getAsFile().toPath(),
                    getDownloadBaseUrl().getOrNull(), token(), getOffline().get(), getLogger()).toFile();
        }
        String mode = getMode().get();
        List<String> args = new ArrayList<>();
        if (mode.equals("check")) args.add("--check");
        if (mode.equals("lint")) args.add("--lint");
        args.addAll(getArgs().get());
        args.addAll(getPaths().get());
        ExecResult result = getExecOperations().exec(spec -> {
            spec.setExecutable(exe.getAbsolutePath());
            spec.setArgs(args);
            spec.setWorkingDir(getWorkingDir().get().getAsFile());
            spec.setIgnoreExitValue(true);
        });
        int code = result.getExitValue();
        if (code == 0) return;
        if (code == 1 && mode.equals("check")) {
            throw new GradleException("groovyfmt: the files listed above need formatting. Run the groovyfmtFormat task to fix them.");
        }
        if (code == 1 && mode.equals("lint")) {
            throw new GradleException("groovyfmt: lint found the violations listed above.");
        }
        if (code == 2) {
            throw new GradleException("groovyfmt could not run (exit code 2): see its error above. Bad arguments, a bad config file, or an unreadable file.");
        }
        throw new GradleException("groovyfmt exited with code " + code);
    }

    // ponytail: env vars are read here, at execution, so the token never lands in the configuration cache.
    private String token() {
        for (String name : new String[] {"GROOVYFMT_GITHUB_TOKEN", "GITHUB_TOKEN"}) {
            String value = System.getenv(name);
            if (value != null && !value.isEmpty()) return value;
        }
        return getTokenProperty().getOrNull();
    }
}
