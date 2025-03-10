#![doc = include_str!("../README.md")]
#![deny(
    rustdoc::broken_intra_doc_links,
    rustdoc::private_intra_doc_links,
    rustdoc::missing_crate_level_docs,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_rust_codeblocks,
    rustdoc::bare_urls,
    rustdoc::invalid_html_tags
)]
#![warn(
    trivial_casts,
    trivial_numeric_casts,
    unused_lifetimes,
    unused_import_braces,
    unreachable_pub,
    clippy::dbg_macro
)]

pub mod hitbox;

use std::num::Wrapping;
use std::ops::Range;

use bevy_app::{App, CoreSet, Plugin};
use bevy_ecs::prelude::*;
use glam::{DVec3, Vec3};
use paste::paste;
use rustc_hash::FxHashMap;
use tracing::warn;
use uuid::Uuid;
use valence_core::chunk_pos::ChunkPos;
use valence_core::despawn::Despawned;
use valence_core::packet::var_int::VarInt;
use valence_core::packet::{Decode, Encode};
use valence_core::uuid::UniqueId;
use valence_core::DEFAULT_TPS;

include!(concat!(env!("OUT_DIR"), "/entity.rs"));
pub struct EntityPlugin;

/// When new Minecraft entities are initialized and added to
/// [`EntityManager`].
///
/// Systems that need Minecraft entities to be in a valid state should run
/// _after_ this set.
#[derive(SystemSet, Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct InitEntitiesSet;

/// When tracked data is written to the entity's [`TrackedData`] component.
/// Systems that modify tracked data should run _before_ this.
#[derive(SystemSet, Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct UpdateTrackedDataSet;

/// When entities are updated and changes from the current tick are cleared.
/// Systems that need to observe changes to entities (Such as the difference
/// between [`Position`] and [`OldPosition`]) should run _before_ this set (and
/// probably after [`InitEntitiesSet`]).
#[derive(SystemSet, Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ClearEntityChangesSet;

impl Plugin for EntityPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EntityManager::new())
            .configure_sets((
                InitEntitiesSet.in_base_set(CoreSet::PostUpdate),
                UpdateTrackedDataSet.in_base_set(CoreSet::PostUpdate),
                ClearEntityChangesSet
                    .after(InitEntitiesSet)
                    .after(UpdateTrackedDataSet)
                    .in_base_set(CoreSet::PostUpdate),
            ))
            .add_systems(
                (init_entities, remove_despawned_from_manager)
                    .chain()
                    .in_set(InitEntitiesSet),
            )
            .add_systems(
                (
                    clear_status_changes,
                    clear_animation_changes,
                    clear_tracked_data_changes,
                    update_old_position,
                    update_old_location,
                )
                    .in_set(ClearEntityChangesSet),
            );

        add_tracked_data_systems(app);
    }
}

fn update_old_position(mut query: Query<(&Position, &mut OldPosition)>) {
    for (pos, mut old_pos) in &mut query {
        old_pos.0 = pos.0;
    }
}

fn update_old_location(mut query: Query<(&Location, &mut OldLocation)>) {
    for (loc, mut old_loc) in &mut query {
        old_loc.0 = loc.0;
    }
}

fn init_entities(
    mut entities: Query<
        (
            Entity,
            &mut EntityId,
            &mut UniqueId,
            &Position,
            &mut OldPosition,
        ),
        Added<EntityKind>,
    >,
    mut manager: ResMut<EntityManager>,
) {
    for (entity, mut id, uuid, pos, mut old_pos) in &mut entities {
        *old_pos = OldPosition::new(pos.0);

        if *id == EntityId::default() {
            *id = manager.next_id();
        }

        if let Some(conflict) = manager.id_to_entity.insert(id.0, entity) {
            warn!(
                "entity {entity:?} has conflicting entity ID of {} with entity {conflict:?}",
                id.0
            );
        }

        if let Some(conflict) = manager.uuid_to_entity.insert(uuid.0, entity) {
            warn!(
                "entity {entity:?} has conflicting UUID of {} with entity {conflict:?}",
                uuid.0
            );
        }
    }
}

