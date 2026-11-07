//! Entity simulation below the client library: mobs, projectiles, player motion and
//! health, and their saved form. Nothing here reaches saves, menus, the network hub
//! or the screens, so a change above does not rebuild it.
#![forbid(unsafe_code)]
extern crate self as horizon_client;
pub mod arrows;
pub mod attribute_updates;
pub mod auto_jump;
pub mod beehive;
pub mod block_contact;
pub mod block_entity_item;
pub mod breathing;
pub mod data_cmd;
pub mod entities;
pub mod entity_nbt;
pub mod entity_rules;
pub mod falling_block;
pub mod first_person;
pub mod gossip;
pub mod health;
pub mod hunger;
pub mod join_seq;
pub mod landing;
pub mod merchant;
pub mod mobs;
pub mod motion;
pub mod native_client_hooks;
pub mod pose;
pub mod powder_snow;
pub mod projectiles;
pub mod spear;
pub mod sprint;
pub mod stand_up;
pub use horizon_client_core::Result;
pub use horizon_client_core::block_entity_nbt;
pub use horizon_client_core::bossbar;
pub use horizon_client_core::component;
pub use horizon_client_core::fastutil_order;
pub use horizon_client_core::gamerules;
pub use horizon_client_core::lang;
pub use horizon_client_core::nbt_path;
pub use horizon_client_core::scoreboard;
pub use horizon_client_core::snbt;
pub use horizon_client_core::text;
pub use horizon_client_input::actions;
pub use horizon_client_input::clipboard;
pub use horizon_client_input::gamepad;
pub use horizon_client_input::held;
pub use horizon_client_input::input;
pub use horizon_client_input::interaction_input;
pub use horizon_client_input::key_names;
pub use horizon_client_input::keybinds;
pub use horizon_client_input::keymapping;
pub use horizon_client_input::mouse;
pub use horizon_client_input::window;
pub use horizon_client_render::{
    assets, atlas_sources, audio, auth, ban_notice, bidi, biome, block_edit, block_entities,
    block_entity_anim, block_entity_render, block_sprites, block_states, bluemap, bluemap_detect,
    border_render, breaking, brewing, camera, chat, chunk, chunk_map, cit, cit_armor, cit_assets,
    cit_glint, client_resources, clouds, collisions, command_block, containers, controller_ui,
    crash, datapack, dynamic_light, effects, environment, extras, fast_hash, fog, font, gfx,
    graphics, graphics_style, graphics_ui, hdr_calibration, hud, hud_layout, hud_overlays,
    inventory_effects, java_text, jigsaw_block, json_folder, level_loading, light, lod, logging,
    menu_blur, mesher, minimap, mipmap, mod_browser, mod_list, model_library, models,
    multiline_edit, native_emissive, options_file, options_fixes, options_ui, overlay,
    pack_browser, pack_screen, packs, pad_cursor, pad_glyph, par, particles, poi_sections,
    post_chain, potent_sulfur, preedit, providers, pvp, pvp_hud, screen_draw, screen_effects,
    sculk, section_mask, server_terrain, settings, shader_options, shader_pack_list, signs, skins,
    sound_engine, sound_events, sound_preview, spans, split_layout, split_map, split_profiles,
    sprites, streaming, structure_block, tab_list, telemetry, test_block, test_instance_block,
    testgen, title_edition, trims, ui, vault, weather, widgets, world, worldborder, zip,
};
pub use horizon_command as command;
pub use horizon_items::blocks;
pub use horizon_items::bundle;
pub use horizon_items::cooldowns;
pub use horizon_items::enchant;
pub use horizon_items::flowers;
pub use horizon_items::item_data;
pub use horizon_items::items;
pub use horizon_items::random_sequences;
pub use horizon_server_files as server;
