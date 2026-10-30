package dev.horizonlab.parity;

import com.google.common.collect.ImmutableList;
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.gui.components.AbstractSelectionList;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.components.tabs.MenuTabBar;
import net.minecraft.client.gui.components.tabs.Tab;
import net.minecraft.client.gui.screens.achievement.StatsScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;

/** Calls the real StatsScreen.setTabActiveStateAndTooltip without a window or client. */
public final class VanillaStatsTabsOracle {
    static Object unsafe;
    static Method allocate;

    static Object alloc(Class<?> type) throws Exception {
        return allocate.invoke(unsafe, type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }

    /** Only captures the two setters; the statistics method and list emptiness check are vanilla. */
    public static final class CaptureBar extends MenuTabBar {
        List<Tab> tabs;
        boolean active;
        Tooltip tooltip;
        private CaptureBar() { super(0, 0, 0, 0, null, ImmutableList.of(), ImmutableList.of()); }
        @Override public List<Tab> getTabs() { return tabs; }
        @Override public void setTabActiveState(int index, boolean value) { active = value; }
        @Override public void setTabTooltip(int index, Tooltip value) { tooltip = value; }
    }

    public static void main(String[] args) throws Exception {
        Class<?> u = Class.forName("sun.misc.Unsafe");
        Field singleton = u.getDeclaredField("theUnsafe");
        singleton.setAccessible(true);
        unsafe = singleton.get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        Class<?> tabClass = Class.forName(StatsScreen.class.getName() + "$StatisticsTab");
        Class<?> listClass = Class.forName(StatsScreen.class.getName() + "$ItemStatisticsList");
        Method update = StatsScreen.class.getDeclaredMethod("setTabActiveStateAndTooltip", int.class);
        update.setAccessible(true);
        Field message = Tooltip.class.getDeclaredField("message");
        message.setAccessible(true);
        Object screen = alloc(StatsScreen.class);
        CaptureBar bar = (CaptureBar) alloc(CaptureBar.class);
        put(screen, StatsScreen.class, "tabNavigationBar", bar);
        Object list = alloc(listClass);
        Tab tab = (Tab) alloc(tabClass);
        put(tab, tabClass, "list", list);
        bar.tabs = List.of(tab, tab, tab);
        var results = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            JsonObject c = JsonParser.parseString(line).getAsJsonObject();
            put(list, AbstractSelectionList.class, "children", Collections.nCopies(c.get("entries").getAsInt(), null));
            update.invoke(screen, c.get("index").getAsInt());
            JsonObject out = new JsonObject();
            out.addProperty("active", bar.active);
            if (bar.tooltip == null) out.add("tooltip", JsonNull.INSTANCE);
            else {
                Component text = (Component) message.get(bar.tooltip);
                out.addProperty("tooltip", ((TranslatableContents) text.getContents()).getKey());
            }
            results.add(new GsonBuilder().serializeNulls().create().toJson(out));
        }
        Files.write(Path.of(args[1]), results);
    }
}
