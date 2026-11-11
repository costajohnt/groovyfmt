package io.github.costajohnt.groovyfmt;

import java.io.File;
import java.util.List;
import java.util.stream.Collectors;
import org.gradle.api.Plugin;
import org.gradle.api.Project;
import org.gradle.api.provider.ProviderFactory;
import org.gradle.api.tasks.TaskProvider;
import org.gradle.language.base.plugins.LifecycleBasePlugin;

/**
 * Adds groovyfmtCheck, groovyfmtFormat and groovyfmtLint. Apply it to the root project only:
 * groovyfmt searches the whole directory tree, subprojects included.
 */
public class GroovyfmtPlugin implements Plugin<Project> {
    @Override
    public void apply(Project project) {
        GroovyfmtExtension ext = project.getExtensions().create("groovyfmt", GroovyfmtExtension.class);
        ext.getVersion().convention(Provisioner.pluginVersion());
        ext.getArgs().convention(List.of());
        ext.getPaths().convention(List.of());
        File projectDir = project.getProjectDir();
        ProviderFactory providers = project.getProviders();
        boolean offline = project.getGradle().getStartParameter().isOffline();
        File cacheDir = new File(project.getGradle().getGradleUserHomeDir(), "groovyfmt");

        TaskProvider<GroovyfmtTask> check = register(project, "groovyfmtCheck", "check",
                LifecycleBasePlugin.VERIFICATION_GROUP, "Lists files that groovyfmt would reformat and fails if there are any.");
        TaskProvider<GroovyfmtTask> format = register(project, "groovyfmtFormat", "format",
                "formatting", "Formats Groovy, Gradle and Jenkinsfile sources in place with groovyfmt.");
        TaskProvider<GroovyfmtTask> lint = register(project, "groovyfmtLint", "lint",
                LifecycleBasePlugin.VERIFICATION_GROUP, "Reports groovyfmt lint violations and fails if there are any.");
        project.getTasks().withType(GroovyfmtTask.class).configureEach(task -> {
            task.getVersion().set(ext.getVersion());
            task.getBinary().set(ext.getBinary());
            task.getArgs().set(ext.getArgs());
            task.getPaths().set(ext.getPaths().map(paths -> paths.isEmpty()
                    ? List.of(projectDir.getAbsolutePath())
                    : paths.stream().map(p -> projectDir.toPath().resolve(p).toString()).collect(Collectors.toList())));
            task.getWorkingDir().set(projectDir);
            task.getCacheDir().set(cacheDir);
            task.getOffline().set(offline);
            task.getDownloadBaseUrl().set(providers.gradleProperty("groovyfmt.internal.downloadBaseUrl"));
            task.getTokenProperty().set(providers.gradleProperty("groovyfmt.githubToken"));
        });
        check.configure(t -> t.mustRunAfter(format));
        lint.configure(t -> t.mustRunAfter(format));
        project.getPlugins().withType(LifecycleBasePlugin.class, p ->
                project.getTasks().named(LifecycleBasePlugin.CHECK_TASK_NAME).configure(t -> t.dependsOn(check, lint)));
    }

    private static TaskProvider<GroovyfmtTask> register(Project project, String name, String mode, String group, String description) {
        return project.getTasks().register(name, GroovyfmtTask.class, task -> {
            task.setGroup(group);
            task.setDescription(description);
            task.getMode().set(mode);
        });
    }
}
