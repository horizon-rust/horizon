package dev.horizonlab.parity;

import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.UUID;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.Marker;

/** Only source identity getters: no damage processing, entity constructors or world/server loop. */
public final class VanillaDamageIdentityOracle {
    static Object unsafe;
    static Method allocate;

    static <T extends Entity> T entity(Class<T> type, UUID uuid) throws Exception {
        var entity = type.cast(allocate.invoke(unsafe, type));
        entity.setUUID(uuid);
        return entity;
    }

    static String identity(String name, Entity direct, Entity causing) {
        // The pinned constructor stores this unused holder. Only identity getters are called.
        var source = new DamageSource(null, direct, causing);
        var owner = source.getEntity();
        if (owner == null) return name + " null";
        var uuid = owner.getUUID();
        var hi = uuid.getMostSignificantBits();
        var lo = uuid.getLeastSignificantBits();
        return name + " " + Arrays.toString(new int[] {
            (int) (hi >> 32), (int) hi, (int) (lo >> 32), (int) lo
        }).replace(" ", "");
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var type = Class.forName("sun.misc.Unsafe");
        var field = type.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = type.getMethod("allocateInstance", Class.class);
        var a = new UUID(0x0000000100000002L, 0x0000000300000004L);
        var direct = entity(Marker.class, new UUID(0, 99));
        var rows = new ArrayList<String>();
        rows.add(identity("player", direct, entity(ServerPlayer.class, a)));
        rows.add(identity("npc", direct, entity(Marker.class, a)));
        rows.add(identity("game_npc", direct, entity(Marker.class, new UUID(0, 19))));
        rows.add(identity("ownerless", direct, null));
        Files.write(Path.of(args[0]), rows);
    }
}
