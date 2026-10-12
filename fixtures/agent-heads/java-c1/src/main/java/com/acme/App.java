package com.acme;

import org.apache.commons.lang3.StringUtils;
import java.nio.file.Files;
import java.nio.file.Path;
import com.google.common.base.Strings;

public class App {
    public void run(Path p) throws Exception {
        StringUtils.isBlank("x");
        StringUtils.abbreviate("x", 3);
        Files.readString(p);
        Strings.isNullOrEmpty("x");
        org.apache.commons.lang3.ObjectUtils.isEmpty("x");
        java.lang.Integer.compare(1, 2);
    }
}
