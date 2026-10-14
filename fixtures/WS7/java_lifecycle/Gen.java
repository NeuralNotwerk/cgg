package demo;

import org.gradle.api.DefaultTask;
import org.gradle.api.tasks.TaskAction;
import org.gradle.api.tasks.Input;

public abstract class Gen extends DefaultTask {
    @TaskAction
    void generate() {}

    @Input
    private String getMode() { return "m"; }

    // near miss
    private void generateLater() {}
}
