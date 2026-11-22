package com.jvandort.stax;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Random;
import java.util.zip.Deflater;

import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Tests {@link FlamegraphGenerator}.
 */
class FlamegraphGeneratorTest {

    @TempDir
    Path dir;

    @Test
    void compressesAsWellAsASingleStream() throws IOException {
        // An incompressible 24 KiB block, repeated over four 4 MiB compression chunks. Each chunk
        // compresses well only if it's primed with the end of the previous one, so its matches can
        // reach back into it. Compressed cold, every chunk would store the block again.
        byte[] block = new byte[24 * 1024];
        new Random(42).nextBytes(block);
        Path input = dir.resolve("repeated-stacks.txt");
        try (OutputStream out = Files.newOutputStream(input)) {
            for (long written = 0; written < 13L * 1024 * 1024; written += block.length) {
                out.write(block);
            }
        }
        Path html = dir.resolve("flames.html");

        FlamegraphGenerator.generate(List.of(input), html);

        FlamegraphGeneratorTestFixture.assertHasEmbeddedFiles(html, List.of(input));
        long chunked = embeddedCompressedSize(html, 0);
        long singleStream = deflatedSize(Files.readAllBytes(input));
        assertTrue(chunked < singleStream * 1.01,
            "Expected about the size of a single deflate stream (" + singleStream + " bytes), but it's " + chunked + " bytes");
    }

    private static long deflatedSize(byte[] data) {
        Deflater deflater = new Deflater(Deflater.DEFAULT_COMPRESSION, true);
        try {
            deflater.setInput(data);
            deflater.finish();
            byte[] buf = new byte[65536];
            long size = 0;
            while (!deflater.finished()) {
                size += deflater.deflate(buf);
            }
            return size;
        } finally {
            deflater.end();
        }
    }

    /**
     * The size of the compressed data embedded at {@code index}, decoded from its Base64.
     */
    private static long embeddedCompressedSize(Path html, int index) throws IOException {
        String tag = "<template id=\"embedded-file-" + index + "\">";
        for (String line : Files.readAllLines(html, StandardCharsets.UTF_8)) {
            if (line.startsWith(tag)) {
                String base64 = line.substring(tag.length(), line.indexOf('<', tag.length()));
                long padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
                return base64.length() / 4 * 3 - padding;
            }
        }
        throw new AssertionError("No " + tag + " in " + html);
    }

}