#[allow(clippy::type_complexity)]
fn remove_despawned_from_manager(
    entities: Query<(&EntityId, &UniqueId), (With<EntityKind>, With<Despawned>)>,
    mut manager: ResMut<EntityManager>,
) {
    for (id, uuid) in &entities {
        manager.id_to_entity.remove(&id.0);
        manager.uuid_to_entity.remove(&uuid.0);
    }
}

fn clear_status_changes(mut statuses: Query<&mut EntityStatuses, Changed<EntityStatuses>>) {
    for mut statuses in &mut statuses {
        statuses.0 = 0;
    }
}

fn clear_animation_changes(
    mut animations: Query<&mut EntityAnimations, Changed<EntityAnimations>>,
) {
    for mut animations in &mut animations {
        animations.0 = 0;
    }
}

fn clear_tracked_data_changes(mut tracked_data: Query<&mut TrackedData, Changed<TrackedData>>) {
    for mut tracked_data in &mut tracked_data {
        tracked_data.clear_update_values();
    }
}

/// Contains the `Instance` an entity is located in. For the coordinates
/// within the instance, see [`Position`].
#[derive(Component, Copy, Clone, PartialEq, Eq, Debug)]
pub struct Location(pub Entity);

impl Default for Location {
    fn default() -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

impl PartialEq<OldLocation> for Location {
    fn eq(&self, other: &OldLocation) -> bool {
        self.0 == other.0
    }
}

/// The value of [`Location`] from the end of the previous tick.
///
/// **NOTE**: You should not modify this component after the entity is spawned.
#[derive(Component, Copy, Clone, PartialEq, Eq, Debug)]
pub struct OldLocation(Entity);

impl OldLocation {
    pub fn new(instance: Entity) -> Self {
        Self(instance)
    }

    pub fn get(&self) -> Entity {
        self.0
    }
}

impl Default for OldLocation {
    fn default() -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

impl PartialEq<Location> for OldLocation {
    fn eq(&self, other: &Location) -> bool {
        self.0 == other.0
    }
}

#[derive(Component, Copy, Clone, PartialEq, Default, Debug)]
pub struct Position(pub DVec3);

impl Position {
    pub fn new(pos: impl Into<DVec3>) -> Self {
        Self(pos.into())
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        ChunkPos::from_dvec3(self.0)
    }

    pub fn get(self) -> DVec3 {
        self.0
    }

    pub fn set(&mut self, pos: impl Into<DVec3>) {
        self.0 = pos.into();
    }
}

impl PartialEq<OldPosition> for Position {
    fn eq(&self, other: &OldPosition) -> bool {
        self.0 == other.0
    }
}

/// The value of [`Position`] from the end of the previous tick.
///
/// **NOTE**: You should not modify this component after the entity is spawned.
#[derive(Component, Copy, Clone, PartialEq, Default, Debug)]
pub struct OldPosition(DVec3);

impl OldPosition {
    pub fn new(pos: impl Into<DVec3>) -> Self {
        Self(pos.into())
    }

    pub fn get(self) -> DVec3 {
        self.0
    }

    pub fn chunk_pos(self) -> ChunkPos {
        ChunkPos::from_dvec3(self.0)
    }
}

impl PartialEq<Position> for OldPosition {
    fn eq(&self, other: &Position) -> bool {
        self.0 == other.0
    }
}

/// Describes the direction an entity is looking using pitch and yaw angles.
#[derive(Component, Copy, Clone, PartialEq, Default, Debug)]
pub struct Look {
    /// The yaw angle in degrees.
    pub yaw: f32,
    /// The pitch angle in degrees.
    pub pitch: f32,
}

impl Look {
    pub const fn new(yaw: f32, pitch: f32) -> Self {
        Self { yaw, pitch }
    }

