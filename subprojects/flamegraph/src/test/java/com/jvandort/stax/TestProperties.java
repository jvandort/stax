package com.jvandort.stax;

import java.nio.file.Path;

/**
 * System properties the test task sets.
 */
final class TestProperties {

    private TestProperties() {
    }

    /**
     * The test-data directory, with recordings shared with the Rust tests.
     */
    static Path testData() {
        return requiredPath("stax.test-data.path");
    }

    private static Path requiredPath(String name) {
        String value = System.getProperty(name);
        if (value == null) {
            throw new IllegalStateException("The " + name + " system property is required");
        }
        return Path.of(value);
    }

}
