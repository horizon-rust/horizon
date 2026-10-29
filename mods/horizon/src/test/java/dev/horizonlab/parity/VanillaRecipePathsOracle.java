package dev.horizonlab.parity;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonParser;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.resources.FileToIdConverter;
import net.minecraft.server.packs.PathPackResources;

/** Observes resource enumeration and full recipe IDs without booting a server. */
public final class VanillaRecipePathsOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        var converter = new FileToIdConverter("recipe", ".json");
        var output = new ArrayList<List<String>>();
        JsonArray cases = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray();
        for (var entry : cases) {
            var ids = new ArrayList<String>();
            PathPackResources.listPath("fixture", Path.of(entry.getAsString()), List.of("recipe"),
                (id, supplier) -> {
                    if (id.getPath().endsWith(".json")) {
                        ids.add(converter.fileToId(id).getPath());
                    }
                });
            ids.sort(String::compareTo);
            output.add(ids);
        }
        Files.writeString(Path.of(args[1]), new Gson().toJson(output));
    }
}