    /// Gets a normalized direction vector from the yaw and pitch.
    pub fn vec(self) -> Vec3 {
        let (yaw_sin, yaw_cos) = (self.yaw + 90.0).to_radians().sin_cos();
        let (pitch_sin, pitch_cos) = (-self.pitch).to_radians().sin_cos();

        Vec3::new(yaw_cos * pitch_cos, pitch_sin, yaw_sin * pitch_cos)
    }

    /// Sets the yaw and pitch using a normalized direction vector.
    pub fn set_vec(&mut self, dir: Vec3) {
        debug_assert!(
            dir.is_normalized(),
            "the direction vector should be normalized"
        );

        // Preserve the current yaw if we're looking straight up or down.
        if dir.x != 0.0 || dir.z != 0.0 {
            self.yaw = f32::atan2(dir.z, dir.x).to_degrees() - 90.0;
        }

        self.pitch = -(dir.y).asin().to_degrees();
    }
}

#[derive(Component, Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct OnGround(pub bool);

/// A Minecraft entity's ID according to the protocol.
///
/// IDs should be _unique_ for the duration of the server and  _constant_ for
/// the lifetime of the entity. IDs of -1 (the default) will be assigned to
/// something else on the tick the entity is added. If you need to know the ID
/// ahead of time, set this component to the value returned by
/// [`EntityManager::next_id`] before spawning.
#[derive(Component, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntityId(i32);

impl EntityId {
    /// Returns the underlying entity ID as an integer.
    pub fn get(self) -> i32 {
        self.0
    }
}

/// Returns an entity ID of -1.
impl Default for EntityId {
    fn default() -> Self {
        Self(-1)
    }
}

#[derive(Component, Copy, Clone, PartialEq, Default, Debug)]
pub struct HeadYaw(pub f32);

/// Entity velocity in m/s.
#[derive(Component, Copy, Clone, Default, Debug)]
pub struct Velocity(pub Vec3);

impl Velocity {
    pub fn to_packet_units(self) -> [i16; 3] {
        // The saturating casts to i16 are desirable.
        (8000.0 / DEFAULT_TPS.get() as f32 * self.0)
            .to_array()
            .map(|v| v as i16)
    }
}

#[derive(Component, Copy, Clone, Default, Debug)]
pub struct EntityStatuses(pub u64);

impl EntityStatuses {
    pub fn trigger(&mut self, status: EntityStatus) {
        self.set(status, true);
    }

    pub fn set(&mut self, status: EntityStatus, triggered: bool) {
        self.0 |= (triggered as u64) << status as u64;
    }

    pub fn get(&self, status: EntityStatus) -> bool {
        (self.0 >> status as u64) & 1 == 1
    }
}

#[derive(Component, Default, Debug)]
pub struct EntityAnimations(pub u8);

impl EntityAnimations {
    pub fn trigger(&mut self, anim: EntityAnimation) {
        self.set(anim, true);
    }

    pub fn set(&mut self, anim: EntityAnimation, triggered: bool) {
        self.0 |= (triggered as u8) << anim as u8;
    }

