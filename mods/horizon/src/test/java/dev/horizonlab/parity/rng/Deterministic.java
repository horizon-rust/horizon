package dev.horizonlab.parity.rng;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.Writer;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The seed every patched {@code RandomSupport.generateUniqueSeed()} call returns.
 *
 * <p>Vanilla's formula is {@code uniquifier.updateAndGet(x -> x * STEP) ^ System.nanoTime()}.
 * With a master seed the clock term is the seed instead, so a vanilla oracle run becomes
 * repeatable. This does not make vanilla match Horizon; the two need not create random sources in
 * the same order. The clock term is the only nondeterministic part; seeded sources
 * ({@code RandomSource.create(long)}) are already repeatable.
 *
 * <p>The agent is opt-in ({@code tools/rng_determinism.py}); without it nothing here runs.
 */
public final class Deterministic {
    /** {@code RandomSupport.SEED_UNIQUIFIER}'s start and the latter's multiplier. */
    private static final long UNIQUIFIER_START = 8_682_522_807_148_012L;
    private static final long STEP = 1_181_783_497_276_652_981L;

    private static final AtomicLong UNIQUIFIER = new AtomicLong(UNIQUIFIER_START);
    private static final AtomicLong INDEX = new AtomicLong();

    private static long master;
    private static Writer trace;

    private Deterministic() {}

    /**
     * Called from {@link Agent#premain}. Reads {@code -Dhorizon.rng.seed}/{@code HORIZON_RNG_SEED},
     * falling back to seed 0 under {@code HORIZON_STILL=1} as Horizon does, and
     * {@code -Dhorizon.rng.trace}/{@code HORIZON_RNG_TRACE}.
     */
    public static synchronized void configure(String arguments) {
        String seed = System.getProperty("horizon.rng.seed", System.getenv("HORIZON_RNG_SEED"));
        if (seed == null && "1".equals(System.getenv("HORIZON_STILL"))) {
            seed = "0";
        }
        master = parse(seed, 0L);
        String file = System.getProperty("horizon.rng.trace", System.getenv("HORIZON_RNG_TRACE"));
        if (file != null && !file.isBlank()) {
            try {
                Path path = Path.of(file);
                if (path.getParent() != null) {
                    Files.createDirectories(path.getParent());
                }
                trace = new BufferedWriter(Files.newBufferedWriter(path, StandardCharsets.UTF_8));
            } catch (IOException error) {
                System.err.println("horizon.rng: cannot open trace " + file + ": " + error);
            }
        }
    }

    /** One {@code RandomSupport.generateUniqueSeed()}: the uniquifier, XORed with the master seed. */
    public static long seed() {
        long next = UNIQUIFIER.updateAndGet(value -> value * STEP);
        long value = next ^ master;
        if (trace != null) {
            record(INDEX.getAndIncrement(), value);
        }
        return value;
    }

    /**
     * A seed named by {@code name}, for call sites whose creation order must not matter
     * (particles and other effects). Matches Horizon's
     * {@code horizon_core::random::pinned_seed}: MD5 of the name's UTF-8 (the first 64 bits,
     * vanilla's {@code RandomSupport.seedFromHashOf}), XORed with the master seed.
     */
    public static long streamSeed(String name) {
        try {
            Class<?> support = Class.forName("net.minecraft.world.level.levelgen.RandomSupport");
            Object seed = support.getMethod("seedFromHashOf", String.class).invoke(null, name);
            Method low = seed.getClass().getMethod("seedLo");
            return (long) low.invoke(seed) ^ master;
        } catch (ReflectiveOperationException error) {
            throw new IllegalStateException("RandomSupport.seedFromHashOf is unavailable", error);
        }
    }

    /** The master seed (0 when the tool was not asked for one). */
    public static long master() {
        return master;
    }

    private static synchronized void record(long index, long value) {
        try {
            trace.write("create " + index + " " + value + " " + caller() + "\n");
            trace.flush();
        } catch (IOException error) {
            trace = null;
        }
    }

    /** The first frame outside the RNG plumbing, as {@code Class.method}. */
    private static String caller() {
        return StackWalker.getInstance().walk(frames -> frames
                .filter(frame -> {
                    String name = frame.getClassName();
                    return !name.startsWith("java.")
                            && !name.startsWith("jdk.")
                            && !name.startsWith("dev.horizonlab.parity.rng.")
                            && !name.equals("net.minecraft.world.level.levelgen.RandomSupport")
                            && !name.equals("net.minecraft.util.RandomSource");
                })
                .findFirst()
                .map(frame -> frame.getClassName() + "." + frame.getMethodName())
                .orElse("?"));
    }

    /** Decimal, {@code 0x} hex, or an optional leading minus. */
    static long parse(String text, long fallback) {
        if (text == null || text.isBlank()) {
            return fallback;
        }
        String value = text.trim().replace("_", "");
        boolean negative = value.startsWith("-");
        if (negative) {
            value = value.substring(1);
        }
        try {
            long parsed = value.startsWith("0x") || value.startsWith("0X")
                    ? Long.parseUnsignedLong(value.substring(2), 16)
                    : Long.parseLong(value);
            return negative ? -parsed : parsed;
        } catch (NumberFormatException error) {
            System.err.println("horizon.rng: ignoring seed " + text);
            return fallback;
        }
    }
}
