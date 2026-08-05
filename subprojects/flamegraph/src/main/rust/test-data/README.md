# Test data

## cpu-sample.jfr

A minimal JFR recording used by the tests in `src/jfr.rs`: a busy loop calling
a recursive `Busy.fib(int)` through a `Supplier<Long>`, recorded for ~1.5s with
only `jdk.ExecutionSample` enabled at a 1ms interval. Implementing
`Supplier<Long>` makes the hot path go through a compiler-generated bridge
method (`Object get()` delegating to `Long get()`), which the decoder must
skip; the `fib` recursion is genuine and must be preserved. Regenerate with a
JDK 14+:

```java
// Busy.java
import java.util.function.Supplier;

public class Busy implements Supplier<Long> {
    public static void main(String[] args) {
        Supplier<Long> supplier = new Busy();
        long end = System.currentTimeMillis() + 1500;
        long sink = 0;
        while (System.currentTimeMillis() < end) {
            sink += supplier.get();
        }
        System.out.println(sink);
    }

    @Override
    public Long get() {
        return fib(28);
    }

    static long fib(int n) {
        return n < 2 ? n : fib(n - 1) + fib(n - 2);
    }
}
```

```
java "-XX:StartFlightRecording:settings=none,+jdk.ExecutionSample#enabled=true,+jdk.ExecutionSample#period=1ms,duration=2s,filename=cpu-sample.jfr" Busy.java
```