    pub fn get(&self, anim: EntityAnimation) -> bool {
        (self.0 >> anim as u8) & 1 == 1
    }
}

/// Extra integer data passed to the entity spawn packet. The meaning depends on
/// the type of entity being spawned.
///
/// Some examples:
/// - **Experience Orb**: Experience count
/// - **(Glowing) Item Frame**: Rotation
/// - **Painting**: Rotation
/// - **Falling Block**: Block state
/// - **Fishing Bobber**: Hook entity ID
/// - **Warden**: Initial pose
#[derive(Component, Default, Debug)]
pub struct ObjectData(pub i32);

/// The range of packet bytes for this entity within the cell the entity is
/// located in. For internal use only.
#[derive(Component, Default, Debug)]
pub struct PacketByteRange(pub Range<usize>);

/// Cache for all the tracked data of an entity. Used for the
/// [`EntityTrackerUpdateS2c`][packet] packet.
///
/// [packet]: valence_core::packet::s2c::play::EntityTrackerUpdateS2c
#[derive(Component, Default, Debug)]
pub struct TrackedData {
    init_data: Vec<u8>,
    /// A map of tracked data indices to the byte length of the entry in
    /// `init_data`.
    init_entries: Vec<(u8, u32)>,
    update_data: Vec<u8>,
}

impl TrackedData {
    /// Returns initial tracked data for the entity, ready to be sent in the
    /// [`EntityTrackerUpdateS2c`][packet] packet. This is used when the entity
    /// enters the view of a client.
    ///
    /// [packet]: valence_core::packet::s2c::play::EntityTrackerUpdateS2c
    pub fn init_data(&self) -> Option<&[u8]> {
        if self.init_data.len() > 1 {
            Some(&self.init_data)
        } else {
            None
        }
    }

    /// Contains updated tracked data for the entity, ready to be sent in the
    /// [`EntityTrackerUpdateS2c`][packet] packet. This is used when tracked
    /// data is changed and the client is already in view of the entity.
    ///
    /// [packet]: valence_core::packet::s2c::play::EntityTrackerUpdateS2c
    pub fn update_data(&self) -> Option<&[u8]> {
        if self.update_data.len() > 1 {
            Some(&self.update_data)
        } else {
            None
        }
    }

    pub fn insert_init_value(&mut self, index: u8, type_id: u8, value: impl Encode) {
        debug_assert!(
            index != 0xff,
            "index of 0xff is reserved for the terminator"
        );

        self.remove_init_value(index);

        self.init_data.pop(); // Remove terminator.

        // Append the new value to the end.
        let len_before = self.init_data.len();

        self.init_data.extend_from_slice(&[index, type_id]);
        if let Err(e) = value.encode(&mut self.init_data) {
            warn!("failed to encode initial tracked data: {e:#}");
        }

        let len = self.init_data.len() - len_before;

        self.init_entries.push((index, len as u32));

        self.init_data.push(0xff); // Add terminator.
    }

    pub fn remove_init_value(&mut self, index: u8) -> bool {
        let mut start = 0;

        for (pos, &(idx, len)) in self.init_entries.iter().enumerate() {
            if idx == index {
                let end = start + len as usize;

                self.init_data.drain(start..end);
                self.init_entries.remove(pos);

                return true;
            }

            start += len as usize;
        }

        false
    }

    pub fn append_update_value(&mut self, index: u8, type_id: u8, value: impl Encode) {
        debug_assert!(
            index != 0xff,
            "index of 0xff is reserved for the terminator"
        );

        self.update_data.pop(); // Remove terminator.

        self.update_data.extend_from_slice(&[index, type_id]);
        if let Err(e) = value.encode(&mut self.update_data) {
            warn!("failed to encode updated tracked data: {e:#}");
        }

        self.update_data.push(0xff); // Add terminator.
    }

