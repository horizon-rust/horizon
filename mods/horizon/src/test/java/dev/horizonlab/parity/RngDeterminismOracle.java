package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.RandomSupport;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;

/**
 * The Java side of tools/test_rng_determinism_parity.py.
 *
 * <p>Run with the determinism agent ({@code -javaagent:rng-agent.jar}) and a seed; the first
 * {@code count} values must equal Horizon's {@code horizon_core::random::unique_seed()}, and the
 * draws from those seeds must equal {@code LegacyRandom}/{@code XoroshiroRandom}. Also exercises
 * the two patched entry points (`RandomSource.create()`, `createThreadLocalInstance()`), so a
 * missing patch shows up as a growing trace or a mismatch.
 *
 * <p>Arguments: an input file holding the count, then the output file.
 */
public final class RngDeterminismOracle {
    private RngDeterminismOracle() {}

    public static void main(String[] args) throws Exception {
        int count = Integer.parseInt(Files.readString(Path.of(args[0])).trim());
        try (PrintWriter out = new PrintWriter(
                Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (int i = 0; i < count; i++) {
                long seed = RandomSupport.generateUniqueSeed();
                out.println("seed " + i + " " + seed);
                out.println("legacy " + i + " " + new LegacyRandomSource(seed).nextLong());
                out.println("xoroshiro " + i + " " + new XoroshiroRandomSource(seed).nextLong());
            }
            for (int i = 0; i < count; i++) {
                out.println("create " + i + " " + RandomSource.create().nextLong());
            }
            out.println("threadlocal " + RandomSource.createThreadLocalInstance().nextLong());
            for (String name : new String[] {"title_splash", "structure_block", "firework_spark"}) {
                out.println("named " + name + " " + named(name));
            }
        }
    }

    /** `Deterministic.streamSeed(name)`, reached reflectively so this oracle compiles without the agent. */
    private static long named(String name) throws ReflectiveOperationException {
        Class<?> deterministic = Class.forName("dev.horizonlab.parity.rng.Deterministic");
        return (long) deterministic.getMethod("streamSeed", String.class).invoke(null, name);
    }
}
