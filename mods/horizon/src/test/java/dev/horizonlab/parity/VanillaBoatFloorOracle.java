package dev.horizonlab.parity;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.math.Axis;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.object.boat.BoatModel;

/** ModelPart.render on the boat floor, with the real renderer's root transform. */
public final class VanillaBoatFloorOracle {
    private static final class Capture implements VertexConsumer {
        final JsonArray vertices = new JsonArray();
        @Override public VertexConsumer addVertex(float x, float y, float z) {
            var point = new JsonArray();
            point.add(x); point.add(y); point.add(z);
            vertices.add(point);
            return this;
        }
        @Override public VertexConsumer setNormal(float x, float y, float z) { return this; }
        @Override public VertexConsumer setColor(int r, int g, int b, int a) { return this; }
        @Override public VertexConsumer setColor(int color) { return this; }
        @Override public VertexConsumer setUv(float u, float v) { return this; }
        @Override public VertexConsumer setUv1(int u, int v) { return this; }
        @Override public VertexConsumer setUv2(int u, int v) { return this; }
        @Override public VertexConsumer setUv3(float u, float v) { return this; }
        @Override public VertexConsumer setLineWidth(float width) { return this; }
    }

    public static void main(String[] args) throws Exception {
        var results = new JsonArray();
        ModelPart root = BoatModel.createBoatModel().bakeRoot();
        ModelPart bottom = root.getChild("bottom");
        for (float yaw : new float[]{0, 45, 90, 135, 180, 270}) {
            for (float cameraY : new float[]{0, 0.25f, 0.5f, 1.2f, 1.4f, 2.1875f}) {
                var stack = new PoseStack();
                stack.translate(0, -cameraY, 0);
                stack.translate(0, 0.375f, 0);
                stack.rotateDegrees(Axis.YP, 180 - yaw);
                stack.scale(-1, -1, 1);
                stack.rotateDegrees(Axis.YP, 90);
                root.translateAndRotate(stack);
                var row = new JsonObject();
                row.addProperty("yaw", yaw);
                row.addProperty("camera_y", cameraY);
                var matrix = new JsonArray();
                for (float value : stack.last().pose().get(new float[16])) matrix.add(value);
                row.add("base", matrix);
                var capture = new Capture();
                bottom.render(stack, capture, 0xF000F0, 0);
                row.add("vertices", capture.vertices);
                results.add(row);
            }
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(results));
    }
}
