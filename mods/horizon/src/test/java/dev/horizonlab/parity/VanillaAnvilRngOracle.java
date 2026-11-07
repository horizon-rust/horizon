package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.util.RandomSource;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AnvilMenu;
import net.minecraft.world.inventory.ContainerLevelAccess;
import net.minecraft.world.inventory.DataSlot;
import net.minecraft.world.inventory.ItemCombinerMenu;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

/** Runs native AnvilMenu.onTake with independent owner, guest and level RNGs. */
public final class VanillaAnvilRngOracle {
    static Object unsafe;
    static <T> T allocate(Class<T> type) throws Exception {
        return type.cast(unsafe.getClass().getMethod("allocateInstance", Class.class).invoke(unsafe, type));
    }
    static void field(Class<?> owner, Object target, String name, Object value) throws Exception {
        var field = owner.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }
    static final class Actor extends ServerPlayer {
        RandomSource rng;
        boolean creative;
        Actor() { super(null, null, null, null); }
        @Override public RandomSource getRandom() { return rng; }
        @Override public boolean hasInfiniteMaterials() { return creative; }
        @Override public void giveExperienceLevels(int levels) { }
    }
    static final class World extends ServerLevel {
        BlockState state;
        RandomSource rng;
        World() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public BlockState getBlockState(BlockPos pos) { return state; }
        @Override public RandomSource getRandom() { return rng; }
        @Override public boolean setBlock(BlockPos pos, BlockState state, int flags, int limit) { this.state = state; return true; }
        @Override public boolean removeBlock(BlockPos pos, boolean moved) { state = Blocks.AIR.defaultBlockState(); return true; }
        @Override public void levelEvent(Entity source, int id, BlockPos pos, int data) { }
    }
    static final class Menu extends AnvilMenu {
        Menu() { super(0, null); }
        void take(Player player) { onTake(player, ItemStack.EMPTY); }
    }
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var singleton = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        singleton.setAccessible(true);
        unsafe = singleton.get(null);
        var lines = new ArrayList<String>();
        for (boolean guestTurn : new boolean[] {false, true}) {
            for (boolean creative : new boolean[] {false, true}) {
                for (String name : new String[] {"minecraft:anvil", "minecraft:chipped_anvil", "minecraft:damaged_anvil", "minecraft:stone"}) {
                    for (long seed : new long[] {0, 4096, 99999}) {
                        Actor owner = allocate(Actor.class), guest = allocate(Actor.class);
                        owner.rng = RandomSource.create(seed);
                        guest.rng = RandomSource.create(seed + 7);
                        owner.creative = guest.creative = creative;
                        World world = allocate(World.class);
                        world.rng = RandomSource.create(98765);
                        world.state = BuiltInRegistries.BLOCK.getValue(Identifier.parse(name)).defaultBlockState();
                        Menu menu = allocate(Menu.class);
                        field(ItemCombinerMenu.class, menu, "inputSlots", new SimpleContainer(2));
                        field(ItemCombinerMenu.class, menu, "access", ContainerLevelAccess.create(world, new BlockPos(3, 64, 3)));
                        field(AnvilMenu.class, menu, "cost", DataSlot.standalone());
                        menu.take(guestTurn ? guest : owner);
                        String result = BuiltInRegistries.BLOCK.getKey(world.state.getBlock()).toString();
                        lines.add((guestTurn ? 1 : 0) + " " + (creative ? 1 : 0) + " " + name + " " + seed + " " + result + " " + owner.rng.nextInt() + " " + guest.rng.nextInt() + " " + world.rng.nextInt());
                    }
                }
            }
        }
        Files.writeString(Path.of(args[0]), String.join("\n", lines) + "\n");
    }
}