    pub fn clear_update_values(&mut self) {
        self.update_data.clear();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Encode, Decode)]
pub struct VillagerData {
    pub kind: VillagerKind,
    pub profession: VillagerProfession,
    pub level: i32,
}

impl VillagerData {
    pub const fn new(kind: VillagerKind, profession: VillagerProfession, level: i32) -> Self {
        Self {
            kind,
            profession,
            level,
        }
    }
}

impl Default for VillagerData {
    fn default() -> Self {
        Self {
            kind: Default::default(),
            profession: Default::default(),
            level: 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum VillagerKind {
    Desert,
    Jungle,
    #[default]
    Plains,
    Savanna,
    Snow,
    Swamp,
    Taiga,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum VillagerProfession {
    #[default]
    None,
    Armorer,
    Butcher,
    Cartographer,
    Cleric,
    Farmer,
    Fisherman,
    Fletcher,
    Leatherworker,
    Librarian,
    Mason,
    Nitwit,
    Shepherd,
    Toolsmith,
    Weaponsmith,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum Pose {
    #[default]
    Standing,
    FallFlying,
    Sleeping,
    Swimming,
    SpinAttack,
    Sneaking,
    LongJumping,
    Dying,
    Croaking,
    UsingTongue,
    Roaring,
    Sniffing,
    Emerging,
    Digging,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum BoatKind {
    #[default]
    Oak,
    Spruce,
    Birch,
    Jungle,
    Acacia,
    DarkOak,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum CatKind {
    Tabby,
    #[default]
    Black,
    Red,
    Siamese,
    BritishShorthair,
    Calico,
    Persian,
    Ragdoll,
    White,
    Jellie,
    AllBlack,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum FrogKind {
    #[default]
    Temperate,
    Warm,
    Cold,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum PaintingKind {
    #[default]
    Kebab,
    Aztec,
    Alban,
    Aztec2,
    Bomb,
    Plant,
    Wasteland,
    Pool,
    Courbet,
    Sea,
    Sunset,
    Creebet,
    Wanderer,
    Graham,
    Match,
    Bust,
    Stage,
    Void,
    SkullAndRoses,
    Wither,
    Fighters,
    Pointer,
    Pigscene,
    BurningSkull,
    Skeleton,
    Earth,
    Wind,
    Water,
    Fire,
    DonkeyKong,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Encode, Decode)]
pub enum SnifferState {
    #[default]
    Idling,
    FeelingHappy,
    Scenting,
    Sniffing,
    Searching,
    Digging,
    Rising,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug, Encode, Decode)]
pub struct EulerAngle {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

#[derive(Copy, Clone)]
struct OptionalInt(Option<i32>);

impl Encode for OptionalInt {
    fn encode(&self, w: impl std::io::Write) -> anyhow::Result<()> {
        if let Some(n) = self.0 {
            VarInt(n.wrapping_add(1))
        } else {
            VarInt(0)
        }
        .encode(w)
    }
}

impl Decode<'_> for OptionalInt {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let n = VarInt::decode(r)?.0;

        Ok(Self(if n == 0 {
            None
        } else {
            Some(n.wrapping_sub(1))
        }))
    }
}

/// A [`Resource`] which maintains information about all spawned Minecraft
/// entities.
#[derive(Resource, Debug)]
pub struct EntityManager {
    /// Maps protocol IDs to ECS entities.
    id_to_entity: FxHashMap<i32, Entity>,
    uuid_to_entity: FxHashMap<Uuid, Entity>,
    next_id: Wrapping<i32>,
}

impl EntityManager {
    fn new() -> Self {
        Self {
            id_to_entity: FxHashMap::default(),
            uuid_to_entity: FxHashMap::default(),
            next_id: Wrapping(1), // Skip 0.
        }
    }

    /// Returns the next unique entity ID and increments the counter.
    pub fn next_id(&mut self) -> EntityId {
        if self.next_id.0 == 0 {
            warn!("entity ID overflow!");
            // ID 0 is reserved for clients, so skip over it.
            self.next_id.0 = 1;
        }

        let id = EntityId(self.next_id.0);

        self.next_id += 1;

        id
    }

    /// Gets the entity with the given entity ID.
    pub fn get_by_id(&self, entity_id: i32) -> Option<Entity> {
        self.id_to_entity.get(&entity_id).cloned()
    }

    /// Gets the entity with the given UUID.
    pub fn get_by_uuid(&self, uuid: Uuid) -> Option<Entity> {
        self.uuid_to_entity.get(&uuid).cloned()
    }
}

// TODO: should `set_if_neq` behavior be the default behavior for setters?
macro_rules! flags {
    (
        $(
            $component:path {
                $($flag:ident: $offset:literal),* $(,)?
            }
        )*

    ) => {
        $(
            impl $component {
                $(
                    #[doc = "Gets the bit at offset "]
                    #[doc = stringify!($offset)]
                    #[doc = "."]
                    #[inline]
                    pub const fn $flag(&self) -> bool {
                        (self.0 >> $offset) & 1 == 1
                    }

                    paste! {
                        #[doc = "Sets the bit at offset "]
                        #[doc = stringify!($offset)]
                        #[doc = "."]
                        #[inline]
                        pub fn [< set_$flag >] (&mut self, $flag: bool) {
                            self.0 = (self.0 & !(1 << $offset)) | (($flag as i8) << $offset);
                        }
                    }
                )*
            }
        )*
    }
}

flags! {
    entity::Flags {
        on_fire: 0,
        sneaking: 1,
        sprinting: 3,
        swimming: 4,
        invisible: 5,
        glowing: 6,
        fall_flying: 7,
    }
    persistent_projectile::ProjectileFlags {
        critical: 0,
        no_clip: 1,
    }
    living::LivingFlags {
        using_item: 0,
        off_hand_active: 1,
        using_riptide: 2,
    }
    player::PlayerModelParts {
        cape: 0,
        jacket: 1,
        left_sleeve: 2,
        right_sleeve: 3,
        left_pants_leg: 4,
        right_pants_leg: 5,
        hat: 6,
    }
    player::MainArm {
        right: 0,
    }
    armor_stand::ArmorStandFlags {
        small: 0,
        show_arms: 1,
        hide_base_plate: 2,
        marker: 3,
    }
    mob::MobFlags {
        ai_disabled: 0,
        left_handed: 1,
        attacking: 2,
    }
    bat::BatFlags {
        hanging: 0,
    }
    abstract_horse::HorseFlags {
        tamed: 1,
        saddled: 2,
        bred: 3,
        eating_grass: 4,
        angry: 5,
        eating: 6,
    }
    fox::FoxFlags {
        sitting: 0,
        crouching: 2,
        rolling_head: 3,
        chasing: 4,
        sleeping: 5,
        walking: 6,
        aggressive: 7,
    }
    panda::PandaFlags {
        sneezing: 1,
        playing: 2,
        sitting: 3,
        lying_on_back: 4,
    }
    tameable::TameableFlags {
        sitting_pose: 0,
        tamed: 2,
    }
    iron_golem::IronGolemFlags {
        player_created: 0,
    }
    snow_golem::SnowGolemFlags {
        has_pumpkin: 4,
    }
    blaze::BlazeFlags {
        fire_active: 0,
    }
    vex::VexFlags {
        charging: 0,
    }
    spider::SpiderFlags {
        climbing_wall: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_remove_init_tracked_data() {
        let mut td = TrackedData::default();

        td.insert_init_value(0, 3, "foo");
        td.insert_init_value(10, 6, "bar");
        td.insert_init_value(5, 9, "baz");

        assert!(td.remove_init_value(10));
        assert!(!td.remove_init_value(10));

        // Insertion overwrites value at index 0.
        td.insert_init_value(0, 64, "quux");

        assert!(td.remove_init_value(0));
        assert!(td.remove_init_value(5));

        assert!(td.init_data.as_slice().is_empty() || td.init_data.as_slice() == [0xff]);
        assert!(td.init_data().is_none());

        assert!(td.update_data.is_empty());
    }

    #[test]
    fn get_set_flags() {
        let mut flags = entity::Flags(0);

        flags.set_on_fire(true);
        let before = flags.clone();
        assert_ne!(flags.0, 0);
        flags.set_on_fire(true);
        assert_eq!(before, flags);
        flags.set_on_fire(false);
        assert_eq!(flags.0, 0);
    }
}
