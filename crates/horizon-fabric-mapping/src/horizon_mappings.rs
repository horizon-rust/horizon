//! Horizon internal name registry for mapping.

use std::collections::HashMap;
use crate::mappings::{HorizonName, HorizonNameKind};
use crate::MappingError;

/// Horizon internal mappings - maps Yarn names to Horizon Rust paths.
pub struct HorizonMappings {
    /// Yarn class/field/method -> Horizon name
    mappings: HashMap<String, HorizonName>,
    /// Reverse lookup: Horizon rust_path::rust_name -> Yarn
    reverse: HashMap<String, String>,
}

impl HorizonMappings {
    /// Load the built-in Horizon mappings.
    pub fn load() -> Result<Self, MappingError> {
        let mut mappings = HashMap::new();
        let mut reverse = HashMap::new();
        
        // Core classes
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.world.World", "horizon_client::world::World", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.client.MinecraftClient", "horizon_client::game::Game", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.client.world.ClientWorld", "horizon_client::world::World", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.server.world.ServerWorld", "horizon_client::world::World", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.entity.player.PlayerEntity", "horizon_client::motion::PlayerMotion", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.entity.LivingEntity", "horizon_client::mobs::Mob", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.block.BlockState", "horizon_client::blocks::BlockState", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.item.ItemStack", "horizon_client::items::ItemStack", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.util.math.BlockPos", "horizon_core::position::BlockPos", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.util.Identifier", "horizon_fabric_bridge::java_types::Identifier", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.util.math.Direction", "horizon_fabric_bridge::java_types::Direction", HorizonNameKind::Class);
        
        // World methods
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.world.World", "getBlockState", "(Lnet/minecraft/util/math/BlockPos;)Lnet/minecraft/block/BlockState;", "horizon_client::world::World", "get_block_state", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.world.World", "setBlockState", "(Lnet/minecraft/util/math/BlockPos;Lnet/minecraft/block/BlockState;Z)Z", "horizon_client::world::World", "set_block_state", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.world.World", "getBlockEntity", "(Lnet/minecraft/util/math/BlockPos;)Lnet/minecraft/block/entity/BlockEntity;", "horizon_client::world::World", "get_block_entity", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.world.World", "spawnEntity", "(Lnet/minecraft/entity/Entity;)Z", "horizon_client::world::World", "spawn_entity", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.world.World", "playSound", "(Lnet/minecraft/entity/player/PlayerEntity;DDDLnet/minecraft/sound/SoundEvent;Lnet/minecraft/sound/SoundCategory;FF)V", "horizon_client::world::World", "play_sound", HorizonNameKind::Method);
        
        // Player methods
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.entity.player.PlayerEntity", "getInventory", "()Lnet/minecraft/inventory/PlayerInventory;", "horizon_client::motion::PlayerMotion", "inventory", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.entity.player.PlayerEntity", "getMainHandStack", "()Lnet/minecraft/item/ItemStack;", "horizon_client::motion::PlayerMotion", "main_hand_stack", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.entity.player.PlayerEntity", "getOffHandStack", "()Lnet/minecraft/item/ItemStack;", "horizon_client::motion::PlayerMotion", "off_hand_stack", HorizonNameKind::Method);
        
        // BlockPos fields
        Self::add_field(&mut mappings, &mut reverse, "net.minecraft.util.math.BlockPos", "x", "I", "horizon_core::position::BlockPos", "x", HorizonNameKind::Field);
        Self::add_field(&mut mappings, &mut reverse, "net.minecraft.util.math.BlockPos", "y", "I", "horizon_core::position::BlockPos", "y", HorizonNameKind::Field);
        Self::add_field(&mut mappings, &mut reverse, "net.minecraft.util.math.BlockPos", "z", "I", "horizon_core::position::BlockPos", "z", HorizonNameKind::Field);
        
        // ItemStack fields/methods
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.item.ItemStack", "getItem", "()Lnet/minecraft/item/Item;", "horizon_client::items::ItemStack", "item", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.item.ItemStack", "getCount", "()I", "horizon_client::items::ItemStack", "count", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.item.ItemStack", "isEmpty", "()Z", "horizon_client::items::ItemStack", "is_empty", HorizonNameKind::Method);
        
        // Identifier methods
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.util.Identifier", "getNamespace", "()Ljava/lang/String;", "horizon_fabric_bridge::java_types::Identifier", "namespace", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.util.Identifier", "getPath", "()Ljava/lang/String;", "horizon_fabric_bridge::java_types::Identifier", "path", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.util.Identifier", "toString", "()Ljava/lang/String;", "horizon_fabric_bridge::java_types::Identifier", "to_string", HorizonNameKind::Method);
        
        // Direction methods
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.util.math.Direction", "getName", "()Ljava/lang/String;", "horizon_fabric_bridge::java_types::Direction", "name", HorizonNameKind::Method);
        
        // BlockState methods
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.block.BlockState", "getBlock", "()Lnet/minecraft/block/Block;", "horizon_client::blocks::BlockState", "block", HorizonNameKind::Method);
        Self::add_method(&mut mappings, &mut reverse, "net.minecraft.block.BlockState", "getProperties", "()Ljava/util/Map;", "horizon_client::blocks::BlockState", "properties", HorizonNameKind::Method);
        
        // Registry classes
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.registry.Registry", "horizon_client::registry::Registry", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.registry.RegistryKey", "horizon_client::registry::RegistryKey", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.registry.RegistryEntry", "horizon_client::registry::RegistryEntry", HorizonNameKind::Class);
        
        // Network classes
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.network.PacketByteBuf", "horizon_protocol::PacketByteBuf", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.minecraft.network.packet.Packet", "horizon_protocol::Packet", HorizonNameKind::Class);
        
