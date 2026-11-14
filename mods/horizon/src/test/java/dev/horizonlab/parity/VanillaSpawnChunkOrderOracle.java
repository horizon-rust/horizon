package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import it.unimi.dsi.fastutil.longs.LongIterator;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.StringJoiner;
import net.minecraft.core.SectionPos;
import net.minecraft.server.level.ChunkHolder;
import net.minecraft.server.level.DistanceManager;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.util.RandomSource;
import net.minecraft.util.Util;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.TicketStorage;

/**
 * Original oracle: the order natural spawning walks its chunks in, from a real
 * {@code DistanceManager}'s {@code naturalSpawnChunkCounter}.
 * Each input line is one level: {@code sim} (its simulation distance) and ops, run in order:
 * {@code ["join", id, x, y, z]} ({@code ChunkMap.updatePlayerStatus(player, true)}: {@code addPlayer}
 * at the player's section), {@code ["move", id, x, y, z]} ({@code ChunkMap.move}: when the section
 * changed, {@code removePlayer} at the old one then {@code addPlayer} at the new one),
 * {@code ["leave", id]} ({@code updatePlayerStatus(player, false)}: {@code removePlayer}) and
 * {@code ["pass", seed]}: a spawning pass, which prints {@code getNaturalSpawnChunkCount}, the chunks
 * of {@code getSpawnCandidateChunks} in its order (both flush the counter's queue), then those chunks
 * after {@code Util.shuffle} with a {@code RandomSource.create(seed)} and that random's next
 * {@code nextInt()}.
 * The players are allocated ServerPlayers, each with its own entity id ({@code Entity.equals} and
 * {@code hashCode} compare ids); the DistanceManager only puts them in its per-chunk sets.
 */
public final class VanillaSpawnChunkOrderOracle {
    private static String chunks(List<Long> chunks) {
        StringJoiner out = new StringJoiner(" ");
        for (long pos : chunks) {
            out.add(ChunkPos.getX(pos) + ":" + ChunkPos.getZ(pos));
        }
        return out.toString();
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Field theUnsafe = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        Object unsafe = theUnsafe.get(null);
        Method allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        Field entityId = Entity.class.getDeclaredField("id");
        entityId.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            int index = 0;
            for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
                JsonObject input = JsonParser.parseString(line).getAsJsonObject();
                DistanceManager manager = new DistanceManager(new TicketStorage(), Runnable::run, Runnable::run) {
                    @Override
                    protected boolean isChunkToRemove(final long node) {
                        return false;
                    }

                    @Override
                    protected ChunkHolder getChunk(final long node) {
                        return null;
                    }

                    @Override
                    protected ChunkHolder updateChunkScheduling(final long node, final int level, final ChunkHolder chunk, final int oldLevel) {
                        return chunk;
                    }
                };
                manager.updateSimulationDistance(input.get("sim").getAsInt());
                Map<Integer, ServerPlayer> players = new HashMap<>();
                Map<Integer, SectionPos> sections = new HashMap<>();
                out.println("== " + index++);
                for (JsonElement element : input.getAsJsonArray("ops")) {
                    JsonArray op = element.getAsJsonArray();
                    switch (op.get(0).getAsString()) {
                        case "join", "move" -> {
                            int id = op.get(1).getAsInt();
                            SectionPos section = SectionPos.of(op.get(2).getAsInt(), op.get(3).getAsInt(), op.get(4).getAsInt());
                            ServerPlayer player = players.get(id);
                            if (player == null) {
                                player = (ServerPlayer) allocate.invoke(unsafe, ServerPlayer.class);
                                entityId.setInt(player, id);
                                players.put(id, player);
                                manager.addPlayer(section, player);
                            } else if (sections.get(id).asLong() != section.asLong()) {
                                manager.removePlayer(sections.get(id), player);
                                manager.addPlayer(section, player);
                            }
                            sections.put(id, section);
                        }
                        case "leave" -> {
                            int id = op.get(1).getAsInt();
                            manager.removePlayer(sections.remove(id), players.remove(id));
                        }
                        case "pass" -> {
                            int count = manager.getNaturalSpawnChunkCount();
                            List<Long> order = new ArrayList<>();
                            for (LongIterator it = manager.getSpawnCandidateChunks(); it.hasNext(); ) {
                                order.add(it.nextLong());
                            }
                            out.println(count + " | " + chunks(order));
                            RandomSource random = RandomSource.create(op.get(1).getAsLong());
                            Util.shuffle(order, random);
                            out.println(random.nextInt() + " | " + chunks(order));
                        }
                        default -> throw new IllegalArgumentException(op.toString());
                    }
                }
            }
        }
    }
}
