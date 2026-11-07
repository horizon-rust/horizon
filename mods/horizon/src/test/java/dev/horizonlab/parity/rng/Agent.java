package dev.horizonlab.parity.rng;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;

/**
 * A {@code -javaagent} that replaces Minecraft's unseeded random sources with a deterministic one.
 *
 * <ul>
 *   <li>{@code RandomSupport.generateUniqueSeed()} returns {@link Deterministic#seed()}.</li>
 *   <li>{@code RandomSource.createThreadLocalInstance()} builds its source from that seed instead
 *       of Netty's thread-local random.</li>
 * </ul>
 *
 * Seeded sources ({@code RandomSource.create(long)}) are left alone: their seed is already part of
 * the compared input. Built and used by {@code tools/rng_determinism.py}:
 *
 * <pre>{@code
 * java -javaagent:rng-agent.jar -Dhorizon.rng.seed=42 ... Oracle input.txt output.txt
 * }</pre>
 *
 * The transformer targets class names, not compile-time Minecraft types, so the agent builds
 * against the JDK alone and works with both the client and server jars.
 */
public final class Agent {
    private static final ClassDesc DETERMINISTIC = ClassDesc.of("dev.horizonlab.parity.rng.Deterministic");
    private static final ClassDesc RANDOM_SOURCE = ClassDesc.of("net.minecraft.util.RandomSource");
    private static final ClassDesc SINGLE_THREADED =
            ClassDesc.of("net.minecraft.world.level.levelgen.SingleThreadedRandomSource");
    private static final MethodTypeDesc LONG_TO_VOID =
            MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_long);
    private static final MethodTypeDesc VOID_TO_LONG = MethodTypeDesc.of(ConstantDescs.CD_long);

    private Agent() {}

    public static void premain(String arguments, Instrumentation instrumentation) {
        Deterministic.configure(arguments);
        instrumentation.addTransformer(new Transformer(), false);
    }

    static final class Transformer implements ClassFileTransformer {
        @Override
        public byte[] transform(ClassLoader loader, String name, Class<?> loaded,
                                ProtectionDomain domain, byte[] bytes) {
            try {
                return switch (name) {
                    case "net/minecraft/world/level/levelgen/RandomSupport" ->
                            ClassFile.of().transformClass(ClassFile.of().parse(bytes),
                                    ClassTransform.transformingMethodBodies(
                                            method -> method.methodName().equalsString("generateUniqueSeed"),
                                            replace(body -> {
                                                body.invokestatic(DETERMINISTIC, "seed", VOID_TO_LONG);
                                                body.lreturn();
                                            })));
                    case "net/minecraft/util/RandomSource" ->
                            ClassFile.of().transformClass(ClassFile.of().parse(bytes),
                                    ClassTransform.transformingMethodBodies(
                                            method -> method.methodName().equalsString("createThreadLocalInstance")
                                                    && method.methodTypeSymbol().equals(
                                                            MethodTypeDesc.of(RANDOM_SOURCE)),
                                            replace(body -> {
                                                body.new_(SINGLE_THREADED);
                                                body.dup();
                                                body.invokestatic(DETERMINISTIC, "seed", VOID_TO_LONG);
                                                body.invokespecial(SINGLE_THREADED, "<init>", LONG_TO_VOID);
                                                body.areturn();
                                            })));
                    default -> null;
                };
            } catch (Throwable error) {
                System.err.println("horizon.rng: cannot patch " + name + ": " + error);
                return null;
            }
        }
    }

    /** A body transform that drops every original element and emits the replacement once. */
    private static CodeTransform replace(java.util.function.Consumer<CodeBuilder> replacement) {
        return new CodeTransform() {
            private boolean emitted;

            @Override
            public void accept(CodeBuilder builder, CodeElement element) {
                if (!emitted) {
                    emitted = true;
                    replacement.accept(builder);
                }
            }
        };
    }
}
