package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.TagParser;
import net.minecraft.network.chat.ClickEvent;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.HoverEvent;
import java.util.ArrayList;
import net.minecraft.network.chat.ComponentSerialization;

/** NBT component acceptance and decoded page events from the pinned client codec. */
public final class VanillaNativeTextNbtOracle {
    private static void pages(Component text, ArrayList<Integer> pages) {
        if (text.getStyle().getClickEvent() instanceof ClickEvent.ChangePage page) pages.add(page.page());
        if (text.getStyle().getHoverEvent() instanceof HoverEvent.ShowText hover) pages(hover.value(), pages);
        for (var child : text.getSiblings()) pages(child, pages);
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        try (var out = Files.newBufferedWriter(Path.of(args[1]))) {
            for (var input : Files.readAllLines(Path.of(args[0]))) {
                var result = ComponentSerialization.CODEC.parse(NbtOps.INSTANCE, TagParser.parseCompoundFully(input)).result();
                if (result.isEmpty()) {
                    out.write("reject\n");
                } else {
                    var pages = new ArrayList<Integer>();
                    pages(result.get(), pages);
                    out.write("ok " + pages.toString().replace(" ", "") + "\n");
                }
            }
        }
    }
}
