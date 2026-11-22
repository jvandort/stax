package com.jvandort.stax;

import com.jvandort.stax.internal.LineBuffer;
import org.jspecify.annotations.Nullable;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.Base64;
import java.util.List;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.stream.Collectors;
import java.util.zip.Deflater;

/**
 * Generates a self-contained flamegraph HTML file by embedding profile data into
 * the application bundle template. Input files — stacks (.txt) or JFR
 * recordings (.jfr) — are deflate-compressed and Base64-encoded inline as
 * {@code <template>} elements, allowing the viewer standalone from disk without
 * a server. The viewer routes each embedded entry by its file extension: .jfr
 * entries are decoded natively in the browser, everything else is parsed as
 * stacks.txt files.
 */
public final class FlamegraphGenerator {

    private static final Base64.Encoder ENCODER = Base64.getEncoder();

    /**
     * The line in the application bundle to place embedded stacks before.
     */
    private static final byte[] TARGET_LINE_BYTES = "</body>".getBytes(StandardCharsets.UTF_8);

    /**
     * Chunk size for parallel compression. Small enough that a file of a few tens of megabytes
     * keeps every core busy, and that the chunks in flight (one per core) stay small in memory.
     * With each chunk primed by the one before it (see {@link #deflateChunk}), compression is within
     * about 0.01% of compressing the file as a single stream. Measured on real gradle-profiler
     * stacks files of 240 MB and 774 MB against sizes from 64 KiB to 64 MiB.
     */
    static final int CHUNK_SIZE_BYTES = 4 * 1024 * 1024;

    /**
     * The deflate window: how far back a match can reach, and so how much of the previous chunk
     * primes each chunk's compressor.
     */
    private static final int WINDOW_SIZE_BYTES = 32 * 1024;

    /**
     * Location in jar of the application bundle template to embed stacks files into.
     */
    private static final String FLAMEGRAPH_HTML_TEMPLATE_PATH = "/com/jvandort/stax/index.html";

    private FlamegraphGenerator() {
    }

    /**
     * Command-line entry point. Accepts one or more absolute stacks or JFR file paths
     * followed by the destination path as the final argument.
     *
     * @param args absolute stacks/JFR file paths followed by the output destination path
     * @throws IOException if an I/O error occurs
     */
    public static void main(String... args) throws IOException {
        if (args.length < 2) {
            throw new IllegalArgumentException("Usage: FlamegraphGenerator <input-file>... <destination>");
        }
        List<Path> stacksFiles = Arrays.stream(args, 0, args.length - 1)
            .map(Path::of)
            .collect(Collectors.toList());
        Path destination = Path.of(args[args.length - 1]);
        generate(stacksFiles, destination);
    }

    /**
     * Generates a self-contained flamegraph HTML file at {@code destination} by embedding
     * the given profile files into the application bundle template.
     *
     * @param inputFiles absolute paths to the stacks/JFR files to embed. Must have
     *                   distinct file names, and JFR recordings must be named *.jfr
     * @param destination path of the output HTML file to create
     * @throws IOException if a profile file can't be read or the destination can't be written
     */
    public static void generate(List<Path> inputFiles, Path destination) throws IOException {
        for (Path inputFile : inputFiles) {
            if (!inputFile.isAbsolute()) {
                throw new IllegalArgumentException("Input file '" + inputFile + "' must be absolute");
            }
            if (!Files.isRegularFile(inputFile)) {
                throw new IllegalArgumentException("Input file " + inputFile + " does not exist");
            }
        }

        long distinctNameCount = inputFiles.stream().map(p -> p.getFileName().toString()).distinct().count();
        if (distinctNameCount != inputFiles.size()) {
            throw new IllegalArgumentException("Input files must have distinct names");
        }

        if (destination.getFileName() == null || !destination.getFileName().toString().endsWith(".html")) {
            throw new IllegalArgumentException("Destination file must be an HTML file");
        }

        InputStream template = FlamegraphGenerator.class.getResourceAsStream(FLAMEGRAPH_HTML_TEMPLATE_PATH);
        if (template == null) {
            throw new IllegalStateException("Could not find application bundle template on classpath");
        }

        Files.createDirectories(destination.getParent());

        try (OutputStream output = new BufferedOutputStream(Files.newOutputStream(destination));
             BufferedInputStream input = new BufferedInputStream(template)
        ) {
            boolean foundTargetLine = false;
            LineBuffer lineBuffer = new LineBuffer();

            while (!foundTargetLine) {
                if (!lineBuffer.readFrom(input)) {
                    break;
                }

                if (lineBuffer.trimmedEquals(TARGET_LINE_BYTES)) {
                    foundTargetLine = true;

                    String names = inputFiles.stream()
                        .map(p -> toBase64(p.getFileName().toString()))
                        .collect(Collectors.joining(","));
                    writeUtf8(output, "<template id=\"embedded-file-names\">");
                    writeUtf8(output, names);
                    writeUtf8(output, "</template>\n");

                    for (int i = 0; i < inputFiles.size(); i++) {
                        writeUtf8(output, "<template id=\"embedded-file-" + i + "\">");
                        embedStacksParallel(inputFiles.get(i), output);
                        writeUtf8(output, "</template>\n");
                    }
                }

                lineBuffer.writeTo(output);
            }

            if (!foundTargetLine) {
                throw new IllegalStateException("Could not find </body> in application bundle template");
            }

            input.transferTo(output);
        }
    }

