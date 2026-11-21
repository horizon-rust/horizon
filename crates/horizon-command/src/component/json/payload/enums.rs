use super::*;

pub(super) fn validate(name: &str, value: &Value) -> Option<R> {
    let names: &[&str] = match name {
        "minecraft:dye"
        | "minecraft:base_color"
        | "minecraft:wolf/collar"
        | "minecraft:cat/collar"
        | "minecraft:sheep/color"
        | "minecraft:shulker/color"
        | "minecraft:cushion/color"
        | "minecraft:tropical_fish/base_color"
        | "minecraft:tropical_fish/pattern_color" => &[
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ],
        "minecraft:fox/variant" => &["red", "snow"],
        "minecraft:salmon/size" => &["small", "medium", "large"],
        "minecraft:parrot/variant" => &["red_blue", "blue", "green", "yellow_blue", "gray"],
        "minecraft:tropical_fish/pattern" => &[
            "kob",
            "sunstreak",
            "snooper",
            "dasher",
            "brinely",
            "spotty",
            "flopper",
            "stripey",
            "glitter",
            "blockfish",
            "betty",
            "clayfish",
        ],
        "minecraft:mooshroom/variant" => &["red", "brown"],
        "minecraft:rabbit/variant" => &[
            "brown",
            "white",
            "black",
            "white_splotched",
            "gold",
            "salt",
            "evil",
        ],
        "minecraft:horse/variant" => &[
            "white",
            "creamy",
            "chestnut",
            "brown",
            "black",
            "gray",
            "dark_brown",
        ],
        "minecraft:llama/variant" => &["creamy", "white", "brown", "gray"],
        "minecraft:axolotl/variant" => &["lucy", "wild", "gold", "cyan", "blue"],
        _ => return None,
    };
    Some(one_of(value, names))
}
