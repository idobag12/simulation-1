//! Deferred structural mutation (SPEC §6 command buffer pattern).

use crate::entity::Entity;
use crate::error::EcsError;
use crate::store::Component;
use crate::world::World;

/// One queued world mutation.
type Command = Box<dyn FnOnce(&mut World) -> Result<(), EcsError>>;

/// A queue of structural mutations (spawn/despawn/insert/remove) recorded by
/// a system while it iterates and applied by the schedule immediately after
/// that system runs.
///
/// Invariants:
/// - Commands apply in exactly the order they were queued (deterministic).
/// - Mid-iteration mutation of the world is impossible: systems only ever
///   queue; application happens at the defined post-system point (SPEC §6).
/// - Commands targeting an entity that died earlier in the same buffer fail
///   with a typed error rather than silently retargeting; a system that
///   queues conflicting commands has a logic bug, and the tick aborts with
///   an attributable error (SPEC §3).
#[derive(Default)]
pub struct CommandBuffer {
    commands: Vec<Command>,
}

impl CommandBuffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        CommandBuffer::default()
    }

    /// Number of queued commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// True iff no commands are queued.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Queues inserting `component` on `entity`.
    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        self.commands.push(Box::new(move |world| {
            world.insert(entity, component).map(|_| ())
        }));
    }

    /// Queues removing component `T` from `entity`.
    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.commands
            .push(Box::new(move |world| world.remove::<T>(entity).map(|_| ())));
    }

    /// Queues despawning `entity`.
    pub fn despawn(&mut self, entity: Entity) {
        self.commands
            .push(Box::new(move |world| world.despawn(entity)));
    }

    /// Queues spawning a new entity; `init` runs at application time to
    /// attach its components.
    pub fn spawn(
        &mut self,
        init: impl FnOnce(&mut World, Entity) -> Result<(), EcsError> + 'static,
    ) {
        self.commands.push(Box::new(move |world| {
            let entity = world.spawn();
            init(world, entity)
        }));
    }

    /// Applies all queued commands to `world` in queue order, draining the
    /// buffer. Called by the schedule after each system runs (SPEC §6).
    pub fn apply(&mut self, world: &mut World) -> Result<(), EcsError> {
        for command in self.commands.drain(..) {
            command(world)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StorageKind;
    use core_types::Seed;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Marker(u64);
    impl Component for Marker {
        const NAME: &'static str = "test.marker";
        const STORAGE: StorageKind = StorageKind::Dense;
    }

    #[test]
    fn commands_apply_in_queue_order() {
        let mut world = World::new(Seed::new(1));
        world.register::<Marker>().unwrap();
        let e = world.spawn();

        let mut buffer = CommandBuffer::new();
        buffer.insert(e, Marker(1));
        buffer.insert(e, Marker(2)); // later command wins
        assert_eq!(buffer.len(), 2);
        buffer.apply(&mut world).unwrap();
        assert!(buffer.is_empty());
        assert_eq!(world.get::<Marker>(e).unwrap(), Some(&Marker(2)));
    }

    #[test]
    fn spawn_and_despawn_through_the_buffer() {
        let mut world = World::new(Seed::new(1));
        world.register::<Marker>().unwrap();
        let doomed = world.spawn();

        let mut buffer = CommandBuffer::new();
        buffer.spawn(|w, e| w.insert(e, Marker(7)).map(|_| ()));
        buffer.despawn(doomed);
        buffer.apply(&mut world).unwrap();

        assert!(!world.is_alive(doomed));
        assert_eq!(world.entity_count(), 1);
        let values: Vec<u64> = world.iter::<Marker>().unwrap().map(|(_, m)| m.0).collect();
        assert_eq!(values, vec![7]);
    }

    #[test]
    fn command_on_entity_despawned_earlier_in_buffer_is_an_error() {
        let mut world = World::new(Seed::new(1));
        world.register::<Marker>().unwrap();
        let e = world.spawn();

        let mut buffer = CommandBuffer::new();
        buffer.despawn(e);
        buffer.insert(e, Marker(1));
        assert!(matches!(
            buffer.apply(&mut world),
            Err(EcsError::DeadEntity(_))
        ));
    }
}
