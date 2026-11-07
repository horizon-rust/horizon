package dev.horizonlab.parity;

import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Holder;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.server.level.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.damagesource.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.boss.enderdragon.EndCrystal;
import net.minecraft.world.entity.vehicle.minecart.MinecartTNT;
import net.minecraft.world.level.*;
import net.minecraft.world.level.gamerules.*;
import net.minecraft.world.phys.Vec3;

/** Runs the actual crystal hurt and cart ignition/explode methods, without a server. */
public final class VanillaExplosionCausesOracle {
    static Object unsafe;
    static Method allocate;
    static DamageSources sources;
    static CaptureLevel level;
    static DamageSource captured;
    static Entity direct;
    static boolean named;
    static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe, type)); }
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static class CaptureLevel extends ServerLevel {
        CaptureLevel() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public Entity getEntityInAnyDimension(UUID uuid) { return null; }
        @Override public net.minecraft.world.entity.player.Player getPlayerInAnyDimension(UUID uuid) { return null; }
        @Override public GameRules getGameRules() { return new GameRules(net.minecraft.world.flag.FeatureFlagSet.of()); }
        @Override public void broadcastEntityEvent(Entity entity, byte event) {}
        @Override public void explode(Entity source, DamageSource damage, ExplosionDamageCalculator calculator,
                double x, double y, double z, float radius, boolean fire, Level.ExplosionInteraction interaction) {
            direct = source;
            captured = damage == null ? sources.explosion(source, null) : damage;
        }
    }
    static class Crystal extends EndCrystal {
        Crystal() { super(null, null); }
        @Override public Level level() { return level; }
        @Override public DamageSources damageSources() { return sources; }
        @Override public Component getDisplayName() { return Component.literal("Blast"); }
        @Override public void remove(Entity.RemovalReason reason) {}
    }
    static class Cart extends MinecartTNT {
        Cart() { super(null, null); }
        @Override public Level level() { return level; }
        @Override public DamageSources damageSources() { return sources; }
        @Override public Component getDisplayName() { return Component.literal("Blast"); }
        @Override public boolean isSilent() { return true; }
        @Override public void remove(Entity.RemovalReason reason) {}
        @SuppressWarnings({"rawtypes", "unchecked"})
        void data() throws Exception {
            var builder = new net.minecraft.network.syncher.SynchedEntityData.Builder(this);
            String[] keys = {"DATA_SHARED_FLAGS_ID", "DATA_AIR_SUPPLY_ID", "DATA_CUSTOM_NAME_VISIBLE", "DATA_CUSTOM_NAME", "DATA_SILENT", "DATA_NO_GRAVITY", "DATA_POSE", "DATA_TICKS_FROZEN"};
            Object[] values = {(byte)0, 300, false, Optional.empty(), false, false, Pose.STANDING, 0};
            for (int i = 0; i < keys.length; i++) builder.define((net.minecraft.network.syncher.EntityDataAccessor)field(Entity.class, keys[i]).get(null), values[i]);
            defineSynchedData(builder);
            field(Entity.class, "entityData").set(this, builder.build());
        }
        void resetFuse() throws Exception {
            var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, ParityData.access);
            super.addAdditionalSaveData(out);
            var tag = out.buildResult();
            tag.putInt("fuse", -1);
            super.readAdditionalSaveData(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, ParityData.lookup, tag));
        }
        Cart reload() throws Exception {
            var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, ParityData.access);
            super.addAdditionalSaveData(out);
            Cart loaded = entity(Cart.class, getId());
            loaded.readAdditionalSaveData(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, ParityData.lookup, out.buildResult()));
            return loaded;
        }
        void blast() throws Exception { explode((DamageSource)field(MinecartTNT.class, "ignitionSource").get(this), 0); }
    }
    static class Arrow extends net.minecraft.world.entity.projectile.arrow.Arrow {
        Arrow() { super(null, null); }
        @Override public Level level() { return level; }
        @Override public Component getDisplayName() { return Component.literal("Projectile"); }
    }
    static class Human extends ServerPlayer {
        Human() { super(null, null, null, null); }
        @Override public Component getDisplayName() { return Component.literal("Igniter"); }
        @Override public Component getName() { return getDisplayName(); }
        @Override public net.minecraft.world.item.ItemStack getItemBySlot(EquipmentSlot slot) { if (!named || slot != EquipmentSlot.MAINHAND) return net.minecraft.world.item.ItemStack.EMPTY;
            var stack = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.DIAMOND_SWORD);
            stack.set(net.minecraft.core.component.DataComponents.CUSTOM_NAME, Component.literal("Sword"));
            return stack; }
        @Override public ServerLevel level() { return level; }
    }
    static class Mob extends net.minecraft.world.entity.monster.zombie.Husk {
        Mob() { super(null, null); }
        @Override public Component getDisplayName() { return Component.literal("Igniter"); }
        @Override public Component getName() { return getDisplayName(); }
        @Override public net.minecraft.world.item.ItemStack getItemBySlot(EquipmentSlot slot) { if (!named || slot != EquipmentSlot.MAINHAND) return net.minecraft.world.item.ItemStack.EMPTY;
            var stack = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.DIAMOND_SWORD);
            stack.set(net.minecraft.core.component.DataComponents.CUSTOM_NAME, Component.literal("Sword"));
            return stack; }
        @Override public Level level() { return level; }
    }
    static <T extends Entity> T entity(Class<T> type, int id) throws Exception {
        T entity = make(type);
        field(Entity.class, "id").setInt(entity, id);
        field(Entity.class, "uuid").set(entity, new UUID(0L, id));
        field(Entity.class, "position").set(entity, new Vec3(8.5, 64, 8.5));
        field(Entity.class, "random").set(entity, RandomSource.create(42));
        if (entity instanceof Cart cart) cart.data();
        return entity;
    }
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var type = Class.forName("sun.misc.Unsafe");
        unsafe = field(type, "theUnsafe").get(null);
        allocate = type.getMethod("allocateInstance", Class.class);
        sources = new DamageSources(ParityData.access);
        level = make(CaptureLevel.class);
        var output = new ArrayList<String>();
        for (String mode : List.of("crystal", "crystal_snowball", "crystal_arrow", "crystal_named", "cart", "cart_named", "cart_command", "cart_removed", "cart_reloaded")) {
            for (String owner : List.of("player", "guest", "mob", "none")) {
                Entity cause = switch (owner) {
                    case "player", "guest" -> entity(Human.class, 41);
                    case "mob" -> entity(Mob.class, 43);
                    default -> null;
                };
                named = mode.endsWith("named");
                DamageSource hit = sources.generic();
                if (cause != null) hit = sources.mobAttack((LivingEntity)cause);
                if (mode.startsWith("crystal")) {
                    var projectile = entity(Arrow.class, 71);
                    if (mode.equals("crystal_snowball")) hit = sources.thrown(projectile, cause);
                    if (mode.equals("crystal_arrow")) hit = sources.arrow(projectile, cause == null ? projectile : cause);
                    entity(Crystal.class, 7).hurtServer(level, hit, 1);
                } else {
                    Cart cart = entity(Cart.class, 7);
                    field(MinecartTNT.class, "fuse").setInt(cart, -1);
                    field(MinecartTNT.class, "explosionPowerBase").setFloat(cart, 4);
                    cart.primeFuse(hit);
                    // A live NBT update/reprime preserves the first ignition source.
                    cart.resetFuse();
                    cart.primeFuse(sources.mobAttack(entity(Mob.class, 99)));
                    if (mode.equals("cart_removed") && cause != null)
                        field(Entity.class, "removalReason").set(cause, Entity.RemovalReason.KILLED);
                    if (mode.equals("cart_reloaded"))
                        cart = cart.reload();
                    cart.blast();
                }
                Entity actual = captured.getEntity();
                var victim = entity(Human.class, 42);
                if (actual instanceof LivingEntity living) victim.setLastHurtByMob(living);
                if (actual instanceof net.minecraft.world.entity.player.Player player) victim.setLastHurtByPlayer(player, 100);
                LivingEntity credited = victim.getKillCredit();
                var death = captured.getLocalizedDeathMessage(victim);
                String key = ((TranslatableContents)death.getContents()).getKey();
                String damage = captured.typeHolder().unwrapKey().orElseThrow().identifier().toString();
                output.add(mode + "/" + owner + " " + damage + " " + (actual == null ? 0 : actual.getId()) + " " + (credited == null ? 0 : credited.getId()) + " " + key + " " + ((Component)((TranslatableContents)death.getContents()).getArgs()[1]).getString());
                if (direct == null || direct.getId() != 7) throw new AssertionError("direct explosion source");
                if (Explosion.getIndirectSourceEntity(direct) != null) throw new AssertionError("crystal/cart indirect source");
            }
        }
        Files.write(Path.of(args[0]), output);
    }
}