    /**
     * Compresses {@code stacksFile} using raw deflate and writes it Base64-encoded to {@code output}.
     * <p>
     * The file is split into {@value #CHUNK_SIZE_BYTES}-byte chunks, each compressed in parallel using its
     * own {@link Deflater} instance, primed with the last {@value #WINDOW_SIZE_BYTES} bytes of the chunk
     * before it. The compressed chunks concatenate into a single raw deflate stream, as pigz does
     * it. There is no GZIP wrapper or CRC32.
     * <p>
     * The browser decompresses with {@code DecompressionStream("deflate-raw")}.
     */
    private static void embedStacksParallel(Path stacksFile, OutputStream output) throws IOException {
        int threadCount = Runtime.getRuntime().availableProcessors();
        ExecutorService executor = Executors.newFixedThreadPool(threadCount);
        try {
            // NonClosingOutputStream prevents the Base64 encoder's close() from closing `output`,
            // which we still need to write to after this method returns. Closing the Base64 encoder
            // is important as it has finalization work (padding) to perform.
            try (OutputStream base64Out = ENCODER.wrap(new NonClosingOutputStream(output))) {
                ArrayDeque<Future<byte[]>> inFlight = new ArrayDeque<>();

                try (InputStream in = Files.newInputStream(stacksFile)) {
                    // Read one chunk ahead so we know which chunk is the last (needs finish(), not SYNC_FLUSH).
                    byte[] currentChunk = readChunk(in);
                    byte[] dictionary = null;

                    while (true) {
                        byte[] nextChunk = readChunk(in);
                        boolean isLast = nextChunk.length == 0;

                        while (inFlight.size() >= threadCount) {
                            drainNext(inFlight, base64Out);
                        }
                        final byte[] chunk = currentChunk;
                        final byte[] chunkDictionary = dictionary;
                        inFlight.add(executor.submit(() -> deflateChunk(chunk, chunkDictionary, isLast)));

                        if (isLast) {
                            break;
                        }
                        // The next chunk's dictionary is this chunk's last WINDOW_SIZE_BYTES. Only the last chunk
                        // can be shorter than that, and it never gets here.
                        dictionary = Arrays.copyOfRange(chunk, chunk.length - WINDOW_SIZE_BYTES, chunk.length);
                        currentChunk = nextChunk;
                    }
                }

                while (!inFlight.isEmpty()) {
                    drainNext(inFlight, base64Out);
                }
            }
        } finally {
            executor.shutdownNow();
        }
    }

    /**
     * Reads the next chunk of up to {@value #CHUNK_SIZE_BYTES} bytes into its own array, which is
     * only copied when it's the file's last, partial chunk.
     */
    private static byte[] readChunk(InputStream in) throws IOException {
        byte[] chunk = new byte[CHUNK_SIZE_BYTES];
        int read = in.readNBytes(chunk, 0, CHUNK_SIZE_BYTES);
        return read == CHUNK_SIZE_BYTES ? chunk : Arrays.copyOf(chunk, read);
    }

    private static void drainNext(ArrayDeque<Future<byte[]>> inFlight, OutputStream out) throws IOException {
        try {
            out.write(inFlight.poll().get());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException("Interrupted during parallel compression", e);
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
            if (cause instanceof IOException) {
                throw (IOException) cause;
            }
            throw new IOException("Parallel compression failed", cause);
        }
    }

    /**
     * Compresses {@code data} using raw deflate (no GZIP wrapper).
     * <p>
     * {@code dictionary}, the end of the previous chunk, primes the compressor, so matches can
     * reach back into the previous chunk as they would in a single stream. The decompressor has
     * that data in its window already. Without it, every chunk would start cold.
     * <p>
     * Non-final chunks use {@link Deflater#SYNC_FLUSH}: output is byte-aligned and ends with a
     * BFINAL=0 sync block, so the next chunk's deflate output can be appended directly.
     * The final chunk uses {@link Deflater#finish()}: output ends with a BFINAL=1 block.
     *
     * @param dictionary the last {@value #WINDOW_SIZE_BYTES} bytes of the previous chunk, or null for the first
     */
    private static byte[] deflateChunk(byte[] data, byte @Nullable [] dictionary, boolean isFinal) {
        Deflater deflater = new Deflater(Deflater.DEFAULT_COMPRESSION, true); // Raw deflate, with no zlib header.
        if (dictionary != null) {
            deflater.setDictionary(dictionary);
        }
        deflater.setInput(data);
        ByteArrayOutputStream baos = new ByteArrayOutputStream(data.length / 4 + 64);
        byte[] buf = new byte[65536];
        try {
            if (isFinal) {
                deflater.finish();
                while (!deflater.finished()) {
                    int n = deflater.deflate(buf);
                    if (n > 0) baos.write(buf, 0, n);
                }
            } else {
                int n;
                do {
                    n = deflater.deflate(buf, 0, buf.length, Deflater.SYNC_FLUSH);
                    if (n > 0) baos.write(buf, 0, n);
                } while (n == buf.length);
            }
        } finally {
            deflater.end();
        }
        return baos.toByteArray();
    }

    private static void writeUtf8(OutputStream output, String text) throws IOException {
        output.write(text.getBytes(StandardCharsets.UTF_8));
    }

    private static String toBase64(String text) {
        return ENCODER.encodeToString(text.getBytes(StandardCharsets.UTF_8));
    }

    /**
     * An {@link OutputStream} wrapper that delegates to another stream,
     * but suppresses {@link #close()}.
     */
    private static class NonClosingOutputStream extends OutputStream {

        private final OutputStream delegate;

        NonClosingOutputStream(OutputStream delegate) {
            this.delegate = delegate;
        }

        @Override
        public void write(byte[] b) throws IOException {
            delegate.write(b);
        }

        @Override
        public void write(byte[] b, int off, int len) throws IOException {
            delegate.write(b, off, len);
        }

        @Override
        public void flush() throws IOException {
            delegate.flush();
        }

        @Override
        public void write(int b) throws IOException {
            delegate.write(b);
        }

        @Override
        public void close() {
            // Do nothing
        }

    }

}
