package com.jvandort.stax;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.BufferedWriter;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

/**
 * Verifies that {@link FlamegraphGeneratorTestFixture} accepts what {@link FlamegraphGenerator}
 * really embeds, and rejects anything else, for the reasons it gives.
 */
class FlamegraphGeneratorTestFixtureTest {

    @TempDir
    Path dir;

    private Path raw;
    private Path simplified;
    private Path html;

    @BeforeEach
    void generate() throws IOException {
        raw = write("build-cpu-raw-stacks.txt", "main;compile;parse 10\nmain;compile 5\nmain;test 3\n");
        simplified = write("build-cpu-simplified-stacks.txt", "main;compile 15\nmain;test 3\n");
        html = dir.resolve("flames.html");
        FlamegraphGenerator.generate(List.of(raw, simplified), html);
    }

    @Test
    void acceptsExactlyTheEmbeddedFiles() {
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw, simplified));
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw, simplified), false);
    }

    @Test
    void acceptsTheFilesInAnyOrder() {
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(simplified, raw));
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(simplified, raw), false);
    }

    @Test
    void strictRejectsAnEmbeddedFileThatIsNotExpected() {
        AssertionError e = assertThrows(AssertionError.class,
            () -> FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw)));
        assertEquals("Found unexpected files [build-cpu-simplified-stacks.txt] embedded. Expected only [build-cpu-raw-stacks.txt]", e.getMessage());
    }

    @Test
    void nonStrictAcceptsExtraEmbeddedFiles() {
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw), false);
    }

    @Test
    void rejectsAnExpectedFileThatIsNotEmbedded() throws IOException {
        Path missing = write("other-stacks.txt", "a;b 1\n");
        for (boolean strict : List.of(true, false)) {
            AssertionError e = assertThrows(AssertionError.class,
                () -> FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw, simplified, missing), strict));
            assertEquals("Expected files [other-stacks.txt] to be embedded, but found [build-cpu-raw-stacks.txt, build-cpu-simplified-stacks.txt]", e.getMessage());
        }
    }

    @Test
    void rejectsAFileWhoseContentChanged() throws IOException {
        Files.writeString(raw, "main;compile;parse 11\nmain;compile 5\nmain;test 3\n");
        for (boolean strict : List.of(true, false)) {
            AssertionError e = assertThrows(AssertionError.class,
                () -> FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw, simplified), strict));
            assertEquals("Embedded file build-cpu-raw-stacks.txt does not match expected content", e.getMessage());
        }
    }

    @Test
    void rejectsAFileWithExtraContent() throws IOException {
        Files.writeString(simplified, Files.readString(simplified) + "main;extra 1\n");
        AssertionError e = assertThrows(AssertionError.class,
            () -> FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(raw, simplified)));
        assertEquals("Embedded file build-cpu-simplified-stacks.txt does not match expected content", e.getMessage());
    }

    @Test
    void rejectsHtmlWithNoEmbeddedFiles() throws IOException {
        Path plain = write("plain.html", "<html>\n<body>\n</body>\n</html>\n");
        AssertionError e = assertThrows(AssertionError.class,
            () -> FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(plain, List.of(raw)));
        assertEquals("Expected files [build-cpu-raw-stacks.txt] to be embedded, but found []", e.getMessage());
    }

    @Test
    void verifiesABinaryJfrRecording() throws IOException {
        Path jfr = TestProperties.testData().resolve("cpu-sample.jfr");
        Path jfrHtml = dir.resolve("jfr.html");
        FlamegraphGenerator.generate(List.of(jfr, raw), jfrHtml);
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(jfrHtml, List.of(jfr, raw));
    }

    @Test
    void verifiesAnEmptyFile() throws IOException {
        Path empty = write("empty-stacks.txt", "");
        Path emptyHtml = dir.resolve("empty.html");
        FlamegraphGenerator.generate(List.of(empty), emptyHtml);
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(emptyHtml, List.of(empty));
    }

    @Test
    void verifiesNonAsciiFileNames() throws IOException {
        Path named = write("café, ünïcode-stacks.txt", "a;b 1\n");
        Path namedHtml = dir.resolve("named.html");
        FlamegraphGenerator.generate(List.of(named, raw), namedHtml);
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(namedHtml, List.of(named, raw));
    }

    @Test
    void verifiesAFileSpanningSeveralCompressionChunks() throws IOException {
        // Two full chunks, so the second is primed with the first, and a partial last one.
        long size = 2L * FlamegraphGenerator.CHUNK_SIZE_BYTES + FlamegraphGenerator.CHUNK_SIZE_BYTES / 2;
        Path big = dir.resolve("big-stacks.txt");
        try (BufferedWriter out = Files.newBufferedWriter(big, StandardCharsets.UTF_8)) {
            for (int i = 0; Files.size(big) < size; i++) {
                for (int j = 0; j < 10_000; j++) {
                    out.write("main;worker-" + (i % 97) + ";task-" + j + ";leaf " + (i * 31 + j) + "\n");
                }
                out.flush();
            }
        }
        Path bigHtml = dir.resolve("big.html");
        FlamegraphGenerator.generate(List.of(big), bigHtml);
        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(bigHtml, List.of(big));

        // A change in the last chunk is still caught.
        try (BufferedWriter out = Files.newBufferedWriter(big, StandardCharsets.UTF_8, StandardOpenOption.APPEND)) {
            out.write("main;late 1\n");
        }
        AssertionError e = assertThrows(AssertionError.class,
            () -> FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(bigHtml, List.of(big)));
        assertEquals("Embedded file big-stacks.txt does not match expected content", e.getMessage());
    }

    private Path write(String name, String content) throws IOException {
        return Files.writeString(dir.resolve(name), content, StandardCharsets.UTF_8);
    }

}