        // Command classes
        Self::add_class(&mut mappings, &mut reverse, "com.mojang.brigadier.CommandDispatcher", "horizon_command::CommandDispatcher", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "com.mojang.brigadier.context.CommandContext", "horizon_command::CommandContext", HorizonNameKind::Class);
        
        // Event classes
        Self::add_class(&mut mappings, &mut reverse, "net.fabricmc.fabric.api.event.Event", "horizon_fabric_bridge::callback::FabricEvent", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.fabricmc.fabric.api.networking.v1.PacketReceiver", "horizon_fabric_bridge::fabric_interfaces::PacketReceiver", HorizonNameKind::Class);
        Self::add_class(&mut mappings, &mut reverse, "net.fabricmc.fabric.api.command.v2.CommandRegistrationCallback", "horizon_fabric_bridge::fabric_interfaces::CommandRegistrationCallback", HorizonNameKind::Class);
        
        Ok(Self { mappings, reverse })
    }
    
    fn add_class(
        mappings: &mut HashMap<String, HorizonName>,
        reverse: &mut HashMap<String, String>,
        yarn: &str,
        rust_path: &str,
        kind: HorizonNameKind,
    ) {
        let rust_name = rust_path.rsplit("::").next().unwrap_or(rust_path);
        let horizon = HorizonName {
            rust_path: rust_path.to_string(),
            rust_name: rust_name.to_string(),
            kind,
        };
        mappings.insert(yarn.to_string(), horizon.clone());
        reverse.insert(format!("{}::{}", rust_path, rust_name), yarn.to_string());
    }
    
    fn add_method(
        mappings: &mut HashMap<String, HorizonName>,
        reverse: &mut HashMap<String, String>,
        yarn_class: &str,
        yarn_method: &str,
        descriptor: &str,
        rust_path: &str,
        rust_name: &str,
        kind: HorizonNameKind,
    ) {
        let key = format!("{}/{}", yarn_class, yarn_method);
        let horizon = HorizonName {
            rust_path: rust_path.to_string(),
            rust_name: rust_name.to_string(),
            kind,
        };
        mappings.insert(key.clone(), horizon.clone());
        reverse.insert(format!("{}::{}", rust_path, rust_name), key.clone());
        
        // Also add with descriptor for overloaded methods
        let key_with_desc = format!("{}#{}", key, descriptor);
        mappings.insert(key_with_desc, horizon);
    }
    
    fn add_field(
        mappings: &mut HashMap<String, HorizonName>,
        reverse: &mut HashMap<String, String>,
        yarn_class: &str,
        yarn_field: &str,
        _descriptor: &str,
        rust_path: &str,
        rust_name: &str,
        kind: HorizonNameKind,
    ) {
        let key = format!("{}/{}", yarn_class, yarn_field);
        let horizon = HorizonName {
            rust_path: rust_path.to_string(),
            rust_name: rust_name.to_string(),
            kind,
        };
        mappings.insert(key.clone(), horizon.clone());
        reverse.insert(format!("{}::{}", rust_path, rust_name), key);
    }
    
    /// Find a class mapping.
    pub fn find_class(&self, yarn_class: &str) -> Option<&HorizonName> {
        self.mappings.get(yarn_class)
    }
    
    /// Find a field mapping.
    pub fn find_field(&self, yarn_class: &str, yarn_field: &str) -> Option<&HorizonName> {
        let key = format!("{}/{}", yarn_class, yarn_field);
        self.mappings.get(&key)
    }
    
    /// Find a method mapping.
    pub fn find_method(&self, yarn_class: &str, yarn_method: &str, descriptor: &str) -> Option<&HorizonName> {
        // Try with descriptor first
        let key_with_desc = format!("{}/{}#{}", yarn_class, yarn_method, descriptor);
        if let Some(h) = self.mappings.get(&key_with_desc) {
            return Some(h);
        }
        
        // Try without descriptor
        let key = format!("{}/{}", yarn_class, yarn_method);
        self.mappings.get(&key)
    }
    
    /// Get all mappings.
    pub fn all(&self) -> &HashMap<String, HorizonName> {
        &self.mappings
    }

    /// The Yarn key for a Horizon `rust_path::rust_name` pair, if mapped.
    pub fn to_yarn(&self, rust_path: &str, rust_name: &str) -> Option<&str> {
        self.reverse
            .get(&format!("{rust_path}::{rust_name}"))
            .map(String::as_str)
    }
}